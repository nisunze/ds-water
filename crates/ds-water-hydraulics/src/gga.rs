//! Global Gradient Algorithm (Todini & Pilati, 1988), the method EPANET uses.
//!
//! Unknowns are junction heads. Each Newton step linearises every link's
//! head-loss law around its current flow, `q' = q - Y + P (Hᵢ - Hⱼ)` with
//! `P = 1/g` and `Y = P·h(q)`, writes continuity at every junction, and
//! solves the resulting symmetric positive-definite system for heads. Flows
//! follow from the heads. Link statuses (check valves, pumps against
//! shut-off, PRV/PSV/FCV regimes) are re-decided as EPANET does: every second
//! iteration early on, then only once flows converge.

use ds_water_model::{
    DemandModel, HeadlossFormula, LinkKind, LinkStatus, NodeKind, ValveKind, WaterNetwork,
};
use serde::{Deserialize, Serialize};

use crate::sparse::{Cholesky, FactorError, Symbolic};

const G: f64 = 9.806_65;
/// Head-loss gradient floor (m per m³/s): links with almost no flow stay
/// linear instead of producing an infinite `P`.
const RQTOL: f64 = 1e-6;
/// Resistance of a closed link (m per m³/s).
const CBIG: f64 = 1e8;
/// Weight that pins a head for an active pressure valve.
const BIG: f64 = 1e10;
const HTOL: f64 = 1.5e-4;
/// Flow below which a link is treated as stopped for status decisions
/// (EPANET's 1e-4 cfs): solver noise in still zones stays under it.
const QTOL: f64 = 2.8e-6;
const QFLOOR: f64 = 1e-6;
const CHECK_EVERY: u32 = 2;
const CHECK_UNTIL: u32 = 10;

#[derive(Debug, thiserror::Error)]
pub enum PrepareError {
    #[error("the network is not valid: {0:?}")]
    Invalid(Vec<ds_water_model::ValidationIssue>),
}

/// How a link is operating in a solution.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Open,
    Closed,
    /// Closed by the solver: a check valve against reverse flow, or a pump
    /// that cannot lift against the head it faces.
    TempClosed,
    /// A PRV, PSV or FCV regulating at its setting.
    Active,
}

#[derive(Clone, Debug)]
enum Element {
    Pipe {
        r: f64,
        n: f64,
        m: f64,
        area: f64,
        d: f64,
        rough: f64,
        cv: bool,
    },
    Pump {
        h0: f64,
        r: f64,
        n: f64,
        speed: f64,
        pattern: Option<usize>,
    },
    Valve {
        kind: ValveKind,
        setting: f64,
        m: f64,
        area: f64,
    },
}

#[derive(Clone, Debug)]
struct Link {
    from: usize,
    to: usize,
    element: Element,
    initial: RunStatus,
    /// Off-diagonal slot in the factor when both ends are junctions.
    position: Option<usize>,
}

/// A network compiled for repeated solves.
pub struct Hydraulics {
    nj: usize,
    elevation: Vec<f64>,
    base_demand: Vec<f64>,
    demand_pattern: Vec<Option<usize>>,
    reservoir_head: Vec<(f64, Option<usize>)>,
    tanks: Vec<TankState>,
    links: Vec<Link>,
    patterns: Vec<Vec<f64>>,
    options: ds_water_model::Options,
    chol: Cholesky,
    link_kinds: Vec<(LinkKind, usize)>,
    node_links: Vec<Vec<usize>>,
}

#[derive(Clone, Debug)]
struct TankState {
    elevation: f64,
    min_level: f64,
    max_level: f64,
    area: f64,
}

/// One hydraulic solution, in the model's own orders: nodes are junctions,
/// then reservoirs, then tanks; links are pipes, then pumps, then valves.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub time_s: u64,
    pub converged: bool,
    pub iterations: u32,
    pub relative_change: f64,
    pub head_m: Vec<f64>,
    pub pressure_m: Vec<f64>,
    /// Demand actually delivered, L/s (less than asked under PDA).
    pub delivered_lps: Vec<f64>,
    pub flow_lps: Vec<f64>,
    pub velocity_ms: Vec<f64>,
    pub headloss_m: Vec<f64>,
    pub status: Vec<RunStatus>,
    /// Junctions with no open path to any reservoir or tank.
    pub disconnected: Vec<usize>,
    pub warnings: Vec<String>,
}

