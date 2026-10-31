//! Pressure zoning with break-pressure tanks.
//!
//! On a steep gravity scheme the static pressure grows with every metre the
//! pipe falls below its source. Before sizing, walk the network from its
//! sources in supply order; wherever a pipe would take static pressure past
//! the limit, split it where the static pressure reaches the target share of
//! the limit and put a break-pressure tank there. Downstream of a tank the
//! static line restarts at the tank's water level.
//!
//! Both devices are modelled the way EPANET modellers do: a pressure-reducing
//! valve holding its outlet pressure (a tank's water depth, or a PRV's
//! setting), between an inlet and an outlet junction at the same point. The network stays one connected
//! system, the static solve shows each zone at its own level, and the
//! profile and drawings mark it as a BPT (its id starts with `BPT`).
//!
//! Elevation along a pipe is interpolated between its end nodes, so the
//! placement is as good as the node density; the profile, which samples the
//! ground, flags any remaining over-limit stretch.

use std::collections::{HashMap, VecDeque};

use ds_water_model::{Junction, Pipe, Valve, ValveKind, WaterNetwork};
use serde::{Deserialize, Serialize};

/// What resets the static line at a zone boundary.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ZoneDevice {
    /// A PRV holding its outlet at a set pressure: consumers just below it
    /// keep their minimum pressure. The default for distribution.
    #[default]
    PressureReducingValve,
    /// An open tank: the line restarts at atmospheric pressure plus the
    /// water depth. Cheap and robust on transmission mains without
    /// consumers.
    BreakPressureTank,
}

impl ZoneDevice {
    fn prefix(self) -> &'static str {
        match self {
            ZoneDevice::PressureReducingValve => "PRV",
            ZoneDevice::BreakPressureTank => "BPT",
        }
    }
}

/// A zone boundary the design inserted (a break-pressure tank or a PRV).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BreakPressureTank {
    pub id: String,
    /// The pipe that was split to hold it.
    pub pipe: String,
    pub ground_m: f64,
    pub water_level_m: f64,
    pub at: Option<[f64; 2]>,
}

/// Insert break-pressure tanks so that no static pressure passes
/// `max_static_m`. Each tank is placed where static pressure reaches
/// `target` × the limit and holds its outlet at `depth_m` — or higher up the
/// pipe, when the pipe's far end draws water and would otherwise sit too
/// close below the tank to get `min_pressure_m`.
pub fn insert_break_pressure_tanks(
    net: &WaterNetwork,
    max_static_m: f64,
    target: f64,
    depth_m: f64,
    min_pressure_m: f64,
    device: ZoneDevice,
) -> (WaterNetwork, Vec<BreakPressureTank>) {
    let draws: std::collections::HashSet<&str> = net
        .junctions
        .iter()
        .filter(|j| j.demand_lps > 0.0)
        .map(|j| j.id.as_str())
        .collect();
    let mut out = net.clone();
    let mut tanks = Vec::new();
    let elevation: HashMap<String, f64> = net
        .node_refs()
        .map(|(id, r)| (id.to_owned(), net.node_elevation(r)))
        .collect();
    let mut level: HashMap<String, f64> = HashMap::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    for r in &net.reservoirs {
        level.insert(r.id.clone(), r.head_m);
        queue.push_back(r.id.clone());
    }
    for t in &net.tanks {
        level.insert(t.id.clone(), t.elevation_m + t.max_level_m);
        queue.push_back(t.id.clone());
    }
    // Adjacency over all links; only pipes are split. A PRV already in the
    // network resets the level to its own setting downstream.
    let mut adjacency: HashMap<String, Vec<(usize, bool)>> = HashMap::new();
    for (k, p) in net.pipes.iter().enumerate() {
        adjacency.entry(p.from.clone()).or_default().push((k, true));
        adjacency.entry(p.to.clone()).or_default().push((k, false));
    }
    let mut valve_out: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for v in &net.valves {
        let reset = match v.kind {
            ValveKind::Prv => Some(elevation[&v.to] + v.setting),
            _ => None,
        };
        valve_out
            .entry(v.from.clone())
            .or_default()
            .push((v.to.clone(), reset.unwrap_or(f64::NAN)));
    }
    let mut visited_pipe = vec![false; net.pipes.len()];
    let mut replaced: Vec<Option<Vec<Pipe>>> = vec![None; net.pipes.len()];
    let mut counter = 0;
    while let Some(node) = queue.pop_front() {
        let h = level[&node];
        for (to, reset) in valve_out.get(&node).cloned().unwrap_or_default() {
            if !level.contains_key(&to) {
                level.insert(to.clone(), if reset.is_nan() { h } else { reset.min(h) });
                queue.push_back(to);
            }
        }
        for &(k, forward) in adjacency.get(&node).map(Vec::as_slice).unwrap_or(&[]) {
            if visited_pipe[k] {
                continue;
            }
            visited_pipe[k] = true;
            let pipe = &net.pipes[k];
            let far = if forward { &pipe.to } else { &pipe.from };
            if level.contains_key(far) {
                continue; // a loop closing on a node already levelled
            }
            let (z0, z1) = (elevation[&node], elevation[far]);
            let mut h = h;
            let mut t0 = 0.0;
            // (t start, t end, start node, end node) in the supply direction.
            let mut pieces: Vec<(f64, f64, String, String)> = Vec::new();
            let mut piece_start = node.clone();
            // Split while the far end would be over the limit.
            while h - z1 > max_static_m && z1 < z0 {
                // Margin for friction between the tank and its first consumer.
                let floor = if draws.contains(far.as_str()) {
                    z1 + min_pressure_m + 5.0 - depth_m
                } else {
                    z1
                };
                let z_tank = (h - target * max_static_m).max(floor);
                let t = ((z0 - z_tank) / (z0 - z1)).clamp(t0, 1.0);
                if t >= 1.0 - 1e-9 {
                    break;
                }
                counter += 1;
                let id = format!("{}{counter}", device.prefix());
                let z = z0 + t * (z1 - z0);
                let at = point_along(&supply_line(net, pipe, forward), t);
                let inlet = format!("{id}-in");
                for jid in [&inlet, &id] {
                    out.junctions.push(Junction {
                        id: jid.clone(),
                        elevation_m: z,
                        x: at.map(|p| p[0]),
                        y: at.map(|p| p[1]),
                        ..Junction::default()
                    });
                }
                out.valves.push(Valve {
                    id: format!("{id}-valve"),
                    from: inlet.clone(),
                    to: id.clone(),
                    diameter_mm: pipe.diameter_mm,
                    kind: ValveKind::Prv,
                    setting: depth_m,
                    minor_loss: 0.0,
                    status: None,
                });
                tanks.push(BreakPressureTank {
                    id: id.clone(),
                    pipe: pipe.id.clone(),
                    ground_m: z,
                    water_level_m: z + depth_m,
                    at,
                });
                pieces.push((t0, t, piece_start, inlet));
                piece_start = id;
                t0 = t;
                h = z + depth_m;
            }
            if !pieces.is_empty() {
                pieces.push((t0, 1.0, piece_start, far.clone()));
                let line = supply_line(net, pipe, forward);
                let mut parts = Vec::new();
                for (i, (a, b, start, end)) in pieces.into_iter().enumerate() {
                    let mut interior = interior_between(&line, a, b);
                    let (from, to) = if forward {
                        (start, end)
                    } else {
                        interior.reverse();
                        (end, start)
                    };
                    parts.push(Pipe {
                        id: if i == 0 {
                            pipe.id.clone()
                        } else {
                            format!("{}.{}", pipe.id, i)
                        },
                        from,
                        to,
                        length_m: (pipe.length_m * (b - a)).max(0.1),
                        vertices: interior,
                        ..pipe.clone()
                    });
                }
                replaced[k] = Some(parts);
            }
            level.insert(far.clone(), h);
            queue.push_back(far.clone());
        }
    }
    let mut pipes = Vec::with_capacity(out.pipes.len() + tanks.len());
    for (k, pipe) in net.pipes.iter().enumerate() {
        match replaced[k].take() {
            Some(parts) => pipes.extend(parts),
            None => pipes.push(pipe.clone()),
        }
    }
    out.pipes = pipes;
    (out, tanks)
}

