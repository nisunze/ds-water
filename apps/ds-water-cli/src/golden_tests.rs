//! Reference results that pin the Water engine's contract
//! (docs/water-network-contract.md §6). A change to layout, zoning, sizing
//! or hydraulics that moves one of these numbers is a deliberate engineering
//! change: update the reference here with the reason in the commit, never
//! loosen a tolerance.

use std::path::Path;

use serde_json::{Value, json};

fn invoke(operation: &str, request: Value) -> Value {
    ds_water_ops::invoke(operation, request).expect(operation)
}

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/hill-villages")
        .join(name)
}

fn close(actual: &Value, expected: f64, tolerance: f64, what: &str) {
    let actual = actual
        .as_f64()
        .unwrap_or_else(|| panic!("{what} is a number"));
    assert!(
        (actual - expected).abs() <= tolerance,
        "{what}: {actual} differs from the reference {expected}"
    );
}

fn length_m(network: &Value) -> f64 {
    network["pipes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pipe| pipe["length_m"].as_f64().unwrap())
        .sum()
}

fn count(network: &Value, key: &str) -> usize {
    network[key].as_array().map_or(0, Vec::len)
}

/// Example village, seed 1: a gravity scheme every household reaches with
/// pressure to spare (A Testing Project's water_test_village_1).
#[test]
fn example_village_seed_1_is_the_reference_scheme() {
    let answer = invoke("example", json!({"at": [29.70, -2.75], "seed": 1}));
    let (network, design) = (&answer["network"], &answer["design"]);
    assert_eq!(
        (count(network, "junctions"), count(network, "pipes")),
        (214, 214)
    );
    assert!((length_m(network) - 6530.5).abs() < 0.1);
    assert_eq!(answer["layout"]["paths"], 94);
    assert_eq!(answer["layout"]["unserved"], json!([]));
    assert_eq!(design["ok"], true);
    close(&design["total_cost"], 68521.58, 0.01, "cost");
    assert_eq!(design["min_pressure"]["id"], "S4");
    close(
        &design["min_pressure"]["pressure_m"],
        12.83,
        0.01,
        "lowest pressure",
    );
    assert_eq!(
        (design["infeasible"].clone(), design["zone_devices"].clone()),
        (json!(0), json!(0))
    );
}

/// Example village, seed 5: five houses sit above what their spring can
/// serve, and the design says so instead of hiding it
/// (water_test_village_5).
#[test]
fn example_village_seed_5_reports_the_houses_it_cannot_serve() {
    let answer = invoke("example", json!({"at": [29.85, -2.60], "seed": 5}));
    let (network, design) = (&answer["network"], &answer["design"]);
    assert_eq!(
        (count(network, "junctions"), count(network, "pipes")),
        (220, 219)
    );
    assert!((length_m(network) - 7558.6).abs() < 0.1);
    assert_eq!(design["ok"], false);
    close(&design["total_cost"], 82845.34, 0.01, "cost");
    assert_eq!(design["min_pressure"]["id"], "S4");
    close(
        &design["min_pressure"]["pressure_m"],
        -2.44,
        0.01,
        "lowest pressure",
    );
    assert_eq!(
        (
            design["infeasible"].clone(),
            design["static_deficit"].clone()
        ),
        (json!(5), json!(5))
    );
    assert_eq!(
        design["warnings"],
        json!([
            "5 junctions sit too high for their source or tank (static pressure below the minimum)"
        ])
    );
}

/// Gisovu hill villages on the fixture's ground: 250 m of fall below the
/// spring, zoned with break-pressure devices, every point reached.
#[test]
fn gisovu_hill_villages_on_its_ground_is_the_reference_scheme() {
    let features: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("features.geojson")).unwrap())
            .unwrap();
    let surface = crate::surface::read(&fixture("dem.asc")).unwrap();
    let answer = invoke("scheme", json!({"features": features, "surface": surface}));
    let (network, design) = (&answer["network"], &answer["design_report"]);
    assert_eq!(
        (
            count(network, "junctions"),
            count(network, "pipes"),
            count(network, "valves")
        ),
        (270, 265, 5)
    );
    assert!((length_m(network) - 10607.3).abs() < 0.1);
    assert_eq!(answer["layout"]["paths"], 112);
    assert_eq!(answer["layout"]["unserved"], json!([]));
    let elevations: Vec<f64> = network["junctions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|junction| junction["elevation_m"].as_f64().unwrap())
        .collect();
    assert!(
        elevations.iter().all(|z| (1662.0..=1845.0).contains(z)),
        "every node on the ground"
    );
    close(&design["total_cost"], 118616.44, 0.01, "cost");
    assert_eq!(design["min_pressure"]["id"], "J187");
    close(
        &design["min_pressure"]["pressure_m"],
        11.73,
        0.01,
        "lowest pressure",
    );
    assert_eq!(design["warnings"], json!([]));
    assert_eq!(
        (count(&answer, "sheets"), count(&answer, "profiles")),
        (11, 3)
    );
}
