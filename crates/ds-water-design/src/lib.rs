//! Water engineering design: choose a catalogue pipe for every pipe so that
//! each junction gets its minimum pressure at peak demand, no pipe runs
//! faster than the velocity limit, and every pipe is rated for the static
//! head plus a surge allowance it will see — at the lowest cost the method
//! can find.
//!
//! The method, in order:
//! 1. Pressure class: solve with zero demand for static heads; each pipe's
//!    design head is its higher end's static pressure plus the surge
//!    allowance, which restricts it to catalogue items rated for it.
//! 2. Velocity: size every pipe to the smallest rated bore that keeps the
//!    peak-flow velocity under the limit (three passes, since loop flows
//!    move as sizes change).
//! 3. Pressure: while a junction is short of pressure, trace its supply path
//!    upstream (the largest inflow at each node) and upgrade the pipe on that
//!    path that buys the most head per unit of extra cost, until the
//!    estimated gain covers the deficit; then solve again. A junction that no
//!    upgrade can serve is reported as infeasible (it needs a higher source,
//!    a pump or a different route), not hidden.
//!
//! This is a cost-gradient heuristic in the family of Alperovits-Shamir and
//! the "lowest cost per head" upgrade rules; optimal LP or evolutionary
//! sizing can replace step 3 behind the same request and result.

pub mod catalogue;

use std::collections::HashMap;

use ds_water_hydraulics::{Hydraulics, Snapshot};
use ds_water_model::{HeadlossFormula, WaterNetwork};
use serde::{Deserialize, Serialize};

pub use catalogue::{Catalogue, CatalogueItem};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DesignCriteria {
    /// Minimum residual pressure at peak demand, m.
    pub min_pressure_m: f64,
    /// Static pressure above which a junction needs a PRV or break-pressure
    /// tank, m.
    pub max_static_pressure_m: f64,
    pub max_velocity_ms: f64,
    /// Below this at peak, a pipe is flagged for sediment (not resized).
    pub min_velocity_ms: f64,
    /// Fraction of static pressure added for surge when picking classes.
    pub surge_allowance: f64,
    pub min_internal_mm: f64,
    /// Allowed materials; empty allows the whole catalogue.
    pub materials: Vec<String>,
    /// Existing pipes the design must not resize.
    pub fixed_pipes: Vec<String>,
    /// Upper bound on hydraulic solves.
    pub max_simulations: u32,
}

