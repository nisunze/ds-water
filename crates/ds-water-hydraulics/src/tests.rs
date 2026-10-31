use ds_water_model::*;

use crate::{Hydraulics, RunStatus};

fn junction(id: &str, elevation_m: f64, demand_lps: f64) -> Junction {
    Junction {
        id: id.into(),
        elevation_m,
        demand_lps,
        ..Junction::default()
    }
}

fn pipe(id: &str, from: &str, to: &str, length_m: f64, diameter_mm: f64) -> Pipe {
    Pipe {
        id: id.into(),
        from: from.into(),
        to: to.into(),
        length_m,
        diameter_mm,
        roughness: 130.0,
        ..Pipe::default()
    }
}

fn reservoir(id: &str, head_m: f64) -> Reservoir {
    Reservoir {
        id: id.into(),
        head_m,
        ..Reservoir::default()
    }
}

fn hw_loss(q_lps: f64, length_m: f64, d_mm: f64, c: f64) -> f64 {
    10.67 * length_m * (q_lps / 1000.0).powf(1.852) / (c.powf(1.852) * (d_mm / 1000.0).powf(4.871))
}

fn solve(n: &WaterNetwork) -> crate::Snapshot {
    let mut h = Hydraulics::new(n).expect("valid network");
    let s = h.solve(0, &Hydraulics::initial_tank_levels(n));
    assert!(s.converged, "{:?}", s.warnings);
    s
}

/// Net outflow at every junction equals its delivered demand.
fn assert_mass_balance(n: &WaterNetwork, s: &crate::Snapshot) {
    let ids: Vec<&str> = n.node_refs().map(|(id, _)| id).collect();
    let mut net = vec![0.0; ids.len()];
    let ends = n
        .pipes
        .iter()
        .map(|p| (&p.from, &p.to))
        .chain(n.pumps.iter().map(|p| (&p.from, &p.to)))
        .chain(n.valves.iter().map(|v| (&v.from, &v.to)));
    for (k, (a, b)) in ends.enumerate() {
        let ia = ids.iter().position(|x| x == a).unwrap();
        let ib = ids.iter().position(|x| x == b).unwrap();
        net[ia] -= s.flow_lps[k];
        net[ib] += s.flow_lps[k];
    }
    for j in 0..n.junctions.len() {
        assert!(
            (net[j] - s.delivered_lps[j]).abs() < 1e-3 * (1.0 + s.delivered_lps[j]),
            "junction {j}: {} vs {}",
            net[j],
            s.delivered_lps[j]
        );
    }
}

#[test]
fn single_pipe_matches_hazen_williams() {
    let mut n = WaterNetwork::new("one pipe");
    n.reservoirs.push(reservoir("R", 100.0));
    n.junctions.push(junction("J", 50.0, 20.0));
    n.pipes.push(pipe("P", "R", "J", 1000.0, 150.0));
    let s = solve(&n);
    let expected = 100.0 - hw_loss(20.0, 1000.0, 150.0, 130.0);
    assert!(
        (s.head_m[0] - expected).abs() < 1e-3,
        "{} vs {expected}",
        s.head_m[0]
    );
    assert!((s.flow_lps[0] - 20.0).abs() < 1e-6);
    assert!((s.pressure_m[0] - (expected - 50.0)).abs() < 1e-3);
}

#[test]
fn single_pipe_matches_darcy_weisbach() {
    let mut n = WaterNetwork::new("one pipe dw");
    n.options.headloss = HeadlossFormula::DarcyWeisbach;
    n.reservoirs.push(reservoir("R", 100.0));
    n.junctions.push(junction("J", 50.0, 20.0));
    let mut p = pipe("P", "R", "J", 1000.0, 150.0);
    p.roughness = 0.0015 * 1000.0 / 1000.0; // 0.0015 mm, smooth PE
    n.pipes.push(p);
    let s = solve(&n);
    let (q, d, nu) = (0.020_f64, 0.150_f64, 1.004e-6);
    let a = std::f64::consts::PI * d * d / 4.0;
    let v = q / a;
    let re = v * d / nu;
    let f = 0.25
        / (0.0015e-3 / (3.7 * d) + 5.74 / re.powf(0.9))
            .log10()
            .powi(2);
    let h = f * 1000.0 / d * v * v / (2.0 * 9.806_65);
    assert!(
        (100.0 - s.head_m[0] - h).abs() < 1e-3,
        "{} vs {h}",
        100.0 - s.head_m[0]
    );
}

