use serde_json::{Value, json};

use super::*;

fn point(x: f64, y: f64, z: f64, props: Value) -> Value {
    json!({ "type": "Feature", "properties": props, "geometry": { "type": "Point", "coordinates": [x, y, z] } })
}

fn line(coords: &[[f64; 2]], props: Value) -> Value {
    json!({ "type": "Feature", "properties": props, "geometry": { "type": "LineString", "coordinates": coords } })
}

fn fc(features: Vec<Value>) -> Value {
    json!({ "type": "FeatureCollection", "features": features })
}

fn metric() -> LayoutOptions {
    LayoutOptions {
        crs: Some("EPSG:32735".into()),
        ..LayoutOptions::default()
    }
}

#[test]
fn drawn_pipes_split_at_a_t_and_take_demand() {
    let input = fc(vec![
        line(
            &[[0.0, 0.0], [200.0, 0.0]],
            json!({ "id": "A", "diameter_mm": 160 }),
        ),
        // B starts on A's interior: a T-junction at x = 120.
        line(&[[120.0, 0.0], [120.0, 150.0]], json!({ "id": "B" })),
        point(
            0.0,
            0.0,
            1500.0,
            json!({ "water_kind": "source", "id": "Spring", "head_m": 1560 }),
        ),
        point(
            120.0,
            150.0,
            1480.0,
            json!({ "water_kind": "standpost", "id": "SP1", "demand_lps": 0.5 }),
        ),
        point(
            60.0,
            10.0,
            1490.0,
            json!({ "water_kind": "household", "population": 6 }),
        ),
        point(
            5000.0,
            5000.0,
            1490.0,
            json!({ "water_kind": "household", "id": "far", "population": 6 }),
        ),
    ]);
    let r = layout(&input, &metric(), None).unwrap();
    assert_eq!(r.mode, LayoutMode::Pipes);
    let n = &r.network;
    assert_eq!(n.reservoirs.len(), 1);
    assert_eq!(n.reservoirs[0].head_m, 1560.0);
    // A splits at the household tap and at the T; B ends at the standpost.
    assert_eq!(n.pipes.len(), 4, "{:#?}", n.pipes);
    let total: f64 = n.pipes.iter().map(|p| p.length_m).sum();
    // Plan 350 m; slope adds a little where the ends differ in height.
    assert!((350.0..353.0).contains(&total), "total {total}");
    assert!(
        n.pipes
            .iter()
            .filter(|p| p.id.starts_with('A'))
            .all(|p| p.diameter_mm == 160.0)
    );
    let sp = n
        .junctions
        .iter()
        .find(|j| j.id == "SP1")
        .expect("standpost junction");
    assert_eq!(sp.demand_lps, 0.5);
    let household = 6.0 * 20.0 * 2.0 / 86_400.0;
    let demand: f64 = n.junctions.iter().map(|j| j.demand_lps).sum();
    assert!((demand - 0.5 - household).abs() < 1e-12);
    assert_eq!(r.unserved, vec!["far".to_owned()]);
    assert!(
        ds_water_model::validate(n).is_empty(),
        "{:?}",
        ds_water_model::validate(n)
    );
    assert_eq!(r.paths[0].label, "01");
    assert_eq!(r.paths[0].root, "Spring");
}

#[test]
fn roads_grow_only_the_supply_tree() {
    // A road cross; demand on the east and north arms only.
    let input = fc(vec![
        line(
            &[[-500.0, 0.0], [0.0, 0.0], [500.0, 0.0]],
            json!({ "water_kind": "road" }),
        ),
        line(
            &[[0.0, -500.0], [0.0, 0.0], [0.0, 500.0]],
            json!({ "water_kind": "road" }),
        ),
        point(
            -500.0,
            0.0,
            1600.0,
            json!({ "water_kind": "tank", "id": "Tank" }),
        ),
        point(
            450.0,
            30.0,
            1550.0,
            json!({ "water_kind": "kiosk", "id": "K1", "demand_lps": 1.0 }),
        ),
        point(
            20.0,
            400.0,
            1540.0,
            json!({ "water_kind": "kiosk", "id": "K2", "demand_lps": 1.0 }),
        ),
    ]);
    let r = layout(&input, &metric(), None).unwrap();
    assert_eq!(r.mode, LayoutMode::Roads);
    let n = &r.network;
    // West arm 500 + east to tap 450 + service 30 + north to tap 400 + 20.
    let total: f64 = n.pipes.iter().map(|p| p.length_m).sum();
    assert!((1400.0..1404.0).contains(&total), "total {total}");
    // The south arm carries no demand, so no pipe goes there.
    assert!(n.junctions.iter().all(|j| j.y.unwrap() > -1.0));
    assert!(r.unserved.is_empty());
    assert_eq!(n.tanks.len(), 1);
}

#[test]
fn points_alone_join_by_minimum_spanning_tree() {
    let mut features = vec![point(
        0.0,
        0.0,
        100.0,
        json!({ "water_kind": "source", "id": "S" }),
    )];
    for i in 1..=10 {
        features.push(point(
            i as f64 * 100.0,
            (i % 2) as f64 * 10.0,
            90.0,
            json!({ "demand_lps": 0.1 }),
        ));
    }
    let r = layout(&fc(features), &metric(), None).unwrap();
    assert_eq!(r.mode, LayoutMode::Spanning);
    assert_eq!(r.network.pipes.len(), 10);
    assert_eq!(r.paths.len(), 1);
    assert_eq!(r.paths[0].nodes.len(), 11);
    assert_eq!(r.network.reservoirs[0].head_m, 103.0);
}

#[test]
fn geographic_input_measures_on_the_sphere() {
    // 0.01° of latitude near Kigali is about 1,112 m.
    let input = fc(vec![
        line(&[[30.06, -1.95], [30.06, -1.94]], json!({})),
        point(30.06, -1.95, 1500.0, json!({ "water_kind": "source" })),
        point(
            30.06,
            -1.94,
            1500.0,
            json!({ "water_kind": "standpost", "demand_lps": 0.2 }),
        ),
    ]);
    let r = layout(&input, &LayoutOptions::default(), None).unwrap();
    assert_eq!(r.network.crs.as_deref(), Some("EPSG:4326"));
    assert!(
        (r.network.pipes[0].length_m - 1111.95).abs() < 1.0,
        "{}",
        r.network.pipes[0].length_m
    );
}

#[test]
fn missing_elevation_comes_from_the_surface_then_neighbours() {
    let input = fc(vec![
        line(&[[0.0, 0.0], [100.0, 0.0], [200.0, 0.0]], json!({})),
        json!({ "type": "Feature", "properties": { "water_kind": "source" }, "geometry": { "type": "Point", "coordinates": [0.0, 0.0] } }),
        json!({ "type": "Feature", "properties": { "water_kind": "junction" }, "geometry": { "type": "Point", "coordinates": [200.0, 0.0] } }),
    ]);
    let surface = |p: [f64; 2]| Some(1000.0 - p[0] / 10.0);
    let r = layout(&input, &metric(), Some(&surface)).unwrap();
    assert_eq!(r.network.junctions[0].elevation_m, 980.0);
    assert_eq!(r.network.reservoirs[0].head_m, 1003.0);
}

#[test]
fn no_source_is_refused() {
    let input = fc(vec![point(0.0, 0.0, 1.0, json!({ "demand_lps": 1 }))]);
    assert!(matches!(
        layout(&input, &metric(), None),
        Err(LayoutError::NoSource)
    ));
}
