//! The Water command surface.
//!
//! Every host — the `ds-water` CLI, the browser's WASM unit and, through the
//! Kernel, Core and the `ds water` chapter — calls [`invoke`] with an
//! operation name and a JSON request and gets a JSON answer. No host
//! sequences engine steps itself; `scheme` is the whole chain in one call.
//!
//! Ground, when an operation needs it, arrives in the request as a
//! [`GridSurface`] that the caller sampled from the project's existing
//! raster/terrain layers; the engine never reads raster files.

pub mod analysis;

use ds_water_design::{BoqLine, Catalogue, DesignCriteria, DesignResult};
use ds_water_drawing::{DrawingOptions, Sheet, TitleInfo};
use ds_water_hydraulics::{Hydraulics, Snapshot};
use ds_water_layout::{LayoutOptions, LayoutResult};
use ds_water_model::{WaterNetwork, inp};
use ds_water_profile::{ProfileOptions, ProfileResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const OPERATIONS: [&str; 11] = [
    "identity", "validate", "simulate", "analysis", "layout", "process", "design", "profile",
    "draw", "scheme", "example",
];

#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error("unknown water operation {0}; known: {OPERATIONS:?}")]
    Unknown(String),
    #[error("invalid request: {0}")]
    Request(#[from] serde_json::Error),
    #[error("{0}")]
    Engine(String),
}

pub(crate) fn engine<E: std::fmt::Display>(e: E) -> OpError {
    OpError::Engine(e.to_string())
}

/// A profile with the label of the path it follows.
pub type LabelledProfile = (String, ProfileResult);

/// A regular elevation grid in the network's coordinates (row 0 at the top).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GridSurface {
    pub x0: f64,
    pub y_top: f64,
    pub cell: f64,
    pub cols: usize,
    pub rows: usize,
    #[serde(default)]
    pub nodata: Option<f64>,
    pub z: Vec<f64>,
}

impl GridSurface {
    /// Bilinear elevation at `p`, or `None` off the grid or on no-data.
    pub fn at(&self, p: [f64; 2]) -> Option<f64> {
        if self.cols == 0 || self.rows == 0 || self.z.len() != self.cols * self.rows {
            return None;
        }
        let fx = (p[0] - self.x0) / self.cell - 0.5;
        let fy = (self.y_top - p[1]) / self.cell - 0.5;
        if fx < -0.5 || fy < -0.5 || fx > self.cols as f64 - 0.5 || fy > self.rows as f64 - 0.5 {
            return None;
        }
        let (c0, r0) = (fx.floor().max(0.0) as usize, fy.floor().max(0.0) as usize);
        let (c1, r1) = ((c0 + 1).min(self.cols - 1), (r0 + 1).min(self.rows - 1));
        let (tx, ty) = (
            (fx - c0 as f64).clamp(0.0, 1.0),
            (fy - r0 as f64).clamp(0.0, 1.0),
        );
        let v = |c: usize, r: usize| {
            let z = self.z[r * self.cols + c];
            (Some(z) != self.nodata && z.is_finite()).then_some(z)
        };
        let (a, b, c, d) = (v(c0, r0)?, v(c1, r0)?, v(c0, r1)?, v(c1, r1)?);
        Some((a * (1.0 - tx) + b * tx) * (1.0 - ty) + (c * (1.0 - tx) + d * tx) * ty)
    }
}

#[derive(Debug, Deserialize)]
struct NetworkRequest {
    network: WaterNetwork,
}

#[derive(Debug, Deserialize)]
struct SimulateRequest {
    network: WaterNetwork,
    #[serde(default)]
    eps: bool,
}

#[derive(Debug, Deserialize)]
struct AnalysisRequest {
    network: WaterNetwork,
    #[serde(default)]
    criteria: analysis::AnalysisCriteria,
}

#[derive(Debug, Deserialize)]
struct LayoutRequest {
    features: Value,
    #[serde(default)]
    options: LayoutOptions,
    #[serde(default)]
    surface: Option<GridSurface>,
}

#[derive(Debug, Deserialize)]
struct ProcessRequest {
    network: WaterNetwork,
    /// Customer points (survey, household table, buildings) as GeoJSON.
    #[serde(default)]
    customers: Value,
    #[serde(default)]
    settings: ds_water_process::ProcessSettings,
}

#[derive(Debug, Deserialize)]
struct DesignRequest {
    network: WaterNetwork,
    #[serde(default)]
    criteria: DesignCriteria,
    #[serde(default)]
    catalogue: Option<Catalogue>,
}

