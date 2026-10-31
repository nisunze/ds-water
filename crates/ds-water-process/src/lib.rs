//! Water processing, modelled on LV processing.
//!
//! LV processing connects every customer it can reach to the feeder with a
//! service drop, leaves the rest as `unconnected_customers` with the rule
//! that left them there, and numbers what it builds. Water does the same
//! with service connections, using LV's vocabulary so the two verticals read
//! alike:
//!
//! - setting `connect_unconnected_customers` (default true): when false every
//!   customer is left unconnected with reason `customer_connection_disabled`,
//! - setting `max_connection_m`: a customer farther than this from any
//!   distribution pipe is left with reason `beyond_max_connection_length`,
//! - the reason sits under `network_properties.unconnected_reason`, as in LV.
//!
//! A connected customer gets a service connection (a small PE pipe from a
//! tapping saddle on the main to the meter at the customer) and its demand
//! lands on a junction at the tap. Then fittings are placed on the network:
//! horizontal bends where a pipe deflects (rounded up to the standard
//! 11.25°, 22.5°, 45° and 90° bends; thrust blocks from 22.5°), tees at
//! three-way junctions, crosses at four, reducers where the bore changes, end
//! caps at dead ends and tapping saddles at service connections. Vertical
//! bends follow the profile.

use std::collections::HashMap;

use ds_water_layout::geo::Projection;
use ds_water_model::{Junction, Pipe, WaterNetwork};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ProcessSettings {
    pub connect_unconnected_customers: bool,
    pub max_connection_m: f64,
    /// Service connection pipe: internal diameter, Hazen-Williams C.
    pub service_internal_mm: f64,
    pub service_roughness: f64,
    pub service_material: String,
    pub service_pn_bar: f64,
    /// Demand per customer when the customer point carries none, L/s.
    pub default_customer_lps: f64,
    /// Smallest deflection that needs a bend, degrees.
    pub min_bend_deg: f64,
    /// Bends from this angle get a thrust block, degrees.
    pub thrust_block_from_deg: f64,
}

impl Default for ProcessSettings {
    fn default() -> Self {
        Self {
            connect_unconnected_customers: true,
            max_connection_m: 50.0,
            service_internal_mm: 16.0,
            service_roughness: 150.0,
            service_material: "HDPE PE100 20 mm".into(),
            service_pn_bar: 16.0,
            // 6 people × 20 l/c/d × peak 2.
            default_customer_lps: 6.0 * 20.0 * 2.0 / 86_400.0,
            min_bend_deg: 5.0,
            thrust_block_from_deg: 22.5,
        }
    }
}