impl Default for DesignCriteria {
    fn default() -> Self {
        Self {
            min_pressure_m: 10.0,
            max_static_pressure_m: 80.0,
            max_velocity_ms: 2.0,
            min_velocity_ms: 0.3,
            surge_allowance: 0.2,
            min_internal_mm: 20.0,
            materials: Vec::new(),
            fixed_pipes: Vec::new(),
            max_simulations: 400,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PipeDesign {
    pub id: String,
    pub material: String,
    pub nominal_mm: f64,
    pub internal_mm: f64,
    pub pn_bar: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sdr: Option<f64>,
    pub length_m: f64,
    pub flow_lps: f64,
    pub velocity_ms: f64,
    pub headloss_m_per_km: f64,
    /// Static pressure plus surge allowance the class was chosen for, m.
    pub design_head_m: f64,
    pub cost: f64,
    pub fixed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoqLine {
    pub material: String,
    pub nominal_mm: f64,
    pub pn_bar: f64,
    pub length_m: f64,
    pub cost: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodePressure {
    pub id: String,
    pub pressure_m: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DesignResult {
    pub network: WaterNetwork,
    pub pipes: Vec<PipeDesign>,
    pub boq: Vec<BoqLine>,
    pub total_cost: f64,
    pub min_pressure: Option<NodePressure>,
    /// Junctions still below the minimum pressure.
    pub infeasible: Vec<NodePressure>,
    /// Junctions whose static pressure exceeds the limit.
    pub over_pressure: Vec<NodePressure>,
    pub low_velocity_pipes: Vec<String>,
    pub simulations: u32,
    pub upgrades: u32,
    pub warnings: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DesignError {
    #[error("{0}")]
    Network(#[from] ds_water_hydraulics::PrepareError),
    #[error("no catalogue item fits pipe {0}")]
    NoItem(String),
}

struct Context {
    hazen: bool,
    simulations: u32,
    warnings: Vec<String>,
}

impl Context {
    fn solve(&mut self, net: &WaterNetwork) -> Result<Snapshot, DesignError> {
        self.simulations += 1;
        let mut h = Hydraulics::new(net)?;
        let s = h.solve(0, &Hydraulics::initial_tank_levels(net));
        if !s.converged {
            self.warnings
                .push(format!("solve {} did not converge", self.simulations));
        }
        Ok(s)
    }
}

/// Size every pipe of `network` from `catalogue` to `criteria`.
pub fn design(
    network: &WaterNetwork,
    catalogue: &Catalogue,
    criteria: &DesignCriteria,
) -> Result<DesignResult, DesignError> {
    let mut net = network.clone();
    let mut cx = Context {
        hazen: net.options.headloss == HeadlossFormula::HazenWilliams,
        simulations: 0,
        warnings: Vec::new(),
    };
    let nj = net.junctions.len();
    let fixed: Vec<bool> = net
        .pipes
        .iter()
        .map(|p| criteria.fixed_pipes.contains(&p.id))
        .collect();
    let index: HashMap<String, usize> = net
        .node_refs()
        .enumerate()
        .map(|(i, (id, _))| (id.to_owned(), i))
        .collect();
    let ends: Vec<(usize, usize)> = net
        .pipes
        .iter()
        .map(|p| (index[&p.from], index[&p.to]))
        .collect();

    // 1. Static pressure → pressure class → allowed items per pipe.
    let mut still = net.clone();
    still.options.demand_multiplier = 0.0;
    let static_snapshot = cx.solve(&still)?;
    let static_pressure = |node: usize| {
        if node < nj {
            static_snapshot.pressure_m[node].max(0.0)
        } else {
            0.0
        }
    };
    let mut options: Vec<Vec<CatalogueItem>> = Vec::with_capacity(net.pipes.len());
    let mut design_head = Vec::with_capacity(net.pipes.len());
    for (k, pipe) in net.pipes.iter().enumerate() {
        let head = static_pressure(ends[k].0).max(static_pressure(ends[k].1))
            * (1.0 + criteria.surge_allowance);
        let mut items: Vec<CatalogueItem> = catalogue
            .rated(&criteria.materials, head)
            .into_iter()
            .filter(|i| i.internal_mm >= criteria.min_internal_mm)
            .cloned()
            .collect();
        if items.is_empty() {
            cx.warnings.push(format!(
                "pipe {}: nothing in the catalogue is rated for {head:.0} m; using the highest class available",
                pipe.id
            ));
            let top = catalogue.items.iter().map(|i| i.pn_bar).fold(0.0, f64::max);
            items = catalogue
                .rated(&criteria.materials, top * 10.197)
                .into_iter()
                .cloned()
                .collect();
        }
        if items.is_empty() {
            return Err(DesignError::NoItem(pipe.id.clone()));
        }
        options.push(items);
        design_head.push(head);
    }
    let mut choice: Vec<usize> = vec![0; net.pipes.len()];
    let apply = |net: &mut WaterNetwork, k: usize, item: &CatalogueItem, hazen: bool| {
        let p = &mut net.pipes[k];
        p.diameter_mm = item.internal_mm;
        p.roughness = if hazen {
            item.hazen_c
        } else {
            dw_roughness_mm(&item.material)
        };
        p.material = Some(item.material.clone());
        p.pressure_class_bar = Some(item.pn_bar);
    };
    for k in 0..net.pipes.len() {
        if !fixed[k] {
            apply(&mut net, k, &options[k][0], cx.hazen);
        }
    }

    // 2. Velocity sizing.
    for _ in 0..3 {
        let s = cx.solve(&net)?;
        let mut changed = false;
        for k in 0..net.pipes.len() {
            if fixed[k] {
                continue;
            }
            let q = s.flow_lps[k].abs() / 1000.0;
            let pick = options[k]
                .iter()
                .position(|i| velocity(q, i.internal_mm) <= criteria.max_velocity_ms)
                .unwrap_or(options[k].len() - 1);
            if pick != choice[k] {
                choice[k] = pick;
                apply(&mut net, k, &options[k][pick], cx.hazen);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // 3. Pressure: cost-gradient upgrades along each critical supply path.
    let mut node_links: Vec<Vec<usize>> = vec![Vec::new(); net.node_count()];
    for (k, &(a, b)) in ends.iter().enumerate() {
        node_links[a].push(k);
        node_links[b].push(k);
    }
    let mut upgrades = 0;
    let mut given_up: Vec<bool> = vec![false; nj];
    let mut snapshot = cx.solve(&net)?;
    while cx.simulations < criteria.max_simulations {
        let critical = (0..nj)
            .filter(|&j| !given_up[j] && !snapshot.disconnected.contains(&j))
            .filter(|&j| snapshot.pressure_m[j] < criteria.min_pressure_m - 1e-6)
            .min_by(|&a, &b| snapshot.pressure_m[a].total_cmp(&snapshot.pressure_m[b]));
        let Some(critical) = critical else { break };
        let deficit = criteria.min_pressure_m - snapshot.pressure_m[critical];
        let path = supply_path(critical, nj, &ends, &node_links, &snapshot);
        let mut gained = 0.0;
        let mut any = false;
        while gained < deficit {
            let best = path
                .iter()
                .filter(|&&k| !fixed[k] && choice[k] + 1 < options[k].len())
                .map(|&k| {
                    let q = snapshot.flow_lps[k].abs() / 1000.0;
                    let (now, next) = (&options[k][choice[k]], &options[k][choice[k] + 1]);
                    let l = net.pipes[k].length_m;
                    let gain = hw_loss(q, l, now) - hw_loss(q, l, next);
                    let extra = (next.cost_per_m - now.cost_per_m).max(1e-6) * l;
                    (k, gain, gain / extra)
                })
                .filter(|&(_, gain, _)| gain > 1e-9)
                .max_by(|a, b| a.2.total_cmp(&b.2).then(b.0.cmp(&a.0)));
            let Some((k, gain, _)) = best else { break };
            choice[k] += 1;
            apply(&mut net, k, &options[k][choice[k]], cx.hazen);
            gained += gain;
            upgrades += 1;
            any = true;
        }
        if !any {
            given_up[critical] = true;
        }
        snapshot = cx.solve(&net)?;
    }
    if cx.simulations >= criteria.max_simulations {
        cx.warnings.push(format!(
            "stopped at the limit of {} solves",
            criteria.max_simulations
        ));
    }

    // Report.
    let junction = |j: usize| NodePressure {
        id: net.junctions[j].id.clone(),
        pressure_m: snapshot.pressure_m[j],
    };
    let connected = (0..nj).filter(|j| !snapshot.disconnected.contains(j));
    let min_pressure = connected
        .clone()
        .min_by(|&a, &b| snapshot.pressure_m[a].total_cmp(&snapshot.pressure_m[b]))
        .map(junction);
    let infeasible: Vec<NodePressure> = connected
        .clone()
        .filter(|&j| snapshot.pressure_m[j] < criteria.min_pressure_m - 1e-6)
        .map(junction)
        .collect();
    let over_pressure: Vec<NodePressure> = (0..nj)
        .filter(|&j| static_snapshot.pressure_m[j] > criteria.max_static_pressure_m)
        .map(|j| NodePressure {
            id: net.junctions[j].id.clone(),
            pressure_m: static_snapshot.pressure_m[j],
        })
        .collect();
    let mut pipes = Vec::with_capacity(net.pipes.len());
    let mut low_velocity_pipes = Vec::new();
    for (k, pipe) in net.pipes.iter().enumerate() {
        let item = &options[k][choice[k]];
        let v = snapshot.velocity_ms[k];
        if v < criteria.min_velocity_ms && snapshot.flow_lps[k].abs() > 0.0 {
            low_velocity_pipes.push(pipe.id.clone());
        }
        let cost = if fixed[k] {
            0.0
        } else {
            item.cost_per_m * pipe.length_m
        };
        pipes.push(PipeDesign {
            id: pipe.id.clone(),
            material: pipe.material.clone().unwrap_or_default(),
            nominal_mm: if fixed[k] {
                pipe.diameter_mm
            } else {
                item.nominal_mm
            },
            internal_mm: pipe.diameter_mm,
            pn_bar: pipe.pressure_class_bar.unwrap_or(0.0),
            sdr: if fixed[k] { None } else { item.sdr },
            length_m: pipe.length_m,
            flow_lps: snapshot.flow_lps[k],
            velocity_ms: v,
            headloss_m_per_km: (snapshot.headloss_m[k].abs() / pipe.length_m) * 1000.0,
            design_head_m: design_head[k],
            cost,
            fixed: fixed[k],
        });
    }
    let mut boq: Vec<BoqLine> = Vec::new();
    for p in pipes.iter().filter(|p| !p.fixed) {
        match boq.iter_mut().find(|b| {
            b.material == p.material && b.nominal_mm == p.nominal_mm && b.pn_bar == p.pn_bar
        }) {
            Some(line) => {
                line.length_m += p.length_m;
                line.cost += p.cost;
            }
            None => boq.push(BoqLine {
                material: p.material.clone(),
                nominal_mm: p.nominal_mm,
                pn_bar: p.pn_bar,
                length_m: p.length_m,
                cost: p.cost,
            }),
        }
    }
    boq.sort_by(|a, b| {
        a.material
            .cmp(&b.material)
            .then(a.nominal_mm.total_cmp(&b.nominal_mm))
            .then(a.pn_bar.total_cmp(&b.pn_bar))
    });
    let total_cost = boq.iter().map(|b| b.cost).sum();
    if !over_pressure.is_empty() {
        cx.warnings.push(format!(
            "{} junctions exceed {} m static pressure: place a PRV or break-pressure tank (see the profile)",
            over_pressure.len(),
            criteria.max_static_pressure_m
        ));
    }
    Ok(DesignResult {
        network: net,
        pipes,
        boq,
        total_cost,
        min_pressure,
        infeasible,
        over_pressure,
        low_velocity_pipes,
        simulations: cx.simulations,
        upgrades,
        warnings: cx.warnings,
    })
}

/// Pipes carrying water to `node`: from each node, the pipe with the largest
/// inflow, until a source is reached.
fn supply_path(
    node: usize,
    nj: usize,
    ends: &[(usize, usize)],
    node_links: &[Vec<usize>],
    s: &Snapshot,
) -> Vec<usize> {
    let mut path = Vec::new();
    let mut at = node;
    let mut seen = std::collections::HashSet::new();
    while at < nj && seen.insert(at) {
        let inflow = node_links[at]
            .iter()
            .map(|&k| {
                let q = s.flow_lps[k];
                let into = if ends[k].1 == at { q } else { -q };
                (k, into)
            })
            .filter(|&(_, into)| into > 0.0)
            .max_by(|a, b| a.1.total_cmp(&b.1));
        let Some((k, _)) = inflow else { break };
        path.push(k);
        at = if ends[k].1 == at {
            ends[k].0
        } else {
            ends[k].1
        };
    }
    path
}

fn velocity(q_m3s: f64, internal_mm: f64) -> f64 {
    let d = internal_mm / 1000.0;
    q_m3s / (std::f64::consts::PI * d * d / 4.0)
}

fn hw_loss(q_m3s: f64, length_m: f64, item: &CatalogueItem) -> f64 {
    let d = item.internal_mm / 1000.0;
    10.67 * length_m * q_m3s.powf(1.852) / (item.hazen_c.powf(1.852) * d.powf(4.871))
}

/// Darcy-Weisbach absolute roughness for new pipe, mm.
fn dw_roughness_mm(material: &str) -> f64 {
    match material {
        m if m.starts_with("HDPE") => 0.007,
        m if m.starts_with("PVC") => 0.0015,
        _ => 0.1,
    }
}

#[cfg(test)]
mod tests;
