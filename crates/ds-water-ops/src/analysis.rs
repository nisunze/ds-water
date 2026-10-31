//! Pressure analysis with LV voltage-drop semantics.
//!
//! LV writes `vd_*` columns on its processed features: the drop from the
//! source, the voltage left, whether it is within limits, the current, the
//! loading and the worst drop downstream, plus a summary per feeder path.
//! Water reads the same way, with pressure in place of voltage:
//!
//! | LV | Water |
//! | --- | --- |
//! | `vd_pct` drop from the transformer | `pd_m` head lost from the static line, `pd_pct` of the static pressure |
//! | `vd_u_v` voltage at the bus | `p_m` residual pressure at peak |
//! | `vd_ok` within the limit | `p_ok` at least the required pressure (demand junctions) or not negative |
//! | `vd_i_a` current | `q_lps` flow |
//! | `vd_loading_pct` | `pd_loading_pct` velocity against the velocity limit |
//! | `vd_worst_pct` worst downstream | `pd_worst_m` worst residual pressure downstream |
//!
//! Every column is recomputed on each run; nothing outlives the network it
//! was computed on.

use std::collections::HashMap;

use ds_water_hydraulics::{Hydraulics, Snapshot};
use ds_water_model::WaterNetwork;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{OpError, engine};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AnalysisCriteria {
    pub min_pressure_m: f64,
    pub max_velocity_ms: f64,
}