impl Hydraulics {
    pub fn new(network: &WaterNetwork) -> Result<Self, PrepareError> {
        let issues = ds_water_model::validate(network);
        if !issues.is_empty() {
            return Err(PrepareError::Invalid(issues));
        }
        let nj = network.junctions.len();
        let index: std::collections::HashMap<&str, usize> = network
            .node_refs()
            .enumerate()
            .map(|(i, (id, _))| (id, i))
            .collect();
        let pattern_index = |id: &Option<String>| {
            id.as_ref()
                .and_then(|p| network.patterns.iter().position(|x| &x.id == p))
        };
        let mut elevation = Vec::with_capacity(network.node_count());
        for (_, node) in network.node_refs() {
            elevation.push(match node.kind {
                NodeKind::Junction => network.junctions[node.index].elevation_m,
                NodeKind::Reservoir => network.reservoirs[node.index].head_m,
                NodeKind::Tank => network.tanks[node.index].elevation_m,
            });
        }
        let options = network.options.clone();
        let mut links = Vec::with_capacity(network.link_count());
        let mut link_kinds = Vec::with_capacity(network.link_count());
        for (i, p) in network.pipes.iter().enumerate() {
            let d = p.diameter_mm / 1000.0;
            let area = std::f64::consts::PI * d * d / 4.0;
            let (r, n) = match options.headloss {
                HeadlossFormula::HazenWilliams => (
                    10.67 * p.length_m / (p.roughness.powf(1.852) * d.powf(4.871)),
                    1.852,
                ),
                HeadlossFormula::DarcyWeisbach => (
                    8.0 * p.length_m / (std::f64::consts::PI.powi(2) * G * d.powi(5)),
                    2.0,
                ),
            };
            let initial = match p.status {
                LinkStatus::Closed => RunStatus::Closed,
                _ => RunStatus::Open,
            };
            links.push(Link {
                from: index[p.from.as_str()],
                to: index[p.to.as_str()],
                element: Element::Pipe {
                    r,
                    n,
                    m: minor(p.minor_loss, area),
                    area,
                    d,
                    rough: p.roughness / 1000.0,
                    cv: p.status == LinkStatus::CheckValve,
                },
                initial,
                position: None,
            });
            link_kinds.push((LinkKind::Pipe, i));
        }
        for (i, p) in network.pumps.iter().enumerate() {
            let curve = network.curve(&p.curve).expect("validated");
            let (h0, r, n) = pump_curve(&curve.points);
            links.push(Link {
                from: index[p.from.as_str()],
                to: index[p.to.as_str()],
                element: Element::Pump {
                    h0,
                    r,
                    n,
                    speed: p.speed,
                    pattern: pattern_index(&p.pattern),
                },
                initial: if p.status == LinkStatus::Closed {
                    RunStatus::Closed
                } else {
                    RunStatus::Open
                },
                position: None,
            });
            link_kinds.push((LinkKind::Pump, i));
        }
        for (i, v) in network.valves.iter().enumerate() {
            let d = v.diameter_mm / 1000.0;
            let area = std::f64::consts::PI * d * d / 4.0;
            let (setting, k) = match v.kind {
                ValveKind::Prv | ValveKind::Psv => (v.setting, v.minor_loss),
                ValveKind::Fcv => (v.setting / 1000.0, v.minor_loss),
                ValveKind::Tcv => (v.setting, v.setting),
            };
            let initial = match (v.status, v.kind) {
                (Some(LinkStatus::Closed), _) => RunStatus::Closed,
                (Some(LinkStatus::Open), _) | (_, ValveKind::Tcv) => RunStatus::Open,
                _ => RunStatus::Active,
            };
            links.push(Link {
                from: index[v.from.as_str()],
                to: index[v.to.as_str()],
                element: Element::Valve {
                    kind: v.kind,
                    setting,
                    m: minor(k, area),
                    area,
                },
                initial,
                position: None,
            });
            link_kinds.push((LinkKind::Valve, i));
        }

        let pattern: Vec<(usize, usize)> = links
            .iter()
            .filter(|l| l.from < nj && l.to < nj)
            .map(|l| (l.from, l.to))
            .collect();
        let symbolic = Symbolic::analyse(nj, &pattern);
        for link in &mut links {
            if link.from < nj && link.to < nj {
                link.position = symbolic.position(link.from, link.to);
            }
        }
        let mut node_links = vec![Vec::new(); elevation.len()];
        for (k, link) in links.iter().enumerate() {
            node_links[link.from].push(k);
            node_links[link.to].push(k);
        }
        Ok(Self {
            nj,
            node_links,
            elevation,
            base_demand: network
                .junctions
                .iter()
                .map(|j| j.demand_lps / 1000.0)
                .collect(),
            demand_pattern: network
                .junctions
                .iter()
                .map(|j| pattern_index(&j.pattern))
                .collect(),
            reservoir_head: network
                .reservoirs
                .iter()
                .map(|r| (r.head_m, pattern_index(&r.pattern)))
                .collect(),
            tanks: network
                .tanks
                .iter()
                .map(|t| TankState {
                    elevation: t.elevation_m,
                    min_level: t.min_level_m,
                    max_level: t.max_level_m,
                    area: t.area_m2(),
                })
                .collect(),
            links,
            patterns: network
                .patterns
                .iter()
                .map(|p| p.multipliers.clone())
                .collect(),
            options,
            chol: Cholesky::new(symbolic),
            link_kinds,
        })
    }

