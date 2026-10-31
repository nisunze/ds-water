use ds_water_model::*;
use serde_json::json;

use super::*;

/// A main R → A (east 200 m) → B (bent 90° north 200 m), a branch at A.
fn main_with_bend() -> WaterNetwork {
    let mut n = WaterNetwork::new("street");
    n.crs = Some("EPSG:32735".into());
    n.reservoirs.push(Reservoir {
        id: "R".into(),
        head_m: 100.0,
        x: Some(0.0),
        y: Some(0.0),
        ..Reservoir::default()
    });
    for (id, x, y) in [("A", 200.0, 0.0), ("B", 200.0, 200.0), ("C", 300.0, 0.0)] {
        n.junctions.push(Junction {
            id: id.into(),
            elevation_m: 50.0,
            x: Some(x),
            y: Some(y),
            ..Junction::default()
        });
    }
    let pipe = |id: &str, from: &str, to: &str, d: f64, v: Vec<[f64; 2]>| Pipe {
        id: id.into(),
        from: from.into(),
        to: to.into(),
        length_m: 200.0,
        diameter_mm: d,
        roughness: 150.0,
        vertices: v,
        ..Pipe::default()
    };
    n.pipes.push(pipe("P1", "R", "A", 110.0, vec![]));
    // A 30° kink in the middle of P2.
    n.pipes
        .push(pipe("P2", "A", "B", 90.0, vec![[200.0, 100.0]]));
    n.pipes.push(pipe("P3", "A", "C", 63.0, vec![]));
    n
}

fn customers() -> serde_json::Value {
    json!({ "type": "FeatureCollection", "features": [
        { "type": "Feature", "properties": { "id": "H1", "demand_lps": 0.05 }, "geometry": { "type": "Point", "coordinates": [100.0, 20.0] } },
        { "type": "Feature", "properties": { "id": "H2" }, "geometry": { "type": "Point", "coordinates": [160.0, -15.0] } },
        { "type": "Feature", "properties": { "id": "Far" }, "geometry": { "type": "Point", "coordinates": [100.0, 400.0] } }
    ]})
}

#[test]
fn customers_connect_like_lv_with_reasons_for_the_rest() {
    let r = process(&main_with_bend(), &customers(), &ProcessSettings::default());
    assert_eq!((r.connected, r.unconnected), (2, 1));
    let left = &r.layers["unconnected_customers"]["features"][0];
    assert_eq!(
        left["properties"]["network_properties"]["unconnected_reason"],
        BEYOND_MAX_CONNECTION_LENGTH
    );
    // Two service pipes, and P1 split at both taps (three pieces).
    let services: Vec<&Pipe> = r
        .network
        .pipes
        .iter()
        .filter(|p| p.id.ends_with("-service"))
        .collect();
    assert_eq!(services.len(), 2);
    assert!((services[0].length_m - 20.0).abs() < 1e-6);
    let p1: f64 = r
        .network
        .pipes
        .iter()
        .filter(|p| p.id == "P1" || p.id.starts_with("P1."))
        .map(|p| p.length_m)
        .sum();
    assert!((p1 - 200.0).abs() < 1e-6, "{p1}");
    assert!(
        ds_water_model::validate(&r.network).is_empty(),
        "{:?}",
        ds_water_model::validate(&r.network)
    );
    let h1 = r
        .network
        .junctions
        .iter()
        .find(|j| j.id == "H1-meter")
        .unwrap();
    assert_eq!(h1.demand_lps, 0.05);
}

#[test]
fn disabling_connection_leaves_every_customer_with_lv_reason() {
    let settings = ProcessSettings {
        connect_unconnected_customers: false,
        ..ProcessSettings::default()
    };
    let r = process(&main_with_bend(), &customers(), &settings);
    assert_eq!((r.connected, r.unconnected), (0, 3));
    for f in r.layers["unconnected_customers"]["features"]
        .as_array()
        .unwrap()
    {
        assert_eq!(
            f["properties"]["network_properties"]["unconnected_reason"],
            CUSTOMER_CONNECTION_DISABLED
        );
    }
}

#[test]
fn fittings_follow_the_geometry() {
    let r = process(
        &main_with_bend(),
        &json!({ "features": [] }),
        &ProcessSettings::default(),
    );
    let count = |k: FittingKind| r.fittings.iter().filter(|f| f.kind == k).count();
    // A is a three-way junction: a tee (110 main, 63 branch).
    assert_eq!(count(FittingKind::Tee), 1);
    let tee = r
        .fittings
        .iter()
        .find(|f| f.kind == FittingKind::Tee)
        .unwrap();
    assert_eq!((tee.internal_mm, tee.branch_mm), (110.0, Some(63.0)));
    // B and C are dead ends.
    assert_eq!(count(FittingKind::EndCap), 2);
    // P2 runs straight north from A through its vertex: no bend inside.
    assert_eq!(count(FittingKind::Bend), 0);
    assert_eq!(
        r.fittings_boq.iter().map(|l| l.count).sum::<usize>(),
        r.fittings.len()
    );
}

#[test]
fn bends_round_up_to_standard_angles_with_thrust_blocks() {
    let mut n = main_with_bend();
    n.pipes[1].vertices = vec![[200.0, 100.0], [230.0, 150.0]];
    let r = process(&n, &json!({ "features": [] }), &ProcessSettings::default());
    let bends: Vec<&Fitting> = r
        .fittings
        .iter()
        .filter(|f| f.kind == FittingKind::Bend)
        .collect();
    // About 31° (a 45° bend), then about 62° back across (a 90° bend); both
    // need thrust blocks.
    let angles: Vec<f64> = bends.iter().filter_map(|b| b.angle_deg).collect();
    assert_eq!(angles, vec![45.0, 90.0], "{bends:?}");
    assert!(bends.iter().all(|b| b.thrust_block));
}