impl Default for AnalysisCriteria {
    fn default() -> Self {
        Self {
            min_pressure_m: 10.0,
            max_velocity_ms: 2.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PathSummary {
    pub label: String,
    pub root: String,
    pub tip: String,
    pub length_m: f64,
    /// The junction on the path with the least residual pressure.
    pub worst_junction: Option<String>,
    pub worst_pressure_m: Option<f64>,
    /// Largest head lost from the static line along the path.
    pub max_drop_m: f64,
    pub ok: bool,
}

/// Peak and static solves, the `pd_*` columns as GeoJSON layers and the
/// per-path summary.
pub fn analyse(net: &WaterNetwork, criteria: &AnalysisCriteria) -> Result<Value, OpError> {
    let levels = Hydraulics::initial_tank_levels(net);
    let peak = Hydraulics::new(net).map_err(engine)?.solve(0, &levels);
    let mut still_net = net.clone();
    still_net.options.demand_multiplier = 0.0;
    let still = Hydraulics::new(&still_net)
        .map_err(engine)?
        .solve(0, &levels);
    let nj = net.junctions.len();
    let index: HashMap<&str, usize> = net
        .node_refs()
        .enumerate()
        .map(|(i, (id, _))| (id, i))
        .collect();
    let required = |j: usize| {
        if net.junctions[j].demand_lps > 0.0 {
            criteria.min_pressure_m
        } else {
            0.0
        }
    };
    let ok = |j: usize| peak.disconnected.contains(&j) || peak.pressure_m[j] >= required(j) - 1e-6;

    let paths = ds_water_layout::orient(net).map_err(engine)?;
    // Worst residual pressure downstream of each junction: the minimum over
    // every path suffix that starts there.
    let mut worst: Vec<f64> = (0..nj).map(|j| peak.pressure_m[j]).collect();
    for path in &paths {
        let mut running = f64::INFINITY;
        for id in path.nodes.iter().rev() {
            if let Some(&n) = index.get(id.as_str())
                && n < nj
            {
                running = running.min(peak.pressure_m[n]);
                worst[n] = worst[n].min(running);
            }
        }
    }

    let mut junctions = Vec::with_capacity(nj);
    for (j, junction) in net.junctions.iter().enumerate() {
        let drop = still.head_m[j] - peak.head_m[j];
        let static_p = still.pressure_m[j].max(1e-9);
        junctions.push(feature(
            net.node_xy(ds_water_model::NodeRef {
                kind: ds_water_model::NodeKind::Junction,
                index: j,
            })
            .map(point),
            json!({
                "id": junction.id,
                "demand_lps": peak.delivered_lps[j],
                "p_m": round(peak.pressure_m[j]),
                "p_ok": ok(j),
                "pd_m": round(drop),
                "pd_pct": round(100.0 * drop / static_p),
                "pd_worst_m": round(worst[j]),
                "pd_disconnected": peak.disconnected.contains(&j),
            }),
        ));
    }
    let mut pipes = Vec::with_capacity(net.pipes.len());
    for (k, pipe) in net.pipes.iter().enumerate() {
        let geometry = line(net, &pipe.from, &pipe.to, &pipe.vertices);
        let downstream = if peak.flow_lps[k] >= 0.0 {
            &pipe.to
        } else {
            &pipe.from
        };
        let worst_down = index
            .get(downstream.as_str())
            .filter(|&&n| n < nj)
            .map(|&n| round(worst[n]));
        pipes.push(feature(
            geometry,
            json!({
                "id": pipe.id,
                "internal_mm": pipe.diameter_mm,
                "q_lps": round(peak.flow_lps[k]),
                "v_ms": round(peak.velocity_ms[k]),
                "hl_m_per_km": round(peak.headloss_m[k].abs() / pipe.length_m * 1000.0),
                "pd_loading_pct": round(100.0 * peak.velocity_ms[k] / criteria.max_velocity_ms),
                "pd_worst_m": worst_down,
            }),
        ));
    }

    let summaries: Vec<PathSummary> = paths
        .iter()
        .map(|path| {
            let on: Vec<usize> = path
                .nodes
                .iter()
                .filter_map(|id| index.get(id.as_str()).copied())
                .filter(|&n| n < nj)
                .collect();
            let worst_at = on
                .iter()
                .copied()
                .min_by(|&a, &b| peak.pressure_m[a].total_cmp(&peak.pressure_m[b]));
            let max_drop = on
                .iter()
                .map(|&n| still.head_m[n] - peak.head_m[n])
                .fold(0.0, f64::max);
            PathSummary {
                label: path.label.clone(),
                root: path.root.clone(),
                tip: path.tip.clone(),
                length_m: path.length_m,
                worst_junction: worst_at.map(|n| net.junctions[n].id.clone()),
                worst_pressure_m: worst_at.map(|n| round(peak.pressure_m[n])),
                max_drop_m: round(max_drop),
                ok: on.iter().all(|&n| ok(n)),
            }
        })
        .collect();
    let fc = |features: Vec<Value>| json!({ "type": "FeatureCollection", "features": features });
    Ok(json!({
        "converged": peak.converged && still.converged,
        "ok": summaries.iter().all(|p| p.ok),
        "paths": summaries,
        "layers": { "water_junctions": fc(junctions), "water_pipes": fc(pipes) },
        "warnings": warnings(&peak, &still),
    }))
}

fn warnings(peak: &Snapshot, still: &Snapshot) -> Vec<String> {
    peak.warnings
        .iter()
        .chain(still.warnings.iter())
        .cloned()
        .collect()
}

fn round(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn point(p: [f64; 2]) -> Value {
    json!({ "type": "Point", "coordinates": p })
}

fn feature(geometry: Option<Value>, properties: Value) -> Value {
    json!({ "type": "Feature", "geometry": geometry, "properties": properties })
}

fn line(net: &WaterNetwork, from: &str, to: &str, vertices: &[[f64; 2]]) -> Option<Value> {
    let xy = |id: &str| {
        net.node_refs()
            .find(|(n, _)| *n == id)
            .and_then(|(_, r)| net.node_xy(r))
    };
    let mut coords = vec![xy(from)?];
    coords.extend(vertices.iter().copied());
    coords.push(xy(to)?);
    Some(json!({ "type": "LineString", "coordinates": coords }))
}