    /// Junction count, link count and non-zeros of the Cholesky factor.
    pub fn size(&self) -> (usize, usize, usize) {
        (
            self.nj,
            self.links.len(),
            self.chol.symbolic().factor_nonzeros(),
        )
    }

    pub fn link_kinds(&self) -> &[(LinkKind, usize)] {
        &self.link_kinds
    }

    fn multiplier(&self, pattern: Option<usize>, time_s: u64) -> f64 {
        match pattern {
            None => 1.0,
            Some(p) => {
                let values = &self.patterns[p];
                if values.is_empty() || self.options.pattern_step_s == 0 {
                    1.0
                } else {
                    values[(time_s / self.options.pattern_step_s) as usize % values.len()]
                }
            }
        }
    }

    pub fn initial_tank_levels(network: &WaterNetwork) -> Vec<f64> {
        network.tanks.iter().map(|t| t.initial_level_m).collect()
    }

    /// Solve one instant with the given tank levels.
    pub fn solve(&mut self, time_s: u64, tank_levels: &[f64]) -> Snapshot {
        let nj = self.nj;
        let nn = self.elevation.len();
        let nr = self.reservoir_head.len();
        let mut head = self.elevation.clone();
        for (r, &(h, pat)) in self.reservoir_head.iter().enumerate() {
            head[nj + r] = h * self.multiplier(pat, time_s);
        }
        for (t, tank) in self.tanks.iter().enumerate() {
            head[nj + nr + t] = tank.elevation + tank_levels[t];
        }
        let demand: Vec<f64> = (0..nj)
            .map(|j| {
                self.base_demand[j]
                    * self.options.demand_multiplier
                    * self.multiplier(self.demand_pattern[j], time_s)
            })
            .collect();
        let pda = self.options.demand_model == DemandModel::PressureDriven;
        let (p_min, p_req) = (
            self.options.minimum_pressure_m,
            self.options
                .required_pressure_m
                .max(self.options.minimum_pressure_m + 0.1),
        );
        let mut delivered = demand.clone();
        for j in 0..nj {
            head[j] = self.elevation[j] + p_req.max(1.0);
        }

        let mut status: Vec<RunStatus> = self.links.iter().map(|l| l.initial).collect();
        let mut flow: Vec<f64> = self
            .links
            .iter()
            .map(|l| match &l.element {
                Element::Pipe { area, .. } | Element::Valve { area, .. } => area * 0.3,
                Element::Pump { r, h0, n, .. } => (h0 / (2.0 * r.max(1e-12))).powf(1.0 / n),
            })
            .collect();
        let mut warnings = Vec::new();
        let nnz = self.chol.symbolic().factor_nonzeros();
        let mut diag = vec![0.0; nj];
        let mut lower = vec![0.0; nnz];
        let mut rhs = vec![0.0; nj];
        let mut p_coef = vec![0.0; self.links.len()];
        let mut pda_coef = vec![(0.0, 0.0); nj];
        let mut y_coef = vec![0.0; self.links.len()];
        let mut disconnected = self.disconnected(&status, nn);
        let mut converged = false;
        let mut relative = f64::INFINITY;
        let mut iterations = 0;
        let max_iterations = self.options.max_iterations.max(1);

        while iterations < max_iterations {
            iterations += 1;
            diag.fill(0.0);
            lower.fill(0.0);
            // Under PDA every junction with demand delivers through a virtual
            // link (below), so its outflow is an unknown, not a constant.
            for j in 0..nj {
                rhs[j] = if disconnected[j] || (pda && demand[j] > 0.0) {
                    0.0
                } else {
                    -demand[j]
                };
            }

            for (k, link) in self.links.iter().enumerate() {
                let (i, j) = (link.from, link.to);
                if (i < nj && disconnected[i]) || (j < nj && disconnected[j]) {
                    p_coef[k] = 0.0;
                    y_coef[k] = flow[k];
                    continue;
                }
                if matches!(status[k], RunStatus::Closed | RunStatus::TempClosed) {
                    // A closed link leaves the system; disconnection is found
                    // by graph search, so no stand-in resistance is needed.
                    p_coef[k] = 0.0;
                    flow[k] = 0.0;
                    continue;
                }
                let q = flow[k];
                match (status[k], &link.element) {
                    (
                        RunStatus::Active,
                        Element::Valve {
                            kind: ValveKind::Prv,
                            setting,
                            ..
                        },
                    ) if j < nj => {
                        p_coef[k] = 0.0;
                        diag[j] += BIG;
                        rhs[j] += BIG * (self.elevation[j] + setting);
                        if i < nj {
                            rhs[i] -= q;
                        }
                        continue;
                    }
                    (
                        RunStatus::Active,
                        Element::Valve {
                            kind: ValveKind::Psv,
                            setting,
                            ..
                        },
                    ) if i < nj => {
                        p_coef[k] = 0.0;
                        diag[i] += BIG;
                        rhs[i] += BIG * (self.elevation[i] + setting);
                        if j < nj {
                            rhs[j] += q;
                        }
                        continue;
                    }
                    (
                        RunStatus::Active,
                        Element::Valve {
                            kind: ValveKind::Fcv,
                            setting,
                            ..
                        },
                    ) => {
                        p_coef[k] = 0.0;
                        flow[k] = *setting;
                        if i < nj {
                            rhs[i] -= setting;
                        }
                        if j < nj {
                            rhs[j] += setting;
                        }
                        continue;
                    }
                    _ => {}
                }
                let (h, g) = self.headloss(k, q, status[k], time_s);
                let p = 1.0 / g;
                p_coef[k] = p;
                y_coef[k] = p * h;
                let excess = q - y_coef[k];
                if i < nj {
                    diag[i] += p;
                    rhs[i] -= excess;
                } else if j < nj {
                    rhs[j] += p * head[i];
                }
                if j < nj {
                    diag[j] += p;
                    rhs[j] += excess;
                } else if i < nj {
                    rhs[i] += p * head[j];
                }
                if let Some(pos) = link.position {
                    lower[pos] -= p;
                }
            }

            // Pressure-driven demand (EPANET 2.2): delivery d flows through a
            // virtual link to the head at which supply starts, with loss
            // (p_req - p_min)·(d/D)², and barriers below 0 and above D.
            if pda {
                for j in 0..nj {
                    if disconnected[j] || demand[j] <= 0.0 {
                        continue;
                    }
                    let (h, g) = demand_headloss(delivered[j], demand[j], p_req - p_min);
                    let p = 1.0 / g;
                    pda_coef[j] = (p, p * h);
                    diag[j] += p;
                    rhs[j] += -(delivered[j] - p * h) + p * (self.elevation[j] + p_min);
                }
            }

            for j in 0..nj {
                if disconnected[j] {
                    diag[j] = 1.0;
                    rhs[j] = self.elevation[j];
                }
            }
            for link in &self.links {
                if let Some(pos) = link.position
                    && (disconnected[link.from] || disconnected[link.to])
                {
                    lower[pos] = 0.0;
                }
            }

            if let Err(FactorError::NotPositiveDefinite { unknown }) =
                self.chol.factor(&diag, &lower)
            {
                // Should not happen once disconnected zones are excluded;
                // isolate the junction and carry on rather than abort a run.
                disconnected[unknown] = true;
                warnings.push(format!(
                    "junction #{unknown} isolated: singular system at iteration {iterations}"
                ));
                continue;
            }
            self.chol.solve(&mut rhs);
            head[..nj].copy_from_slice(&rhs);

            let mut sum_dq = 0.0;
            let mut sum_q = 0.0;
            let mut max_dq: f64 = 0.0;
            for (k, link) in self.links.iter().enumerate() {
                if p_coef[k] == 0.0 {
                    continue;
                }
                let (i, j) = (link.from, link.to);
                let new = flow[k] - y_coef[k] + p_coef[k] * (head[i] - head[j]);
                sum_dq += (new - flow[k]).abs();
                max_dq = max_dq.max((new - flow[k]).abs());
                sum_q += new.abs();
                flow[k] = new;
            }
            // Flows of head-fixing valves follow from continuity at the node
            // whose head they hold.
            for (k, link) in self.links.iter().enumerate() {
                if status[k] != RunStatus::Active {
                    continue;
                }
                let Element::Valve { kind, .. } = link.element else {
                    continue;
                };
                let (node, sign) = match kind {
                    ValveKind::Prv if link.to < nj => (link.to, 1.0),
                    ValveKind::Psv if link.from < nj => (link.from, -1.0),
                    _ => continue,
                };
                let mut out = delivered[node];
                for &m in &self.node_links[node] {
                    if m == k {
                        continue;
                    }
                    if self.links[m].from == node {
                        out += flow[m];
                    } else {
                        out -= flow[m];
                    }
                }
                let new = if sign > 0.0 { out } else { -out };
                sum_dq += (new - flow[k]).abs();
                max_dq = max_dq.max((new - flow[k]).abs());
                sum_q += new.abs();
                flow[k] = new;
            }
            if pda {
                for j in 0..nj {
                    if disconnected[j] || demand[j] <= 0.0 {
                        continue;
                    }
                    let (p, y) = pda_coef[j];
                    let new = delivered[j] - y + p * (head[j] - self.elevation[j] - p_min);
                    sum_dq += (new - delivered[j]).abs();
                    max_dq = max_dq.max((new - delivered[j]).abs());
                    sum_q += new.abs();
                    delivered[j] = new;
                }
            }
            // A floor of 1 mL/s on the total flow: with no demand every flow
            // tends to zero and the plain ratio would never settle.
            relative = sum_dq / sum_q.max(QFLOOR);

            // Or every flow moved by less than the stopped-flow
            // tolerance: what is left is round-off in still zones.
            let flows_converged = relative <= self.options.accuracy || max_dq <= QTOL;
            let check_now =
                flows_converged || (iterations <= CHECK_UNTIL && iterations % CHECK_EVERY == 0);
            if check_now {
                let changed = self.update_status(&mut status, &flow, &head, time_s);
                if changed {
                    disconnected = self.disconnected(&status, nn);
                    continue;
                }
            }
            if flows_converged {
                converged = true;
                break;
            }
        }
        if !converged {
            warnings.push(format!(
                "not converged after {iterations} iterations (relative flow change {relative:.3e})"
            ));
        }

        let mut snapshot = Snapshot {
            time_s,
            converged,
            iterations,
            relative_change: relative,
            pressure_m: (0..nn).map(|i| head[i] - self.elevation[i]).collect(),
            head_m: head.clone(),
            delivered_lps: (0..nj)
                .map(|j| match (disconnected[j], pda) {
                    (true, _) => 0.0,
                    (false, true) => delivered[j].clamp(0.0, demand[j]) * 1000.0,
                    (false, false) => demand[j] * 1000.0,
                })
                .collect(),
            flow_lps: Vec::with_capacity(self.links.len()),
            velocity_ms: Vec::with_capacity(self.links.len()),
            headloss_m: Vec::with_capacity(self.links.len()),
            status: status.clone(),
            disconnected: (0..nj).filter(|&j| disconnected[j]).collect(),
            warnings,
        };
        for (k, link) in self.links.iter().enumerate() {
            let q = if matches!(status[k], RunStatus::Closed | RunStatus::TempClosed) {
                0.0
            } else {
                flow[k]
            };
            let area = match link.element {
                Element::Pipe { area, .. } | Element::Valve { area, .. } => area,
                Element::Pump { .. } => 0.0,
            };
            snapshot.flow_lps.push(q * 1000.0);
            snapshot
                .velocity_ms
                .push(if area > 0.0 { q.abs() / area } else { 0.0 });
            snapshot.headloss_m.push(head[link.from] - head[link.to]);
        }
        if !snapshot.disconnected.is_empty() {
            snapshot.warnings.push(format!(
                "{} junctions have no open path to a reservoir or tank",
                snapshot.disconnected.len()
            ));
        }
        snapshot
    }