#[test]
fn parallel_pipes_share_flow_by_conductance() {
    let mut n = WaterNetwork::new("parallel");
    n.reservoirs.push(reservoir("R", 100.0));
    n.junctions.push(junction("J", 0.0, 50.0));
    n.pipes.push(pipe("A", "R", "J", 1000.0, 200.0));
    n.pipes.push(pipe("B", "R", "J", 1000.0, 100.0));
    let s = solve(&n);
    // Equal head loss: q ∝ D^(4.871/1.852).
    let ratio = (200.0_f64 / 100.0).powf(4.871 / 1.852);
    assert!((s.flow_lps[0] / s.flow_lps[1] - ratio).abs() < 1e-3);
    assert!((s.flow_lps[0] + s.flow_lps[1] - 50.0).abs() < 1e-6);
    assert_mass_balance(&n, &s);
}

#[test]
fn pump_lifts_to_a_higher_tank() {
    let mut n = WaterNetwork::new("pumped");
    n.reservoirs.push(reservoir("Well", 10.0));
    n.junctions.push(junction("Suction", 10.0, 0.0));
    n.reservoirs.push(reservoir("Hill", 60.0));
    n.curves.push(Curve {
        id: "C".into(),
        points: vec![[30.0, 60.0]],
    });
    n.pipes.push(pipe("S", "Well", "Suction", 10.0, 300.0));
    n.pumps.push(Pump {
        id: "PU".into(),
        from: "Suction".into(),
        to: "Rise".into(),
        curve: "C".into(),
        speed: 1.0,
        ..Pump::default()
    });
    n.junctions.push(junction("Rise", 10.0, 0.0));
    n.pipes.push(pipe("M", "Rise", "Hill", 2000.0, 200.0));
    let s = solve(&n);
    let q = s.flow_lps[2];
    // Pump gain at q equals static lift plus main and suction losses.
    let gain = 80.0 - 20.0 * (q / 30.0).powi(2);
    let losses = hw_loss(q, 2000.0, 200.0, 130.0) + hw_loss(q, 10.0, 300.0, 130.0);
    assert!(q > 0.0 && q < 60.0, "q = {q}");
    assert!(
        (gain - 50.0 - losses).abs() < 1e-2,
        "gain {gain} lift+losses {}",
        50.0 + losses
    );
    assert_eq!(s.status[2], RunStatus::Open);
}

#[test]
fn pump_against_too_much_head_closes() {
    let mut n = WaterNetwork::new("weak pump");
    n.reservoirs.push(reservoir("Low", 0.0));
    n.reservoirs.push(reservoir("High", 500.0));
    n.junctions.push(junction("A", 0.0, 0.0));
    n.junctions.push(junction("B", 0.0, 0.0));
    n.curves.push(Curve {
        id: "C".into(),
        points: vec![[10.0, 30.0]],
    });
    n.pipes.push(pipe("P1", "Low", "A", 10.0, 200.0));
    n.pumps.push(Pump {
        id: "PU".into(),
        from: "A".into(),
        to: "B".into(),
        curve: "C".into(),
        speed: 1.0,
        ..Pump::default()
    });
    n.pipes.push(pipe("P2", "B", "High", 10.0, 200.0));
    let s = solve(&n);
    assert_eq!(s.status[2], RunStatus::TempClosed);
    assert!(s.flow_lps[2].abs() < 1e-9);
}

