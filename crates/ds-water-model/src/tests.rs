use super::*;

const SMALL_GPM: &str = "\
[TITLE]
Two loops, US units
[JUNCTIONS]
;ID Elev Demand Pattern
 J1  700  100  P1
 J2  710  50
[RESERVOIRS]
 R1  800
[TANKS]
 T1  850 10 0 20 50
[PIPES]
 P1 R1 J1 1000 12 100 0 Open
 P2 J1 J2 500 8 100 0 CV
 P3 J2 T1 800 8 100
[PUMPS]
 PU1 J1 T1 HEAD C1
[VALVES]
 V1 J2 J1 6 PRV 50
[PATTERNS]
 P1 1.0 1.2
 P1 0.8
[CURVES]
 C1 500 150
[STATUS]
 PU1 Closed
[OPTIONS]
 Units GPM
 Headloss H-W
[TIMES]
 Duration 24:00
 Hydraulic Timestep 1:00
[COORDINATES]
 J1 1 2
 R1 0 0
[VERTICES]
 P1 0.5 1
[REACTIONS]
 Global Bulk -0.5
[END]
";

#[test]
fn inp_reads_us_units_into_si() {
    let import = inp::read(SMALL_GPM).expect("valid INP");
    let n = &import.network;
    assert_eq!(n.name, "Two loops, US units");
    assert_eq!(n.junctions.len(), 2);
    assert!((n.junctions[0].elevation_m - 213.36).abs() < 1e-9);
    assert!((n.junctions[0].demand_lps - 6.309_019_64).abs() < 1e-6);
    assert!((n.pipes[0].length_m - 304.8).abs() < 1e-9);
    assert!((n.pipes[0].diameter_mm - 304.8).abs() < 1e-9);
    assert_eq!(n.pipes[1].status, LinkStatus::CheckValve);
    assert_eq!(n.pumps[0].status, LinkStatus::Closed);
    assert_eq!(n.valves[0].kind, ValveKind::Prv);
    // 50 psi is 35.15 m of water.
    assert!((n.valves[0].setting - 35.153).abs() < 1e-2);
    assert_eq!(n.patterns[0].multipliers, vec![1.0, 1.2, 0.8]);
    assert_eq!(n.options.duration_s, 86_400);
    assert_eq!(n.junctions[0].x, Some(1.0));
    assert_eq!(n.pipes[0].vertices, vec![[0.5, 1.0]]);
    assert_eq!(import.skipped_sections, vec!["REACTIONS".to_owned()]);
    assert!(validate(n).is_empty(), "{:?}", validate(n));
}

#[test]
fn inp_round_trips_through_si() {
    let first = inp::read(SMALL_GPM).unwrap().network;
    let text = inp::write(&first);
    let second = inp::read(&text).unwrap().network;
    assert_eq!(first.junctions.len(), second.junctions.len());
    for (a, b) in first.junctions.iter().zip(&second.junctions) {
        assert_eq!(a.id, b.id);
        assert!((a.elevation_m - b.elevation_m).abs() < 1e-9);
        assert!((a.demand_lps - b.demand_lps).abs() < 1e-9);
    }
    for (a, b) in first.pipes.iter().zip(&second.pipes) {
        assert_eq!(a.status, b.status);
        assert!((a.diameter_mm - b.diameter_mm).abs() < 1e-9);
    }
    assert_eq!(first.valves[0].setting, second.valves[0].setting);
    assert_eq!(first.pumps[0].status, second.pumps[0].status);
    assert_eq!(first.options.duration_s, second.options.duration_s);
}

#[test]
fn inp_refuses_unknown_units_and_bad_numbers() {
    assert_eq!(
        inp::read("[OPTIONS]\nUnits FURLONGS\n").unwrap_err(),
        inp::InpError::Units("FURLONGS".into())
    );
    let err = inp::read("[OPTIONS]\nUnits LPS\n[JUNCTIONS]\nJ1 abc\n").unwrap_err();
    assert!(
        matches!(err, inp::InpError::Line { line: 4, .. }),
        "{err:?}"
    );
}

#[test]
fn validation_reports_every_issue() {
    let mut n = WaterNetwork::new("broken");
    n.junctions.push(Junction {
        id: "A".into(),
        elevation_m: 1.0,
        ..Junction::default()
    });
    n.junctions.push(Junction {
        id: "A".into(),
        elevation_m: f64::NAN,
        ..Junction::default()
    });
    n.pipes.push(Pipe {
        id: "P".into(),
        from: "A".into(),
        to: "Z".into(),
        length_m: 0.0,
        diameter_mm: 100.0,
        roughness: 130.0,
        ..Pipe::default()
    });
    let codes: Vec<String> = validate(&n).into_iter().map(|i| i.code).collect();
    for expected in [
        "node_duplicate_id",
        "network_without_source",
        "not_finite",
        "link_unknown_node",
        "pipe_length",
    ] {
        assert!(
            codes.iter().any(|c| c == expected),
            "missing {expected} in {codes:?}"
        );
    }
}

#[test]
fn json_defaults_fill_options() {
    let n = WaterNetwork::from_json(r#"{"junctions":[{"id":"J","elevation_m":3}]}"#).unwrap();
    assert_eq!(n.schema, SCHEMA);
    assert_eq!(n.options.max_iterations, 200);
    assert_eq!(n.junctions[0].demand_lps, 0.0);
}