    /// Head loss `h` from `from` to `to` and its gradient `g = dh/dq`.
    fn headloss(&self, k: usize, q: f64, status: RunStatus, time_s: u64) -> (f64, f64) {
        if matches!(status, RunStatus::Closed | RunStatus::TempClosed) {
            return (CBIG * q, CBIG);
        }
        let aq = q.abs();
        match &self.links[k].element {
            Element::Pipe {
                r,
                n,
                m,
                area,
                d,
                rough,
                ..
            } => {
                let (h, g) = match self.options.headloss {
                    HeadlossFormula::HazenWilliams => (r * aq.powf(*n), n * r * aq.powf(n - 1.0)),
                    HeadlossFormula::DarcyWeisbach => {
                        let f = friction(aq, *area, *d, *rough, self.options.viscosity_m2s);
                        (f * r * aq * aq, 2.0 * f * r * aq)
                    }
                };
                let (h, g) = (h + m * aq * aq, g + 2.0 * m * aq);
                if g < RQTOL {
                    (RQTOL * q, RQTOL)
                } else {
                    (h * q.signum(), g)
                }
            }
            Element::Pump {
                h0,
                r,
                n,
                speed,
                pattern,
            } => {
                let w = speed * self.multiplier(*pattern, time_s);
                if w <= 0.0 {
                    return (CBIG * q, CBIG);
                }
                // Head gain w²·(h0 - r·(q/w)ⁿ) appears as a negative loss.
                let qa = aq.max(1e-9);
                let gain = w * w * h0 - r * w.powf(2.0 - n) * qa.powf(*n);
                let g = (n * r * w.powf(2.0 - n) * qa.powf(n - 1.0)).max(RQTOL);
                (-gain, g)
            }
            Element::Valve { m, .. } => {
                let (h, g) = (m * aq * aq, 2.0 * m * aq);
                if g < RQTOL {
                    (RQTOL * q, RQTOL)
                } else {
                    (h * q.signum(), g)
                }
            }
        }
    }

