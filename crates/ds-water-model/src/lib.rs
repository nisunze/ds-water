//! The Water network model.
//!
//! One serialisable description of a pressurised water network: junctions,
//! reservoirs and tanks (nodes); pipes, pumps and valves (links); demand
//! patterns and pump curves. Every quantity is SI: metres, millimetres for
//! diameters, litres per second for flows and demands, seconds for time.
//!
//! The model knows no solver. `ds-water-hydraulics` reads it, `ds-water-design`
//! rewrites pipe sizes in it, and [`inp`] exchanges it with EPANET's INP text
//! format, the industry's common water-network interchange.

pub mod inp;
mod validate;

use serde::{Deserialize, Serialize};

pub use validate::{ValidationIssue, validate};

/// Schema id written into every serialised network.
pub const SCHEMA: &str = "ds.water.network/v1";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WaterNetwork {
    #[serde(default = "schema")]
    pub schema: String,
    #[serde(default)]
    pub name: String,
    /// Coordinate reference of node `x`/`y` and link vertices, e.g.
    /// `EPSG:4326`. Hydraulics never reads coordinates; layout and drawings do.
    #[serde(default)]
    pub crs: Option<String>,
    #[serde(default)]
    pub options: Options,
    #[serde(default)]
    pub junctions: Vec<Junction>,
    #[serde(default)]
    pub reservoirs: Vec<Reservoir>,
    #[serde(default)]
    pub tanks: Vec<Tank>,
    #[serde(default)]
    pub pipes: Vec<Pipe>,
    #[serde(default)]
    pub pumps: Vec<Pump>,
    #[serde(default)]
    pub valves: Vec<Valve>,
    #[serde(default)]
    pub patterns: Vec<Pattern>,
    #[serde(default)]
    pub curves: Vec<Curve>,
}

