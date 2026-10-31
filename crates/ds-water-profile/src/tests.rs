use ds_water_hydraulics::Hydraulics;
use ds_water_model::*;

use super::*;

/// Ground: 1500 m at the source, a summit of 1550 m at 1 km, falling to
/// 1400 m at 3 km.
fn ground(x: f64) -> f64 {
    if x <= 1000.0 {
        1500.0 + 50.0 * x / 1000.0
    } else {
        1550.0 - 150.0 * (x - 1000.0) / 2000.0
    }
}

fn main_over_a_hill(diameter_mm: f64, demand_lps: f64) -> WaterNetwork {
    let mut n = WaterNetwork::new("hill main");
    n.crs = Some("EPSG:32735".into());
    n.reservoirs.push(Reservoir {
        id: "Spring".into(),
        head_m: 1600.0,
        x: Some(0.0),
        y: Some(0.0),
        ..Reservoir::default()
    });
    n.junctions.push(Junction {
        id: "Mid".into(),
        elevation_m: ground(1500.0) - 1.1,
        x: Some(1500.0),
        y: Some(0.0),
        ..Junction::default()
    });
    n.junctions.push(Junction {
        id: "Village".into(),
        elevation_m: ground(3000.0) - 1.1,
        demand_lps,
        x: Some(3000.0),
        y: Some(0.0),
        ..Junction::default()
    });
    n.junctions.push(Junction {
        id: "Side".into(),
        elevation_m: ground(1500.0),
        demand_lps: 0.1,
        x: Some(1500.0),
        y: Some(300.0),
        ..Junction::default()
    });
    let pipe = |id: &str, from: &str, to: &str, length_m: f64| Pipe {
        id: id.into(),
        from: from.into(),
        to: to.into(),
        length_m,
        diameter_mm,
        roughness: 150.0,
        pressure_class_bar: Some(16.0),
        ..Pipe::default()
    };
    n.pipes.push(pipe("P1", "Spring", "Mid", 1500.0));
    n.pipes.push(pipe("P2", "Mid", "Village", 1500.0));
    n.pipes.push(pipe("P3", "Mid", "Side", 300.0));
    n
}

fn solves(n: &WaterNetwork) -> (Snapshot, Snapshot) {
    let peak = Hydraulics::new(n).unwrap().solve(0, &[]);
    let mut still_net = n.clone();
    still_net.options.demand_multiplier = 0.0;
    let still = Hydraulics::new(&still_net).unwrap().solve(0, &[]);
    (peak, still)
}

#[test]
fn a_gravity_main_over_a_hill_gets_its_fittings() {
    let n = main_over_a_hill(110.0, 2.0);
    let (peak, still) = solves(&n);
    let surface = |p: [f64; 2]| Some(ground(p[0]));
    let r = profile(
        &n,
        &["P1".into(), "P2".into()],
        &peak,
        &still,
        Some(&surface),
        &ProfileOptions::default(),
    )
    .unwrap();
    assert!((r.length_m - 3000.0).abs() < 1e-6);
    assert!(r.ground_source.contains("ds-terrain-sampling"));
    // One summit at 1 km: an air valve there.
    let air: Vec<&Fitting> = r
        .fittings
        .iter()
        .filter(|f| f.kind == FittingKind::AirValve)
        .collect();
    assert_eq!(air.len(), 1, "{:?}", r.fittings);
    assert!(
        (air[0].chainage_m - 1000.0).abs() < 25.0,
        "{}",
        air[0].chainage_m
    );
    assert!(r.fittings.iter().all(|f| f.kind != FittingKind::Washout));
    // 200 m of fall from the spring: two break-pressure tanks for 80 m.
    let bpt: Vec<&Fitting> = r
        .fittings
        .iter()
        .filter(|f| f.kind == FittingKind::BreakPressureTank)
        .collect();
    assert_eq!(bpt.len(), 2, "{bpt:?}");
    assert!(bpt[0].chainage_m < bpt[1].chainage_m);
    // A tee at Mid and a spacing valve at 1 km... and 2 km.
    assert!(
        r.fittings
            .iter()
            .any(|f| f.kind == FittingKind::IsolationValve && f.reason.contains("Mid"))
    );
    assert!(
        r.findings
            .iter()
            .any(|f| f.kind == FindingKind::OverStaticLimit)
    );
    // PN16 is 163 m: the lower main is over class until tanks are built.
    assert!(r.findings.iter().any(|f| f.kind == FindingKind::OverClass));
    // The HGL meets the node heads at the nodes.
    let end = r.points.last().unwrap();
    assert!((end.hgl_m - peak.head_m[1]).abs() < 1e-9);
    assert_eq!(
        r.nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
        ["Spring", "Mid", "Village"]
    );
    assert!(r.nodes[1].tee);
}

#[test]
fn an_undersized_main_shows_negative_pressure_at_the_summit() {
    let n = main_over_a_hill(50.0, 4.0);
    let (peak, still) = solves(&n);
    let surface = |p: [f64; 2]| Some(ground(p[0]));
    let r = profile(
        &n,
        &["P1".into(), "P2".into()],
        &peak,
        &still,
        Some(&surface),
        &ProfileOptions::default(),
    )
    .unwrap();
    let negative = r
        .findings
        .iter()
        .find(|f| f.kind == FindingKind::NegativePressure)
        .expect("negative pressure");
    assert!(
        negative.from_m <= 1000.0 && negative.to_m >= 1000.0,
        "{negative:?}"
    );
    assert!(negative.worst_m < 0.0);
}

#[test]
fn without_a_surface_ground_follows_the_nodes() {
    let n = main_over_a_hill(110.0, 2.0);
    let (peak, still) = solves(&n);
    let r = profile(
        &n,
        &["P2".into(), "P1".into()]
            .iter()
            .rev()
            .cloned()
            .collect::<Vec<_>>(),
        &peak,
        &still,
        None,
        &ProfileOptions::default(),
    )
    .unwrap();
    assert!(r.ground_source.contains("node elevations"));
    assert_eq!(r.points.len(), 3);
    assert!((r.points[1].ground_m - n.junctions[0].elevation_m).abs() < 1e-9);
}

#[test]
fn a_broken_path_is_refused() {
    let n = main_over_a_hill(110.0, 2.0);
    let (peak, still) = solves(&n);
    let err = profile(
        &n,
        &["P1".into(), "P3".into(), "P2".into()],
        &peak,
        &still,
        None,
        &ProfileOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, ProfileError::Discontinuous(..)), "{err}");
}