    fn update_status(
        &self,
        status: &mut [RunStatus],
        flow: &[f64],
        head: &[f64],
        time_s: u64,
    ) -> bool {
        let mut changed = false;
        for (k, link) in self.links.iter().enumerate() {
            if link.initial == RunStatus::Closed {
                continue;
            }
            let (hi, hj) = (head[link.from], head[link.to]);
            let dh = hi - hj;
            let old = status[k];
            let new = match &link.element {
                Element::Pipe { cv: true, .. } => match old {
                    RunStatus::Open if flow[k] < -QTOL => RunStatus::TempClosed,
                    RunStatus::TempClosed if dh > HTOL => RunStatus::Open,
                    s => s,
                },
                Element::Pipe { .. } => old,
                Element::Pump {
                    h0, speed, pattern, ..
                } => {
                    let w = speed * self.multiplier(*pattern, time_s);
                    let shutoff = w * w * h0;
                    match old {
                        RunStatus::Open if -dh > shutoff + HTOL || flow[k] < -QTOL => {
                            RunStatus::TempClosed
                        }
                        RunStatus::TempClosed if -dh < shutoff - HTOL => RunStatus::Open,
                        s => s,
                    }
                }
                Element::Valve { kind, setting, .. } => match kind {
                    ValveKind::Tcv => old,
                    ValveKind::Prv => {
                        let hset = self.elevation[link.to] + setting;
                        match old {
                            RunStatus::Active if flow[k] < -QTOL => RunStatus::TempClosed,
                            RunStatus::Active if hi < hset - HTOL => RunStatus::Open,
                            RunStatus::Open if flow[k] < -QTOL => RunStatus::TempClosed,
                            RunStatus::Open if hj > hset + HTOL => RunStatus::Active,
                            RunStatus::TempClosed if hi >= hset + HTOL && hj < hset - HTOL => {
                                RunStatus::Active
                            }
                            RunStatus::TempClosed if hi < hset - HTOL && hi > hj + HTOL => {
                                RunStatus::Open
                            }
                            s => s,
                        }
                    }
                    ValveKind::Psv => {
                        let hset = self.elevation[link.from] + setting;
                        match old {
                            RunStatus::Active if flow[k] < -QTOL => RunStatus::TempClosed,
                            RunStatus::Active if hj > hset + HTOL => RunStatus::Open,
                            RunStatus::Open if flow[k] < -QTOL => RunStatus::TempClosed,
                            RunStatus::Open if hi < hset - HTOL => RunStatus::Active,
                            RunStatus::TempClosed if hj < hset - HTOL && hi > hset + HTOL => {
                                RunStatus::Active
                            }
                            RunStatus::TempClosed if hj > hset + HTOL && hi > hj + HTOL => {
                                RunStatus::Open
                            }
                            s => s,
                        }
                    }
                    ValveKind::Fcv => match old {
                        RunStatus::Active if dh < -HTOL => RunStatus::Open,
                        RunStatus::Open if flow[k] > *setting + QTOL => RunStatus::Active,
                        s => s,
                    },
                },
            };
            if new != old {
                status[k] = new;
                changed = true;
            }
        }
        changed
    }