#[derive(Debug, Deserialize)]
struct ProfileRequest {
    network: WaterNetwork,
    /// A path label from the layout ("01", "001"...); the trunk if absent.
    #[serde(default)]
    path: Option<String>,
    /// Explicit links in order, instead of a path label.
    #[serde(default)]
    links: Vec<String>,
    #[serde(default)]
    options: ProfileOptions,
    #[serde(default)]
    surface: Option<GridSurface>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct SheetRequest {
    pub title: TitleInfo,
    pub drawing: DrawingOptions,
    /// Profile at most this many paths, longest first (default 3).
    pub paths: Option<usize>,
    /// Skip paths shorter than this, m (default 200).
    pub min_path_m: Option<f64>,
    pub surface: Option<GridSurface>,
}

#[derive(Debug, Deserialize)]
struct DrawRequest {
    network: WaterNetwork,
    #[serde(default)]
    criteria: DesignCriteria,
    #[serde(flatten)]
    sheets: SheetRequest,
}

/// A hypothetical village scheme for tests and demonstrations.
#[derive(Debug, Deserialize)]
struct ExampleRequest {
    /// Where its spring is, `[lon, lat]`.
    at: [f64; 2],
    #[serde(default = "one")]
    seed: u64,
    #[serde(default = "village_households")]
    households: usize,
}

fn one() -> u64 {
    1
}

fn village_households() -> usize {
    120
}

/// The most households an example village holds.
pub const MAX_EXAMPLE_HOUSEHOLDS: usize = 2_000;

#[derive(Debug, Deserialize)]
struct SchemeRequest {
    features: Value,
    #[serde(default)]
    layout: LayoutOptions,
    #[serde(default)]
    criteria: DesignCriteria,
    #[serde(default)]
    catalogue: Option<Catalogue>,
    #[serde(flatten)]
    sheets: SheetRequest,
}

/// Run one Water operation.
pub fn invoke(operation: &str, request: Value) -> Result<Value, OpError> {
    match operation {
        "identity" => Ok(json!({
            "engine": "ds-water",
            "version": env!("CARGO_PKG_VERSION"),
            "operations": OPERATIONS,
        })),
        "validate" => {
            let r: NetworkRequest = serde_json::from_value(request)?;
            let issues = ds_water_model::validate(&r.network);
            Ok(json!({ "ok": issues.is_empty(), "issues": issues }))
        }
        "simulate" => {
            let r: SimulateRequest = serde_json::from_value(request)?;
            let steps = simulate(&r.network, r.eps)?;
            Ok(json!({ "ok": steps.iter().all(|s| s.converged), "steps": steps }))
        }
        "analysis" => {
            let r: AnalysisRequest = serde_json::from_value(request)?;
            analysis::analyse(&r.network, &r.criteria)
        }
        "layout" => {
            let r: LayoutRequest = serde_json::from_value(request)?;
            Ok(serde_json::to_value(layout(
                &r.features,
                &r.options,
                r.surface.as_ref(),
            )?)?)
        }
        "process" => {
            let r: ProcessRequest = serde_json::from_value(request)?;
            Ok(serde_json::to_value(ds_water_process::process(
                &r.network,
                &r.customers,
                &r.settings,
            ))?)
        }
        "design" => {
            let r: DesignRequest = serde_json::from_value(request)?;
            let catalogue = r.catalogue.unwrap_or_else(Catalogue::standard);
            Ok(serde_json::to_value(
                ds_water_design::design(&r.network, &catalogue, &r.criteria).map_err(engine)?,
            )?)
        }
        "profile" => {
            let r: ProfileRequest = serde_json::from_value(request)?;
            let links = if r.links.is_empty() {
                path_links(&r.network, r.path.as_deref())?
            } else {
                r.links
            };
            Ok(serde_json::to_value(profile(
                &r.network,
                &links,
                r.surface.as_ref(),
                &r.options,
            )?)?)
        }
        "draw" => {
            let r: DrawRequest = serde_json::from_value(request)?;
            let boq = boq_as_is(&r.network);
            let (sheets, profiles) = draw(&r.network, &boq, &r.criteria, &r.sheets)?;
            Ok(json!({ "sheets": sheets, "profiles": profiles }))
        }
        "scheme" => {
            let r: SchemeRequest = serde_json::from_value(request)?;
            let laid = layout(&r.features, &r.layout, r.sheets.surface.as_ref())?;
            let catalogue = r.catalogue.unwrap_or_else(Catalogue::standard);
            let designed =
                ds_water_design::design(&laid.network, &catalogue, &r.criteria).map_err(engine)?;
            let (sheets, profiles) =
                draw(&designed.network, &designed.boq, &r.criteria, &r.sheets)?;
            Ok(json!({
                "layout": { "mode": laid.mode, "paths": laid.paths.len(), "unserved": laid.unserved, "notes": laid.notes },
                "design": summary(&designed),
                "network": designed.network,
                "inp": inp::write(&designed.network),
                "design_report": designed,
                "sheets": sheets,
                "profiles": profiles,
            }))
        }
        "example" => {
            let r: ExampleRequest = serde_json::from_value(request)?;
            let valid = r.at[0].abs() <= 180.0 && r.at[1].abs() <= 85.0;
            if !valid || r.households == 0 || r.households > MAX_EXAMPLE_HOUSEHOLDS {
                return Err(OpError::Engine(format!(
                    "an example village needs a lon/lat `at` and 1..={MAX_EXAMPLE_HOUSEHOLDS} households"
                )));
            }
            let features = ds_water_synth::village::Village {
                at: r.at,
                seed: r.seed,
                households: r.households,
            }
            .features();
            let laid = layout(&features, &LayoutOptions::default(), None)?;
            let designed = ds_water_design::design(
                &laid.network,
                &Catalogue::standard(),
                &DesignCriteria::default(),
            )
            .map_err(engine)?;
            let mut network = designed.network.clone();
            network.name = format!("Example village {} (seed {})", r.households, r.seed);
            Ok(json!({
                "layout": { "mode": laid.mode, "paths": laid.paths.len(), "unserved": laid.unserved, "notes": laid.notes },
                "design": summary(&designed),
                "network": network,
                "features": features,
            }))
        }
        other => Err(OpError::Unknown(other.to_owned())),
    }
}

pub fn simulate(n: &WaterNetwork, eps: bool) -> Result<Vec<Snapshot>, OpError> {
    let mut h = Hydraulics::new(n).map_err(engine)?;
    Ok(if eps {
        h.run(n)
    } else {
        vec![h.solve(0, &Hydraulics::initial_tank_levels(n))]
    })
}

pub fn layout(
    features: &Value,
    opts: &LayoutOptions,
    surface: Option<&GridSurface>,
) -> Result<LayoutResult, OpError> {
    let at = |p: [f64; 2]| surface.and_then(|g| g.at(p));
    ds_water_layout::layout(
        features,
        opts,
        surface.map(|_| &at as &dyn Fn([f64; 2]) -> Option<f64>),
    )
    .map_err(engine)
}

fn path_links(n: &WaterNetwork, label: Option<&str>) -> Result<Vec<String>, OpError> {
    let paths = ds_water_layout::orient(n).map_err(engine)?;
    let found = match label {
        Some(l) => paths.into_iter().find(|p| p.label == l),
        None => paths.into_iter().next(),
    };
    found
        .map(|p| p.pipes)
        .ok_or_else(|| OpError::Engine(format!("no path {}", label.unwrap_or("(trunk)"))))
}

pub fn profile(
    n: &WaterNetwork,
    links: &[String],
    surface: Option<&GridSurface>,
    opts: &ProfileOptions,
) -> Result<ProfileResult, OpError> {
    let levels = Hydraulics::initial_tank_levels(n);
    let peak = Hydraulics::new(n).map_err(engine)?.solve(0, &levels);
    let mut still_net = n.clone();
    still_net.options.demand_multiplier = 0.0;
    let still = Hydraulics::new(&still_net)
        .map_err(engine)?
        .solve(0, &levels);
    let at = |p: [f64; 2]| surface.and_then(|g| g.at(p));
    ds_water_profile::profile(
        n,
        links,
        &peak,
        &still,
        surface.map(|_| &at as &dyn Fn([f64; 2]) -> Option<f64>),
        opts,
    )
    .map_err(engine)
}

/// Quantities of a network as it is (no design run, no costs).
pub fn boq_as_is(n: &WaterNetwork) -> Vec<BoqLine> {
    let mut out: Vec<BoqLine> = Vec::new();
    for p in &n.pipes {
        let material = p.material.clone().unwrap_or_else(|| "pipe".into());
        let pn = p.pressure_class_bar.unwrap_or(0.0);
        match out
            .iter_mut()
            .find(|b| b.material == material && b.nominal_mm == p.diameter_mm && b.pn_bar == pn)
        {
            Some(b) => b.length_m += p.length_m,
            None => out.push(BoqLine {
                material,
                nominal_mm: p.diameter_mm,
                pn_bar: pn,
                length_m: p.length_m,
                cost: 0.0,
            }),
        }
    }
    out
}

/// The sheet set: general layout, plan & profile of the longest paths,
/// schedules. Returns the sheets and the profiles they were drawn from.
pub fn draw(
    n: &WaterNetwork,
    boq: &[BoqLine],
    criteria: &DesignCriteria,
    req: &SheetRequest,
) -> Result<(Vec<Sheet>, Vec<LabelledProfile>), OpError> {
    let mut info = req.title.clone();
    if info.project.is_empty() {
        info.project = n.name.clone();
    }
    if info.drawing_prefix.is_empty() {
        info.drawing_prefix = "WS".into();
    }
    let profile_opts = ProfileOptions {
        min_pressure_m: criteria.min_pressure_m,
        max_static_pressure_m: criteria.max_static_pressure_m,
        ..ProfileOptions::default()
    };
    let mut paths = ds_water_layout::orient(n).map_err(engine)?;
    paths.retain(|p| p.length_m >= req.min_path_m.unwrap_or(200.0));
    paths.sort_by(|a, b| b.length_m.total_cmp(&a.length_m));
    paths.truncate(req.paths.unwrap_or(3));
    let opts = &req.drawing;
    let mut sheets = vec![ds_water_drawing::network_plan_sheet(
        n,
        &info,
        opts,
        &format!("{}-001", info.drawing_prefix),
    )];
    let mut profiles = Vec::new();
    for path in &paths {
        let p = profile(n, &path.pipes, req.surface.as_ref(), &profile_opts)?;
        sheets.extend(ds_water_drawing::plan_profile_sheets(
            n,
            &p,
            &path.label,
            &info,
            opts,
            sheets.len() + 1,
        ));
        profiles.push((path.label.clone(), p));
    }
    let refs: Vec<(String, &ProfileResult)> =
        profiles.iter().map(|(l, p)| (l.clone(), p)).collect();
    let number = format!("{}-{:03}", info.drawing_prefix, sheets.len() + 1);
    sheets.push(ds_water_drawing::schedules_sheet(
        boq, &refs, criteria, &info, opts, &number,
    ));
    Ok((sheets, profiles))
}

pub fn summary(r: &DesignResult) -> Value {
    json!({
        "ok": r.infeasible.is_empty(),
        "total_cost": r.total_cost,
        "boq": r.boq,
        "min_pressure": r.min_pressure,
        "infeasible": r.infeasible.len(),
        "static_deficit": r.static_deficit.len(),
        "over_pressure": r.over_pressure.len(),
        "zone_devices": r.break_pressure_tanks.len(),
        "low_velocity_pipes": r.low_velocity_pipes.len(),
        "simulations": r.simulations,
        "upgrades": r.upgrades,
        "warnings": r.warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_example_village_is_placed_designed_and_repeatable() {
        let request = json!({"at": [29.72, -2.77], "seed": 3, "households": 60});
        let answer = invoke("example", request.clone()).expect("an example village");
        let network = &answer["network"];
        assert_eq!(network["crs"], "EPSG:4326");
        assert!(network["junctions"].as_array().unwrap().len() >= 10);
        assert!(!network["pipes"].as_array().unwrap().is_empty());
        assert!(
            network["reservoirs"].as_array().unwrap().len() == 1,
            "one spring"
        );
        let x = network["junctions"][0]["x"].as_f64().unwrap();
        let y = network["junctions"][0]["y"].as_f64().unwrap();
        assert!(
            (x - 29.72).abs() < 0.05 && (y + 2.77).abs() < 0.05,
            "near the point: {x}, {y}"
        );
        assert_eq!(
            answer["layout"]["unserved"],
            json!([]),
            "every household is reached"
        );
        assert_eq!(
            invoke("example", request).unwrap(),
            answer,
            "the same seed, the same village"
        );
        let other = invoke(
            "example",
            json!({"at": [29.72, -2.77], "seed": 4, "households": 60}),
        )
        .unwrap();
        assert_ne!(other["network"], answer["network"]);
        assert!(invoke("example", json!({"at": [400.0, 0.0]})).is_err());
        assert!(invoke("example", json!({"at": [29.7, -2.7], "households": 0})).is_err());
        assert!(invoke("example", json!({"seed": 1})).is_err());
    }

    fn fixture() -> Value {
        json!({
            "type": "FeatureCollection",
            "features": [
                { "type": "Feature", "properties": { "water_kind": "spring", "id": "S", "elevation_m": 1600 },
                  "geometry": { "type": "Point", "coordinates": [0.0, 0.0] } },
                { "type": "Feature", "properties": { "water_kind": "road" },
                  "geometry": { "type": "LineString", "coordinates": [[0.0, 0.0], [600.0, -300.0], [1200.0, -900.0]] } },
                { "type": "Feature", "properties": { "water_kind": "standpost", "population": 300, "elevation_m": 1560 },
                  "geometry": { "type": "Point", "coordinates": [1180.0, -880.0] } },
                { "type": "Feature", "properties": { "water_kind": "primary_school", "students": 600, "elevation_m": 1575 },
                  "geometry": { "type": "Point", "coordinates": [610.0, -280.0] } }
            ]
        })
    }

    #[test]
    fn the_whole_scheme_runs_through_one_call() {
        let out = invoke(
            "scheme",
            json!({ "features": fixture(), "layout": { "crs": "EPSG:32735" }, "title": { "project": "Ops test" }, "min_path_m": 100.0 }),
        )
        .unwrap();
        assert_eq!(out["design"]["infeasible"], 0);
        assert!(out["inp"].as_str().unwrap().contains("[PIPES]"));
        let sheets = out["sheets"].as_array().unwrap();
        assert!(sheets.len() >= 3, "{}", sheets.len());
        assert!(sheets[0]["svg"].as_str().unwrap().starts_with("<svg"));
    }

    #[test]
    fn processed_customers_are_served_by_the_solver() {
        let laid = invoke(
            "layout",
            json!({ "features": fixture(), "options": { "crs": "EPSG:32735" } }),
        )
        .unwrap();
        let customers = json!({ "type": "FeatureCollection", "features": [
            { "type": "Feature", "properties": { "id": "H1", "demand_lps": 0.1 }, "geometry": { "type": "Point", "coordinates": [300.0, -170.0] } }
        ]});
        let processed = invoke(
            "process",
            json!({ "network": laid["network"], "customers": customers }),
        )
        .unwrap();
        assert_eq!(processed["connected"], 1);
        let solved = invoke("simulate", json!({ "network": processed["network"] })).unwrap();
        assert_eq!(solved["ok"], true);
    }

    #[test]
    fn analysis_reads_like_lv_voltage_drop() {
        let laid = invoke(
            "layout",
            json!({ "features": fixture(), "options": { "crs": "EPSG:32735" } }),
        )
        .unwrap();
        let designed = invoke("design", json!({ "network": laid["network"] })).unwrap();
        let a = invoke("analysis", json!({ "network": designed["network"] })).unwrap();
        assert_eq!(a["converged"], true);
        let junctions = a["layers"]["water_junctions"]["features"]
            .as_array()
            .unwrap();
        assert!(!junctions.is_empty());
        for j in junctions {
            let p = &j["properties"];
            for column in ["p_m", "p_ok", "pd_m", "pd_pct", "pd_worst_m"] {
                assert!(!p[column].is_null(), "{column} missing: {p}");
            }
            // Worst downstream is never better than here.
            assert!(p["pd_worst_m"].as_f64().unwrap() <= p["p_m"].as_f64().unwrap() + 1e-9);
            // Drop from the static line is never negative at peak.
            assert!(p["pd_m"].as_f64().unwrap() >= -1e-6);
        }
        let path = &a["paths"][0];
        assert!(path["worst_junction"].is_string());
        assert_eq!(
            a["ok"],
            json!(
                a["paths"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|p| p["ok"] == true)
            )
        );
    }

    #[test]
    fn unknown_operations_and_bad_requests_refuse() {
        assert!(matches!(
            invoke("teleport", json!({})),
            Err(OpError::Unknown(_))
        ));
        assert!(matches!(
            invoke("simulate", json!({ "nope": 1 })),
            Err(OpError::Request(_))
        ));
        assert_eq!(
            invoke("identity", Value::Null).unwrap()["engine"],
            "ds-water"
        );
    }

    #[test]
    fn a_grid_surface_interpolates_and_refuses_outside() {
        let g = GridSurface {
            x0: 0.0,
            y_top: 2.0,
            cell: 1.0,
            cols: 2,
            rows: 2,
            nodata: None,
            z: vec![10.0, 20.0, 30.0, 40.0],
        };
        assert_eq!(g.at([1.0, 1.0]), Some(25.0));
        assert_eq!(g.at([5.0, 5.0]), None);
    }
}