pub const CUSTOMER_CONNECTION_DISABLED: &str = "customer_connection_disabled";
pub const BEYOND_MAX_CONNECTION_LENGTH: &str = "beyond_max_connection_length";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum FittingKind {
    Bend,
    Tee,
    Cross,
    Reducer,
    EndCap,
    TappingSaddle,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Fitting {
    pub kind: FittingKind,
    /// Source coordinate.
    pub at: [f64; 2],
    /// Pipe or node it belongs to.
    pub on: String,
    /// Main bore, mm (the larger one for a reducer).
    pub internal_mm: f64,
    /// Branch or smaller bore, mm, for tees, reducers and saddles.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_mm: Option<f64>,
    /// Standard bend angle, degrees.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub angle_deg: Option<f64>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub thrust_block: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FittingLine {
    pub kind: FittingKind,
    pub internal_mm: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub angle_deg: Option<f64>,
    pub count: usize,
    pub thrust_blocks: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessResult {
    pub network: WaterNetwork,
    /// GeoJSON FeatureCollections for the map, LV-style layer names.
    pub layers: Value,
    pub connected: usize,
    pub unconnected: usize,
    pub fittings: Vec<Fitting>,
    pub fittings_boq: Vec<FittingLine>,
}

/// A customer the run will connect, before pipes are split.
struct Tap {
    pipe: usize,
    /// Fraction along the pipe.
    t: f64,
    id: String,
    at: [f64; 2],
    demand: f64,
    distance: f64,
    z: Option<f64>,
    feature: Value,
}

const BENDS: [f64; 4] = [11.25, 22.5, 45.0, 90.0];

/// Connect `customers` (a GeoJSON FeatureCollection of points) to the
/// network's pipes and place fittings.
pub fn process(net: &WaterNetwork, customers: &Value, settings: &ProcessSettings) -> ProcessResult {
    let mut out = net.clone();
    let refs: Vec<_> = net.node_refs().map(|(id, r)| (id.to_owned(), r)).collect();
    let xy: HashMap<String, [f64; 2]> = refs
        .iter()
        .filter_map(|(id, r)| net.node_xy(*r).map(|p| (id.clone(), p)))
        .collect();
    let all: Vec<[f64; 2]> = xy.values().copied().collect();
    let proj = Projection::for_points(&all, net.crs.as_deref());

    let mut connected_features = Vec::new();
    let mut unconnected_features = Vec::new();
    let mut service_features = Vec::new();
    let mut fittings = Vec::new();
    let empty = Vec::new();
    let points = customers
        .get("features")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    // Taps are collected first and applied pipe by pipe, so every customer
    // is measured against the network as drawn.
    let mut taps: Vec<Tap> = Vec::new();
    for (i, f) in points.iter().enumerate() {
        let Some(at) = f["geometry"]["coordinates"]
            .as_array()
            .and_then(|c| Some([c.first()?.as_f64()?, c.get(1)?.as_f64()?]))
        else {
            continue;
        };
        let props = f.get("properties").cloned().unwrap_or(json!({}));
        let id = props["id"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("C{}", i + 1));
        let demand = props["demand_lps"]
            .as_f64()
            .unwrap_or(settings.default_customer_lps);
        let z = props["elevation_m"]
            .as_f64()
            .or_else(|| f["geometry"]["coordinates"].get(2).and_then(Value::as_f64));
        if !settings.connect_unconnected_customers {
            unconnected_features.push(unconnected(f, CUSTOMER_CONNECTION_DISABLED));
            continue;
        }
        match nearest_pipe(net, &xy, &proj, at) {
            Some((k, t, d)) if d <= settings.max_connection_m => {
                taps.push(Tap {
                    pipe: k,
                    t,
                    id,
                    at,
                    demand,
                    distance: d,
                    z,
                    feature: f.clone(),
                });
            }
            _ => unconnected_features.push(unconnected(f, BEYOND_MAX_CONNECTION_LENGTH)),
        }
    }

    // Split each tapped pipe at its taps (in order along the pipe).
    let mut by_pipe: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, tap) in taps.iter().enumerate() {
        by_pipe.entry(tap.pipe).or_default().push(i);
    }
    let mut replaced: HashMap<usize, Vec<Pipe>> = HashMap::new();
    for (&k, list) in &by_pipe {
        let mut list = list.clone();
        list.sort_by(|&a, &b| taps[a].t.total_cmp(&taps[b].t));
        let pipe = &net.pipes[k];
        let line = polyline(pipe, &xy);
        let (za, zb) = (elevation(net, &pipe.from), elevation(net, &pipe.to));
        let mut start = pipe.from.clone();
        let mut t0 = 0.0;
        let mut parts = Vec::new();
        for (n, &ti) in list.iter().enumerate() {
            let Tap {
                t,
                id: cid,
                at: cat,
                demand,
                distance: d,
                z,
                feature,
                ..
            } = &taps[ti];
            let tap_at = point_at(&line, *t);
            let tap_id = format!("{}-tap{}", pipe.id, n + 1);
            let z_tap = za + t * (zb - za);
            out.junctions.push(Junction {
                id: tap_id.clone(),
                elevation_m: z_tap,
                x: Some(tap_at[0]),
                y: Some(tap_at[1]),
                ..Junction::default()
            });
            let meter_id = format!("{cid}-meter");
            out.junctions.push(Junction {
                id: meter_id.clone(),
                elevation_m: z.unwrap_or(z_tap),
                demand_lps: *demand,
                x: Some(cat[0]),
                y: Some(cat[1]),
                ..Junction::default()
            });
            out.pipes.push(Pipe {
                id: format!("{cid}-service"),
                from: tap_id.clone(),
                to: meter_id,
                length_m: d.max(0.5),
                diameter_mm: settings.service_internal_mm,
                roughness: settings.service_roughness,
                material: Some(settings.service_material.clone()),
                pressure_class_bar: Some(settings.service_pn_bar),
                ..Pipe::default()
            });
            parts.push(Pipe {
                id: if n == 0 {
                    pipe.id.clone()
                } else {
                    format!("{}.{}", pipe.id, n)
                },
                from: start.clone(),
                to: tap_id.clone(),
                length_m: (pipe.length_m * (t - t0)).max(0.1),
                vertices: interior(&line, t0, *t),
                ..pipe.clone()
            });
            fittings.push(Fitting {
                kind: FittingKind::TappingSaddle,
                at: tap_at,
                on: pipe.id.clone(),
                internal_mm: pipe.diameter_mm,
                branch_mm: Some(settings.service_internal_mm),
                angle_deg: None,
                thrust_block: false,
            });
            service_features.push(json!({
                "type": "Feature",
                "properties": { "id": format!("{cid}-service"), "customer": cid, "length_m": d, "material": settings.service_material },
                "geometry": { "type": "LineString", "coordinates": [tap_at, cat] },
            }));
            let mut connected = feature.clone();
            connected["properties"]["network_properties"] =
                json!({ "connected": true, "pipe": pipe.id, "service_length_m": d });
            connected_features.push(connected);
            start = tap_id;
            t0 = *t;
        }
        parts.push(Pipe {
            id: format!("{}.{}", pipe.id, list.len()),
            from: start,
            to: pipe.to.clone(),
            length_m: (pipe.length_m * (1.0 - t0)).max(0.1),
            vertices: interior(&line, t0, 1.0),
            ..pipe.clone()
        });
        replaced.insert(k, parts);
    }
    let services: Vec<Pipe> = out.pipes.drain(net.pipes.len()..).collect();
    let mut pipes = Vec::new();
    for (k, p) in net.pipes.iter().enumerate() {
        match replaced.remove(&k) {
            Some(parts) => pipes.extend(parts),
            None => pipes.push(p.clone()),
        }
    }
    pipes.extend(services);
    out.pipes = pipes;

    structural_fittings(&out, &proj, settings, &mut fittings);
    let fittings_boq = boq(&fittings);
    let fc = |features: Vec<Value>| json!({ "type": "FeatureCollection", "features": features });
    let fitting_features: Vec<Value> = fittings
        .iter()
        .map(|f| json!({ "type": "Feature", "properties": f, "geometry": { "type": "Point", "coordinates": f.at } }))
        .collect();
    ProcessResult {
        connected: connected_features.len(),
        unconnected: unconnected_features.len(),
        layers: json!({
            "connected_customers": fc(connected_features),
            "unconnected_customers": fc(unconnected_features),
            "service_connections": fc(service_features),
            "fittings": fc(fitting_features),
        }),
        network: out,
        fittings,
        fittings_boq,
    }
}

fn unconnected(f: &Value, reason: &str) -> Value {
    let mut f = f.clone();
    f["properties"]["network_properties"] =
        json!({ "connected": false, "unconnected_reason": reason });
    f
}

fn elevation(net: &WaterNetwork, id: &str) -> f64 {
    net.node_refs()
        .find(|(n, _)| *n == id)
        .map(|(_, r)| net.node_elevation(r))
        .unwrap_or(0.0)
}

fn polyline(pipe: &Pipe, xy: &HashMap<String, [f64; 2]>) -> Vec<[f64; 2]> {
    let mut line = vec![xy.get(&pipe.from).copied().unwrap_or_default()];
    line.extend(pipe.vertices.iter().copied());
    line.push(xy.get(&pipe.to).copied().unwrap_or_default());
    line
}

fn fractions(line: &[[f64; 2]], proj: Option<&Projection>) -> Vec<f64> {
    let mut acc = vec![0.0];
    for w in line.windows(2) {
        let d = match proj {
            Some(p) => {
                let (a, b) = (p.forward(w[0]), p.forward(w[1]));
                (b[0] - a[0]).hypot(b[1] - a[1])
            }
            None => (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]),
        };
        acc.push(acc.last().unwrap() + d);
    }
    let total = acc.last().copied().unwrap_or(0.0).max(1e-12);
    acc.iter().map(|a| a / total).collect()
}