    /// Junctions with no path through non-closed links to a fixed head.
    fn disconnected(&self, status: &[RunStatus], nn: usize) -> Vec<bool> {
        let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); nn];
        for (k, link) in self.links.iter().enumerate() {
            if !matches!(status[k], RunStatus::Closed | RunStatus::TempClosed) {
                adjacency[link.from].push(link.to);
                adjacency[link.to].push(link.from);
            }
        }
        let mut reached = vec![false; nn];
        let mut stack: Vec<usize> = (self.nj..nn).collect();
        for &s in &stack {
            reached[s] = true;
        }
        while let Some(v) = stack.pop() {
            for &u in &adjacency[v] {
                if !reached[u] {
                    reached[u] = true;
                    stack.push(u);
                }
            }
        }
        reached.iter().map(|r| !r).collect()
    }

    /// Extended-period simulation: tank levels integrate net inflow between
    /// hydraulic steps. A full tank spills and an empty one stops supplying;
    /// both are reported as warnings at the step they happen.
    pub fn run(&mut self, network: &WaterNetwork) -> Vec<Snapshot> {
        let mut levels = Self::initial_tank_levels(network);
        let step = self.options.hydraulic_step_s.max(1);
        let duration = self.options.duration_s;
        let nj = self.nj;
        let nr = self.reservoir_head.len();
        let mut out = Vec::new();
        let mut t = 0;
        loop {
            let mut snapshot = self.solve(t, &levels);
            if t >= duration {
                out.push(snapshot);
                break;
            }
            let dt = step.min(duration - t);
            for (ti, tank) in self.tanks.iter().enumerate() {
                let node = nj + nr + ti;
                let mut inflow = 0.0;
                for (k, link) in self.links.iter().enumerate() {
                    if link.to == node {
                        inflow += snapshot.flow_lps[k] / 1000.0;
                    } else if link.from == node {
                        inflow -= snapshot.flow_lps[k] / 1000.0;
                    }
                }
                let level = levels[ti] + inflow * dt as f64 / tank.area;
                if level > tank.max_level {
                    snapshot
                        .warnings
                        .push(format!("tank #{ti} full and overflowing"));
                } else if level < tank.min_level {
                    snapshot.warnings.push(format!("tank #{ti} empty"));
                }
                levels[ti] = level.clamp(tank.min_level, tank.max_level);
            }
            out.push(snapshot);
            t += dt;
        }
        out
    }
}

