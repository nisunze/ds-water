use super::*;

fn network() -> WaterNetwork {
    serde_json::from_value(serde_json::json!({
        "name": "Two houses",
        "crs": "EPSG:4326",
        "reservoirs": [{"id": "R1", "head_m": 1520.0, "x": 29.6, "y": -2.1}],
        "junctions": [
            {"id": "J1", "elevation_m": 1490.0, "demand_lps": 0.2, "x": 29.601, "y": -2.1},
            {"id": "J2", "elevation_m": 1485.5, "demand_lps": 0.1, "x": 29.602, "y": -2.101}
        ],
        "pipes": [
            {"id": "P1", "from": "R1", "to": "J1", "length_m": 110.0, "diameter_mm": 63.0, "roughness": 140.0},
            {"id": "P2", "from": "J1", "to": "J2", "length_m": 140.0, "diameter_mm": 50.0, "roughness": 140.0}
        ]
    }))
    .unwrap()
}

fn snapshot() -> WaterNetworkSnapshot {
    WaterNetworkSnapshot {
        source_project_id: Some("arjgpydw_survey_test".into()),
        model_id: "water_two_houses".into(),
        source_revision: 3,
        network: network(),
    }
}

#[test]
fn a_network_round_trips_through_its_dsgrid_package() {
    let (bytes, packed) = pack_network(&snapshot()).unwrap();
    assert_eq!(unpack_network(&bytes).unwrap(), snapshot());
    assert_eq!(
        packed.manifest_model_id,
        manifest_model_id("water_two_houses")
    );
    assert_eq!(packed.manifest_model_revision, 3);
    assert_eq!(packed.byte_length, bytes.len() as u64);
    assert_eq!(packed.digest, sha256_hex(&bytes));
}

#[test]
fn the_same_network_packs_to_the_same_bytes() {
    let (first, _) = pack_network(&snapshot()).unwrap();
    let (second, _) = pack_network(&snapshot()).unwrap();
    assert_eq!(first, second);
    let mut moved = snapshot();
    moved.network.junctions[1].elevation_m = 1486.0;
    assert_ne!(pack_network(&moved).unwrap().0, first);
}

#[test]
fn identity_is_refused_before_packing() {
    let mut zero = snapshot();
    zero.source_revision = 0;
    assert!(matches!(
        pack_network(&zero),
        Err(WaterPackageError::Invalid(_))
    ));
    let mut blank = snapshot();
    blank.model_id = " water_x".into();
    assert!(matches!(
        pack_network(&blank),
        Err(WaterPackageError::Invalid(_))
    ));
}

#[test]
fn another_profile_or_plain_bytes_never_unpack_as_water() {
    let lv = ds_grid_exchange::lv_transformer::pack_lv_transformer_layers(
        &ds_grid_exchange::lv_transformer::LvTransformerLayers {
            source_project_id: None,
            transformer_id: "tr-1".into(),
            source_revision: 1,
            document: serde_json::json!({"name": "tr-1"}),
            layers: BTreeMap::new(),
        },
    )
    .unwrap();
    assert!(unpack_network(&lv).is_err());
    assert!(unpack_network(b"not a package").is_err());
}
