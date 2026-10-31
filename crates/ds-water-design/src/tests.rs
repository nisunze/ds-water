use ds_water_model::*;

use super::*;

/// Source, then a chain of junctions 500 m apart, 1 L/s each, with a branch.
fn scheme(source_head: f64, nodes: usize) -> WaterNetwork {
    let mut n = WaterNetwork::new("scheme");
    n.reservoirs.push(Reservoir {
        id: "R".into(),
        head_m: source_head,
        ..Reservoir::default()
    });
    let mut prev = "R".to_owned();
    for i in 0..nodes {
        let id = format!("J{i}");
        n.junctions.push(Junction {
            id: id.clone(),
            elevation_m: 0.0,
            demand_lps: 1.0,
            ..Junction::default()
        });
        n.pipes.push(Pipe {
            id: format!("P{i}"),
            from: prev,
            to: id.clone(),
            length_m: 500.0,
            diameter_mm: 20.0,
            roughness: 150.0,
            ..Pipe::default()
        });
        prev = id;
    }
    n.junctions.push(Junction {
        id: "B".into(),
        elevation_m: 5.0,
        demand_lps: 2.0,
        ..Junction::default()
    });
    n.pipes.push(Pipe {
        id: "PB".into(),
        from: "J1".into(),
        to: "B".into(),
        length_m: 800.0,
        diameter_mm: 20.0,
        roughness: 150.0,
        ..Pipe::default()
    });
    n
}

#[test]
fn sizes_a_branched_gravity_scheme_to_criteria() {
    let net = scheme(60.0, 6);
    let r = design(&net, &Catalogue::standard(), &DesignCriteria::default()).unwrap();
    assert!(r.infeasible.is_empty(), "{:?}", r.infeasible);
    let mut h = ds_water_hydraulics::Hydraulics::new(&r.network).unwrap();
    let s = h.solve(0, &[]);
    for j in 0..r.network.junctions.len() {
        assert!(
            s.pressure_m[j] >= 10.0 - 1e-6,
            "{} at {}",
            r.network.junctions[j].id,
            s.pressure_m[j]
        );
    }
    for p in &r.pipes {
        assert!(p.velocity_ms <= 2.0 + 1e-9, "{} at {}", p.id, p.velocity_ms);
    }
    // Trunk bores never grow downstream in a branched gravity main.
    let trunk: Vec<f64> = (0..6).map(|i| r.pipes[i].internal_mm).collect();
    assert!(trunk.windows(2).all(|w| w[0] >= w[1] - 1e-9), "{trunk:?}");
    assert!(r.total_cost > 0.0);
    let boq_length: f64 = r.boq.iter().map(|b| b.length_m).sum();
    assert!((boq_length - 3800.0).abs() < 1e-6);
}

#[test]
fn pressure_class_covers_static_head_and_surge() {
    let net = scheme(150.0, 3);
    let criteria = DesignCriteria {
        break_pressure_tanks: false,
        ..DesignCriteria::default()
    };
    let r = design(&net, &Catalogue::standard(), &criteria).unwrap();
    for p in &r.pipes {
        assert!(
            p.pn_bar * 10.197 + 1e-6 >= p.design_head_m,
            "{} PN{} for {} m",
            p.id,
            p.pn_bar,
            p.design_head_m
        );
    }
    // 150 m static × 1.2 = 180 m: PN16 is not enough.
    assert!(r.pipes.iter().all(|p| p.pn_bar >= 20.0));
    assert_eq!(r.over_pressure.len(), 4);
}

#[test]
fn an_unreachable_pressure_is_reported_not_hidden() {
    let net = scheme(8.0, 2);
    let r = design(&net, &Catalogue::standard(), &DesignCriteria::default()).unwrap();
    assert!(!r.infeasible.is_empty());
    assert!(r.infeasible.iter().all(|n| n.pressure_m < 10.0));
}

#[test]
fn fixed_pipes_keep_their_size_and_cost_nothing() {
    let mut net = scheme(60.0, 3);
    net.pipes[0].diameter_mm = 150.0;
    let criteria = DesignCriteria {
        fixed_pipes: vec!["P0".into()],
        ..DesignCriteria::default()
    };
    let r = design(&net, &Catalogue::standard(), &criteria).unwrap();
    assert_eq!(r.pipes[0].internal_mm, 150.0);
    assert_eq!(r.pipes[0].cost, 0.0);
    assert!(r.pipes[0].fixed);
}