/// Virtual-link loss above the no-supply head for a delivery `d` of a full
/// demand `full`, and its gradient (EPANET 2.2 `demandheadloss`).
fn demand_headloss(d: f64, full: f64, band: f64) -> (f64, f64) {
    const N: f64 = 2.0;
    let r = d / full;
    if r <= 0.0 {
        (CBIG * d, CBIG)
    } else if r < 1.0 {
        let g = N * band * r.powf(N - 1.0) / full;
        if g < RQTOL {
            (RQTOL * d, RQTOL)
        } else {
            (g * d / N, g)
        }
    } else {
        (band + CBIG * (d - full), CBIG)
    }
}

/// Minor-loss coefficient K as m per (m³/s)².
fn minor(k: f64, area: f64) -> f64 {
    if area > 0.0 {
        k / (2.0 * G * area * area)
    } else {
        0.0
    }
}

/// Darcy friction factor: laminar below Re 2000, Swamee-Jain above 4000,
/// linear between.
fn friction(q: f64, area: f64, d: f64, rough: f64, nu: f64) -> f64 {
    let re = (q / area) * d / nu;
    let turbulent = |re: f64| {
        let x = rough / (3.7 * d) + 5.74 / re.powf(0.9);
        0.25 / x.log10().powi(2)
    };
    if re < 1.0 {
        64.0
    } else if re < 2000.0 {
        64.0 / re
    } else if re < 4000.0 {
        let (f2, f4) = (64.0 / 2000.0, turbulent(4000.0));
        f2 + (f4 - f2) * (re - 2000.0) / 2000.0
    } else {
        turbulent(re)
    }
}