/// A pipe's full polyline (ends included) in the supply direction; empty
/// when its nodes have no coordinates.
fn supply_line(net: &WaterNetwork, pipe: &Pipe, forward: bool) -> Vec<[f64; 2]> {
    let xy = |id: &str| {
        net.node_refs()
            .find(|(n, _)| *n == id)
            .and_then(|(_, r)| net.node_xy(r))
    };
    let (Some(a), Some(b)) = (xy(&pipe.from), xy(&pipe.to)) else {
        return Vec::new();
    };
    let mut line = vec![a];
    line.extend(pipe.vertices.iter().copied());
    line.push(b);
    if !forward {
        line.reverse();
    }
    line
}

/// Cumulative fractions of length at each vertex of `line`.
fn fractions(line: &[[f64; 2]]) -> Vec<f64> {
    let mut acc = vec![0.0];
    for w in line.windows(2) {
        acc.push(acc.last().unwrap() + (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]));
    }
    let total = acc.last().copied().unwrap_or(0.0).max(1e-12);
    acc.iter().map(|a| a / total).collect()
}

fn point_along(line: &[[f64; 2]], t: f64) -> Option<[f64; 2]> {
    if line.len() < 2 {
        return None;
    }
    let f = fractions(line);
    let i = f.partition_point(|&x| x < t).clamp(1, line.len() - 1);
    let span = (f[i] - f[i - 1]).max(1e-12);
    let u = ((t - f[i - 1]) / span).clamp(0.0, 1.0);
    Some([
        line[i - 1][0] + u * (line[i][0] - line[i - 1][0]),
        line[i - 1][1] + u * (line[i][1] - line[i - 1][1]),
    ])
}

/// Interior vertices of `line` strictly between fractions `a` and `b`.
fn interior_between(line: &[[f64; 2]], a: f64, b: f64) -> Vec<[f64; 2]> {
    let f = fractions(line);
    line.iter()
        .zip(&f)
        .filter(|(_, x)| **x > a + 1e-9 && **x < b - 1e-9)
        .map(|(p, _)| *p)
        .collect()
}