fn schema() -> String {
    SCHEMA.to_owned()
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HeadlossFormula {
    /// Hazen-Williams; pipe roughness is the C factor.
    #[default]
    HazenWilliams,
    /// Darcy-Weisbach with Swamee-Jain friction; roughness is in millimetres.
    DarcyWeisbach,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Options {
    pub headloss: HeadlossFormula,
    /// Kinematic viscosity, m²/s (water at 20 °C by default).
    pub viscosity_m2s: f64,
    /// Convergence: sum of |flow change| over sum of |flow|.
    pub accuracy: f64,
    pub max_iterations: u32,
    /// Global multiplier on every junction demand.
    pub demand_multiplier: f64,
    /// Extended-period length; 0 is a single snapshot.
    pub duration_s: u64,
    pub hydraulic_step_s: u64,
    pub pattern_step_s: u64,
    /// Pressure a junction needs for full service (design and PDA), m.
    pub required_pressure_m: f64,
    /// Pressure below which a junction delivers nothing (PDA), m.
    pub minimum_pressure_m: f64,
    pub demand_model: DemandModel,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            headloss: HeadlossFormula::HazenWilliams,
            viscosity_m2s: 1.004e-6,
            accuracy: 1e-3,
            max_iterations: 200,
            demand_multiplier: 1.0,
            duration_s: 0,
            hydraulic_step_s: 3600,
            pattern_step_s: 3600,
            required_pressure_m: 10.0,
            minimum_pressure_m: 0.0,
            demand_model: DemandModel::DemandDriven,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DemandModel {
    /// Every demand is met whatever the pressure (EPANET's DDA).
    #[default]
    DemandDriven,
    /// Delivery falls with pressure between the minimum and required
    /// pressures (Wagner), so intermittent and under-pressured supply shows.
    PressureDriven,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Junction {
    pub id: String,
    pub elevation_m: f64,
    #[serde(default)]
    pub demand_lps: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
}

/// A fixed-head source: a lake, a spring box, a borehole's static level, a
/// treatment works' clear-water level or a break-pressure tank.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Reservoir {
    pub id: String,
    pub head_m: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
}

/// A cylindrical storage tank; head is its bottom elevation plus level.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Tank {
    pub id: String,
    pub elevation_m: f64,
    pub initial_level_m: f64,
    pub min_level_m: f64,
    pub max_level_m: f64,
    pub diameter_m: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
}

impl Tank {
    pub fn area_m2(&self) -> f64 {
        std::f64::consts::PI * self.diameter_m * self.diameter_m / 4.0
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LinkStatus {
    #[default]
    Open,
    Closed,
    /// Pipe with a check valve: flow only from `from` to `to`.
    CheckValve,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Pipe {
    pub id: String,
    pub from: String,
    pub to: String,
    pub length_m: f64,
    /// Internal diameter.
    pub diameter_mm: f64,
    /// Hazen-Williams C, or Darcy-Weisbach absolute roughness in mm.
    pub roughness: f64,
    #[serde(default)]
    pub minor_loss: f64,
    #[serde(default)]
    pub status: LinkStatus,
    /// Catalogue material, e.g. `HDPE PE100`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    /// Nominal pressure class, bar (PN).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressure_class_bar: Option<f64>,
    /// Interior plan vertices between the end nodes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vertices: Vec<[f64; 2]>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Pump {
    pub id: String,
    pub from: String,
    pub to: String,
    /// Head curve id (flow L/s → head m).
    pub curve: String,
    /// Relative speed; 0 is off.
    #[serde(default = "one")]
    pub speed: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default)]
    pub status: LinkStatus,
}

fn one() -> f64 {
    1.0
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ValveKind {
    /// Pressure reducing: holds downstream pressure at the setting (m).
    Prv,
    /// Pressure sustaining: holds upstream pressure at the setting (m).
    Psv,
    /// Flow control: caps flow at the setting (L/s).
    Fcv,
    /// Throttle control: a minor-loss coefficient equal to the setting.
    #[default]
    Tcv,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Valve {
    pub id: String,
    pub from: String,
    pub to: String,
    pub diameter_mm: f64,
    pub kind: ValveKind,
    pub setting: f64,
    #[serde(default)]
    pub minor_loss: f64,
    #[serde(default)]
    pub status: Option<LinkStatus>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Pattern {
    pub id: String,
    pub multipliers: Vec<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Curve {
    pub id: String,
    /// (x, y) points; for a pump head curve, (flow L/s, head m).
    pub points: Vec<[f64; 2]>,
}

/// Kind of a node, as the solver and the drawings see it.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Junction,
    Reservoir,
    Tank,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    Pipe,
    Pump,
    Valve,
}

/// A node reference resolved to its kind and position in the model's vec.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeRef {
    pub kind: NodeKind,
    pub index: usize,
}

impl WaterNetwork {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            schema: schema(),
            name: name.into(),
            ..Self::default()
        }
    }

    pub fn node_count(&self) -> usize {
        self.junctions.len() + self.reservoirs.len() + self.tanks.len()
    }

    pub fn link_count(&self) -> usize {
        self.pipes.len() + self.pumps.len() + self.valves.len()
    }

    /// Every node id with its kind and index, junctions first.
    pub fn node_refs(&self) -> impl Iterator<Item = (&str, NodeRef)> {
        let j = self.junctions.iter().enumerate().map(|(index, n)| {
            (
                n.id.as_str(),
                NodeRef {
                    kind: NodeKind::Junction,
                    index,
                },
            )
        });
        let r = self.reservoirs.iter().enumerate().map(|(index, n)| {
            (
                n.id.as_str(),
                NodeRef {
                    kind: NodeKind::Reservoir,
                    index,
                },
            )
        });
        let t = self.tanks.iter().enumerate().map(|(index, n)| {
            (
                n.id.as_str(),
                NodeRef {
                    kind: NodeKind::Tank,
                    index,
                },
            )
        });
        j.chain(r).chain(t)
    }

    pub fn node_xy(&self, node: NodeRef) -> Option<[f64; 2]> {
        let (x, y) = match node.kind {
            NodeKind::Junction => (self.junctions[node.index].x, self.junctions[node.index].y),
            NodeKind::Reservoir => (self.reservoirs[node.index].x, self.reservoirs[node.index].y),
            NodeKind::Tank => (self.tanks[node.index].x, self.tanks[node.index].y),
        };
        Some([x?, y?])
    }

    /// Ground (or bottom) elevation of a node; a reservoir's is its head.
    pub fn node_elevation(&self, node: NodeRef) -> f64 {
        match node.kind {
            NodeKind::Junction => self.junctions[node.index].elevation_m,
            NodeKind::Reservoir => self.reservoirs[node.index].head_m,
            NodeKind::Tank => self.tanks[node.index].elevation_m,
        }
    }

    pub fn pattern(&self, id: &str) -> Option<&Pattern> {
        self.patterns.iter().find(|p| p.id == id)
    }

    pub fn curve(&self, id: &str) -> Option<&Curve> {
        self.curves.iter().find(|c| c.id == id)
    }

    /// Total base demand, L/s, before the global multiplier.
    pub fn total_base_demand_lps(&self) -> f64 {
        self.junctions.iter().map(|j| j.demand_lps).sum()
    }

    /// Total pipe length per internal diameter (mm), ascending.
    pub fn pipe_length_by_diameter(&self) -> Vec<(f64, f64)> {
        let mut out: Vec<(f64, f64)> = Vec::new();
        for pipe in &self.pipes {
            match out
                .iter_mut()
                .find(|(d, _)| (*d - pipe.diameter_mm).abs() < 1e-9)
            {
                Some((_, len)) => *len += pipe.length_m,
                None => out.push((pipe.diameter_mm, pipe.length_m)),
            }
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }

    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a water network always serialises")
    }
}

impl Pattern {
    /// Multiplier at `time_s` for a pattern stepping every `step_s`.
    pub fn at(&self, time_s: u64, step_s: u64) -> f64 {
        if self.multipliers.is_empty() || step_s == 0 {
            return 1.0;
        }
        let period = (time_s / step_s) as usize % self.multipliers.len();
        self.multipliers[period]
    }
}

#[cfg(test)]
mod tests;