/// Head curve `h = h0 - r·qⁿ` (q in m³/s) from EPANET's one- or
/// three-point definitions (flow L/s, head m).
fn pump_curve(points: &[[f64; 2]]) -> (f64, f64, f64) {
    match points {
        [[q1, h1]] => {
            let q1 = q1 / 1000.0;
            let h0 = 4.0 / 3.0 * h1;
            (h0, (h0 - h1) / (q1 * q1), 2.0)
        }
        [[q0, h0], [q1, h1], [q2, h2]] if *q0 == 0.0 => {
            let (q1, q2) = (q1 / 1000.0, q2 / 1000.0);
            let n = ((h0 - h2) / (h0 - h1)).ln() / (q2 / q1).ln();
            (*h0, (h0 - h1) / q1.powf(n), n)
        }
        [[q0, h0], [q1, h1], [q2, h2]] => {
            // No shut-off point: fit a parabola through the three points and
            // read its shut-off head.
            let (x0, x1, x2) = (q0 / 1000.0, q1 / 1000.0, q2 / 1000.0);
            let r = ((h0 - h1) / (x1 * x1 - x0 * x0) + (h1 - h2) / (x2 * x2 - x1 * x1)) / 2.0;
            (h1 + r * x1 * x1, r.max(1e-9), 2.0)
        }
        _ => (0.0, 1.0, 2.0),
    }
}