fn point_at(line: &[[f64; 2]], t: f64) -> [f64; 2] {
    let f = fractions(line, None);
    let i = f.partition_point(|&x| x < t).clamp(1, line.len() - 1);
    let u = ((t - f[i - 1]) / (f[i] - f[i - 1]).max(1e-12)).clamp(0.0, 1.0);
    [
        line[i - 1][0] + u * (line[i][0] - line[i - 1][0]),
        line[i - 1][1] + u * (line[i][1] - line[i - 1][1]),
    ]
}

fn interior(line: &[[f64; 2]], a: f64, b: f64) -> Vec<[f64; 2]> {
    let f = fractions(line, None);
    line.iter()
        .zip(&f)
        .filter(|(_, x)| **x > a + 1e-9 && **x < b - 1e-9)
        .map(|(p, _)| *p)
        .collect()
}

/// Nearest pipe to `at`: (pipe index, fraction along it, distance m).
fn nearest_pipe(
    net: &WaterNetwork,
    xy: &HashMap<String, [f64; 2]>,
    proj: &Projection,
    at: [f64; 2],
) -> Option<(usize, f64, f64)> {
    let p = proj.forward(at);
    let mut best: Option<(usize, f64, f64)> = None;
    for (k, pipe) in net.pipes.iter().enumerate() {
        let line: Vec<[f64; 2]> = polyline(pipe, xy)
            .into_iter()
            .map(|q| proj.forward(q))
            .collect();
        let f = fractions(&line, None);
        for s in 0..line.len() - 1 {
            let (a, b) = (line[s], line[s + 1]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len2 = dx * dx + dy * dy;
            let u = if len2 == 0.0 {
                0.0
            } else {
                (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
            };
            let d = (p[0] - a[0] - u * dx).hypot(p[1] - a[1] - u * dy);
            let t = (f[s] + u * (f[s + 1] - f[s])).clamp(0.001, 0.999);
            if best.is_none_or(|b| d < b.2) {
                best = Some((k, t, d));
            }
        }
    }
    best
}

/// Bends at deflections, tees and crosses at junctions, reducers where the
/// bore changes, end caps at dead ends. Service connections are excluded.
fn structural_fittings(
    net: &WaterNetwork,
    proj: &Projection,
    s: &ProcessSettings,
    out: &mut Vec<Fitting>,
) {
    let is_service = |p: &Pipe| p.id.ends_with("-service");
    let xy: HashMap<String, [f64; 2]> = net
        .node_refs()
        .filter_map(|(id, r)| net.node_xy(r).map(|p| (id.to_owned(), p)))
        .collect();
    // Bends inside pipes.
    for pipe in net.pipes.iter().filter(|p| !is_service(p)) {
        let line: Vec<[f64; 2]> = polyline(pipe, &xy);
        let m: Vec<[f64; 2]> = line.iter().map(|&q| proj.forward(q)).collect();
        for i in 1..m.len().saturating_sub(1) {
            if let Some(angle) = deflection(m[i - 1], m[i], m[i + 1]) {
                push_bend(out, angle, line[i], &pipe.id, pipe.diameter_mm, s);
            }
        }
    }
    // Node fittings.
    let mut incident: HashMap<&str, Vec<&Pipe>> = HashMap::new();
    for p in net.pipes.iter().filter(|p| !is_service(p)) {
        incident.entry(p.from.as_str()).or_default().push(p);
        incident.entry(p.to.as_str()).or_default().push(p);
    }
    let fixed: Vec<&str> = net
        .reservoirs
        .iter()
        .map(|r| r.id.as_str())
        .chain(net.tanks.iter().map(|t| t.id.as_str()))
        .collect();
    let mut ids: Vec<&&str> = incident.keys().collect();
    ids.sort();
    for id in ids {
        let pipes = &incident[*id];
        let Some(&at) = xy.get(*id) else { continue };
        let tapped = id.contains("-tap");
        let mut bores: Vec<f64> = pipes.iter().map(|p| p.diameter_mm).collect();
        bores.sort_by(|a, b| b.total_cmp(a));
        let main = bores[0];
        match pipes.len() {
            1 if !fixed.contains(id) && !tapped => out.push(Fitting {
                kind: FittingKind::EndCap,
                at,
                on: (*id).to_owned(),
                internal_mm: main,
                branch_mm: None,
                angle_deg: None,
                thrust_block: true,
            }),
            2 => {
                if (bores[0] - bores[1]).abs() > 1e-6 {
                    out.push(Fitting {
                        kind: FittingKind::Reducer,
                        at,
                        on: (*id).to_owned(),
                        internal_mm: bores[0],
                        branch_mm: Some(bores[1]),
                        angle_deg: None,
                        thrust_block: false,
                    });
                }
                if !tapped {
                    let neighbour = |p: &Pipe| {
                        let line = polyline(p, &xy);
                        if p.from == **id {
                            line[1]
                        } else {
                            line[line.len() - 2]
                        }
                    };
                    let (a, b) = (
                        proj.forward(neighbour(pipes[0])),
                        proj.forward(neighbour(pipes[1])),
                    );
                    if let Some(angle) = deflection(a, proj.forward(at), b) {
                        push_bend(out, angle, at, id, main, s);
                    }
                }
            }
            3 => out.push(Fitting {
                kind: FittingKind::Tee,
                at,
                on: (*id).to_owned(),
                internal_mm: main,
                branch_mm: Some(bores[2]),
                angle_deg: None,
                thrust_block: true,
            }),
            n if n >= 4 => out.push(Fitting {
                kind: FittingKind::Cross,
                at,
                on: (*id).to_owned(),
                internal_mm: main,
                branch_mm: Some(bores[n - 1]),
                angle_deg: None,
                thrust_block: true,
            }),
            _ => {}
        }
    }
}

/// Deflection in degrees of the path a → b → c, or `None` when straight.
fn deflection(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Option<f64> {
    let (u, v) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
    let (lu, lv) = (u[0].hypot(u[1]), v[0].hypot(v[1]));
    if lu < 1e-9 || lv < 1e-9 {
        return None;
    }
    let cos = ((u[0] * v[0] + u[1] * v[1]) / (lu * lv)).clamp(-1.0, 1.0);
    Some(cos.acos().to_degrees())
}

fn push_bend(
    out: &mut Vec<Fitting>,
    angle: f64,
    at: [f64; 2],
    on: &str,
    bore: f64,
    s: &ProcessSettings,
) {
    if angle < s.min_bend_deg {
        return;
    }
    // Round up to the next standard bend; beyond 90° use two 45°+ bends as
    // one 90° entry per 90° of turn.
    let standard = BENDS
        .into_iter()
        .find(|&b| b >= angle - 1e-9)
        .unwrap_or(90.0);
    let count = if angle > 90.0 {
        (angle / 90.0).ceil() as usize
    } else {
        1
    };
    for _ in 0..count {
        out.push(Fitting {
            kind: FittingKind::Bend,
            at,
            on: on.to_owned(),
            internal_mm: bore,
            branch_mm: None,
            angle_deg: Some(standard),
            thrust_block: standard >= s.thrust_block_from_deg,
        });
    }
}

fn boq(fittings: &[Fitting]) -> Vec<FittingLine> {
    let mut lines: Vec<FittingLine> = Vec::new();
    for f in fittings {
        match lines.iter_mut().find(|l| {
            l.kind == f.kind
                && l.internal_mm == f.internal_mm
                && l.branch_mm == f.branch_mm
                && l.angle_deg == f.angle_deg
        }) {
            Some(l) => {
                l.count += 1;
                l.thrust_blocks += usize::from(f.thrust_block);
            }
            None => lines.push(FittingLine {
                kind: f.kind,
                internal_mm: f.internal_mm,
                branch_mm: f.branch_mm,
                angle_deg: f.angle_deg,
                count: 1,
                thrust_blocks: usize::from(f.thrust_block),
            }),
        }
    }
    lines.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then(b.internal_mm.total_cmp(&a.internal_mm))
    });
    lines
}

#[cfg(test)]
mod tests;