#[test]
fn prv_holds_downstream_pressure() {
    let mut n = WaterNetwork::new("prv");
    n.reservoirs.push(reservoir("Spring", 300.0));
    n.junctions.push(junction("Up", 100.0, 0.0));
    n.junctions.push(junction("Down", 100.0, 0.0));
    n.junctions.push(junction("Village", 90.0, 5.0));
    n.pipes.push(pipe("P1", "Spring", "Up", 500.0, 150.0));
    n.valves.push(Valve {
        id: "V".into(),
        from: "Up".into(),
        to: "Down".into(),
        diameter_mm: 150.0,
        kind: ValveKind::Prv,
        setting: 40.0,
        ..Valve::default()
    });
    n.pipes.push(pipe("P2", "Down", "Village", 300.0, 100.0));
    let s = solve(&n);
    assert_eq!(s.status[2], RunStatus::Active);
    assert!(
        (s.pressure_m[1] - 40.0).abs() < 1e-3,
        "downstream pressure {}",
        s.pressure_m[1]
    );
    assert!(
        (s.flow_lps[2] - 5.0).abs() < 1e-4,
        "valve flow {}",
        s.flow_lps[2]
    );
    assert_mass_balance(&n, &s);
}

#[test]
fn prv_opens_when_upstream_is_too_low() {
    let mut n = WaterNetwork::new("prv open");
    n.reservoirs.push(reservoir("Spring", 120.0));
    n.junctions.push(junction("Up", 100.0, 0.0));
    n.junctions.push(junction("Down", 100.0, 2.0));
    n.pipes.push(pipe("P1", "Spring", "Up", 500.0, 150.0));
    n.valves.push(Valve {
        id: "V".into(),
        from: "Up".into(),
        to: "Down".into(),
        diameter_mm: 150.0,
        kind: ValveKind::Prv,
        setting: 40.0,
        ..Valve::default()
    });
    let s = solve(&n);
    assert_eq!(s.status[1], RunStatus::Open);
    assert!(s.pressure_m[1] < 20.0);
}

#[test]
fn check_valve_stops_reverse_flow() {
    let mut n = WaterNetwork::new("cv");
    n.reservoirs.push(reservoir("Low", 50.0));
    n.reservoirs.push(reservoir("High", 80.0));
    n.junctions.push(junction("J", 0.0, 10.0));
    let mut cv = pipe("CV", "Low", "J", 100.0, 150.0);
    cv.status = LinkStatus::CheckValve;
    n.pipes.push(cv);
    n.pipes.push(pipe("P", "High", "J", 100.0, 150.0));
    let s = solve(&n);
    // High drives J above Low, so the check valve shuts.
    assert_eq!(s.status[0], RunStatus::TempClosed);
    assert!((s.flow_lps[1] - 10.0).abs() < 1e-6);
}

#[test]
fn a_zone_behind_a_closed_pipe_is_reported_not_fatal() {
    let mut n = WaterNetwork::new("isolated");
    n.reservoirs.push(reservoir("R", 100.0));
    n.junctions.push(junction("A", 0.0, 1.0));
    n.junctions.push(junction("B", 0.0, 1.0));
    n.junctions.push(junction("C", 0.0, 1.0));
    n.pipes.push(pipe("P1", "R", "A", 100.0, 100.0));
    let mut closed = pipe("P2", "A", "B", 100.0, 100.0);
    closed.status = LinkStatus::Closed;
    n.pipes.push(closed);
    n.pipes.push(pipe("P3", "B", "C", 100.0, 100.0));
    let s = solve(&n);
    assert_eq!(s.disconnected, vec![1, 2]);
    assert_eq!(s.delivered_lps[1], 0.0);
    assert!((s.flow_lps[0] - 1.0).abs() < 1e-6);
}