#[test]
fn rated_items_form_a_cost_frontier() {
    let cat = Catalogue::standard();
    let items = cat.rated(&[], 100.0);
    assert!(items.iter().all(|i| i.pn_bar >= 100.0 / 10.197));
    assert!(
        items
            .windows(2)
            .all(|w| w[0].internal_mm < w[1].internal_mm && w[0].cost_per_m < w[1].cost_per_m)
    );
    let pe_only = cat.rated(&["HDPE PE100".into()], 0.0);
    assert!(pe_only.iter().all(|i| i.material == "HDPE PE100"));
}

/// A spring 400 m above a valley village, 6 km of main falling steadily.
fn steep_main() -> WaterNetwork {
    let mut n = WaterNetwork::new("steep");
    n.reservoirs.push(Reservoir {
        id: "Spring".into(),
        head_m: 2000.0,
        x: Some(0.0),
        y: Some(0.0),
        ..Reservoir::default()
    });
    let mut prev = "Spring".to_owned();
    for i in 1..=6 {
        let id = format!("J{i}");
        n.junctions.push(Junction {
            id: id.clone(),
            elevation_m: 2000.0 - 66.0 * i as f64,
            demand_lps: if i == 6 { 3.0 } else { 0.0 },
            x: Some(i as f64 * 1000.0),
            y: Some(0.0),
            ..Junction::default()
        });
        n.pipes.push(Pipe {
            id: format!("P{i}"),
            from: prev,
            to: id.clone(),
            length_m: 1000.0,
            diameter_mm: 50.0,
            roughness: 150.0,
            vertices: vec![[i as f64 * 1000.0 - 500.0, 10.0]],
            ..Pipe::default()
        });
        prev = id;
    }
    n
}

#[test]
fn break_pressure_tanks_keep_static_pressure_under_the_limit() {
    let criteria = DesignCriteria {
        zone_device: ZoneDevice::BreakPressureTank,
        ..DesignCriteria::default()
    };
    let r = design(&steep_main(), &Catalogue::standard(), &criteria).unwrap();
    // 396 m of fall with an 80 m limit at 95 %: five tanks.
    assert_eq!(
        r.break_pressure_tanks.len(),
        5,
        "{:?}",
        r.break_pressure_tanks
    );
    assert!(r.over_pressure.is_empty(), "{:?}", r.over_pressure);
    assert!(r.infeasible.is_empty(), "{:?}", r.infeasible);
    let mut still = r.network.clone();
    still.options.demand_multiplier = 0.0;
    let s = ds_water_hydraulics::Hydraulics::new(&still)
        .unwrap()
        .solve(0, &[]);
    assert!(s.converged);
    for (j, junction) in r.network.junctions.iter().enumerate() {
        assert!(
            s.pressure_m[j] <= 80.0 + 1e-6,
            "{} static {}",
            junction.id,
            s.pressure_m[j]
        );
    }
    // Tanks lower the classes: nothing above PN10 is needed for 80 m + 20 %.
    assert!(
        r.pipes.iter().all(|p| p.pn_bar <= 10.0 + 1e-9),
        "{:?}",
        r.pipes.iter().map(|p| p.pn_bar).collect::<Vec<_>>()
    );
    // Every split pipe keeps its total length.
    let total: f64 = r.network.pipes.iter().map(|p| p.length_m).sum();
    assert!((total - 6000.0).abs() < 1e-6);
    // The path still runs source to tip through the tanks.
    let valves = r
        .network
        .valves
        .iter()
        .filter(|v| v.id.starts_with("BPT"))
        .count();
    assert_eq!(valves, 5);
}

#[test]
fn prv_zones_keep_consumers_below_them_in_pressure() {
    let r = design(
        &steep_main(),
        &Catalogue::standard(),
        &DesignCriteria::default(),
    )
    .unwrap();
    assert!(!r.break_pressure_tanks.is_empty());
    assert!(
        r.break_pressure_tanks
            .iter()
            .all(|t| t.id.starts_with("PRV"))
    );
    assert!(r.over_pressure.is_empty() && r.infeasible.is_empty() && r.static_deficit.is_empty());
}