#[test]
fn looped_grid_with_two_sources_balances() {
    let side = 12;
    let mut n = WaterNetwork::new("grid");
    for r in 0..side {
        for c in 0..side {
            n.junctions
                .push(junction(&format!("J{r}_{c}"), (r + c) as f64, 0.5));
        }
    }
    n.reservoirs.push(reservoir("North", 90.0));
    n.reservoirs.push(reservoir("South", 85.0));
    let mut k = 0;
    let mut add = |n: &mut WaterNetwork, a: String, b: String, d: f64| {
        k += 1;
        n.pipes.push(pipe(&format!("P{k}"), &a, &b, 150.0, d));
    };
    for r in 0..side {
        for c in 0..side {
            if c + 1 < side {
                add(
                    &mut n,
                    format!("J{r}_{c}"),
                    format!("J{r}_{}", c + 1),
                    100.0,
                );
            }
            if r + 1 < side {
                add(
                    &mut n,
                    format!("J{r}_{c}"),
                    format!("J{}_{c}", r + 1),
                    150.0,
                );
            }
        }
    }
    add(&mut n, "North".into(), "J0_0".into(), 300.0);
    add(
        &mut n,
        "South".into(),
        format!("J{}_{}", side - 1, side - 1),
        300.0,
    );
    let s = solve(&n);
    assert_mass_balance(&n, &s);
    let total: f64 = s.delivered_lps.iter().sum();
    let supplied = s.flow_lps[s.flow_lps.len() - 2] + s.flow_lps[s.flow_lps.len() - 1];
    assert!((total - supplied).abs() < 1e-3, "{total} vs {supplied}");
    assert!(s.iterations < 20, "{} iterations", s.iterations);
}

#[test]
fn pressure_driven_delivery_follows_wagner() {
    let mut n = WaterNetwork::new("pda");
    n.options.demand_model = DemandModel::PressureDriven;
    n.options.required_pressure_m = 20.0;
    n.options.minimum_pressure_m = 0.0;
    n.reservoirs.push(reservoir("R", 30.0));
    n.junctions.push(junction("J", 0.0, 30.0));
    n.pipes.push(pipe("P", "R", "J", 2000.0, 100.0));
    let s = solve(&n);
    let d = s.delivered_lps[0];
    let p = s.pressure_m[0];
    assert!(d > 0.0 && d < 30.0, "delivered {d}");
    assert!((d - 30.0 * (p / 20.0).sqrt()).abs() < 0.05, "d {d} p {p}");
    assert!((30.0 - hw_loss(d, 2000.0, 100.0, 130.0) - s.head_m[0]).abs() < 0.05);
    // Demand-driven on the same network demands more than the pipe can give.
    n.options.demand_model = DemandModel::DemandDriven;
    let dda = solve(&n);
    assert!(dda.pressure_m[0] < 0.0);
}

#[test]
fn a_tank_drains_over_an_extended_period() {
    let mut n = WaterNetwork::new("eps");
    n.options.duration_s = 6 * 3600;
    n.tanks.push(Tank {
        id: "T".into(),
        elevation_m: 50.0,
        initial_level_m: 4.0,
        min_level_m: 0.5,
        max_level_m: 5.0,
        diameter_m: 10.0,
        ..Tank::default()
    });
    n.junctions.push(junction("J", 0.0, 2.0));
    n.pipes.push(pipe("P", "T", "J", 100.0, 150.0));
    let mut h = Hydraulics::new(&n).unwrap();
    let run = h.run(&n);
    assert_eq!(run.len(), 7);
    let area = std::f64::consts::PI * 25.0;
    let drop = 0.002 * 3600.0 / area;
    for w in run.windows(2) {
        let dh = w[0].head_m[1] - w[1].head_m[1];
        assert!((dh - drop).abs() < 1e-6, "level drop {dh} vs {drop}");
    }
}

#[test]
fn invalid_networks_refuse_to_compile() {
    let mut n = WaterNetwork::new("bad");
    n.junctions.push(junction("J", 0.0, 1.0));
    assert!(Hydraulics::new(&n).is_err());
}
