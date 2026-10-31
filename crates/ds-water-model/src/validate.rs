//! Structural validation: what makes a network unsolvable or meaningless.
//!
//! Validation reports every issue it finds instead of stopping at the first,
//! so an imported or drawn network can be repaired in one pass.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::WaterNetwork;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ValidationIssue {
    /// Stable machine code, e.g. `link_unknown_node`.
    pub code: String,
    /// Id of the element concerned, when there is one.
    pub element: Option<String>,
    pub message: String,
}

fn issue(code: &str, element: Option<&str>, message: String) -> ValidationIssue {
    ValidationIssue {
        code: code.to_owned(),
        element: element.map(str::to_owned),
        message,
    }
}

pub fn validate(network: &WaterNetwork) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let mut nodes: HashSet<&str> = HashSet::new();
    for (id, _) in network.node_refs() {
        if !nodes.insert(id) {
            issues.push(issue(
                "node_duplicate_id",
                Some(id),
                format!("node id {id} is used twice"),
            ));
        }
    }
    if network.reservoirs.is_empty() && network.tanks.is_empty() {
        issues.push(issue(
            "network_without_source",
            None,
            "the network has no reservoir or tank, so no head is fixed".into(),
        ));
    }
    let patterns: HashSet<&str> = network.patterns.iter().map(|p| p.id.as_str()).collect();
    let curves: HashMap<&str, usize> = network
        .curves
        .iter()
        .map(|c| (c.id.as_str(), c.points.len()))
        .collect();

    for j in &network.junctions {
        check_number(&mut issues, &j.id, "elevation_m", j.elevation_m);
        check_number(&mut issues, &j.id, "demand_lps", j.demand_lps);
        check_pattern(&mut issues, &patterns, &j.id, j.pattern.as_deref());
    }
    for r in &network.reservoirs {
        check_number(&mut issues, &r.id, "head_m", r.head_m);
        check_pattern(&mut issues, &patterns, &r.id, r.pattern.as_deref());
    }
    for t in &network.tanks {
        if !positive(t.diameter_m) {
            issues.push(issue(
                "tank_diameter",
                Some(&t.id),
                "tank diameter must be positive".into(),
            ));
        }
        if !(t.min_level_m <= t.initial_level_m && t.initial_level_m <= t.max_level_m) {
            issues.push(issue(
                "tank_levels",
                Some(&t.id),
                "tank levels must satisfy min ≤ initial ≤ max".into(),
            ));
        }
    }

    let mut links: HashSet<String> = HashSet::new();
    let mut check_link = |issues: &mut Vec<ValidationIssue>, id: &str, from: &str, to: &str| {
        if !links.insert(id.to_owned()) {
            issues.push(issue(
                "link_duplicate_id",
                Some(id),
                format!("link id {id} is used twice"),
            ));
        }
        for end in [from, to] {
            if !nodes.contains(end) {
                issues.push(issue(
                    "link_unknown_node",
                    Some(id),
                    format!("link {id} names node {end}, which does not exist"),
                ));
            }
        }
        if from == to {
            issues.push(issue(
                "link_self_loop",
                Some(id),
                format!("link {id} starts and ends at {from}"),
            ));
        }
    };
    for p in &network.pipes {
        check_link(&mut issues, &p.id, &p.from, &p.to);
        if !positive(p.length_m) {
            issues.push(issue(
                "pipe_length",
                Some(&p.id),
                "pipe length must be positive".into(),
            ));
        }
        if !positive(p.diameter_mm) {
            issues.push(issue(
                "pipe_diameter",
                Some(&p.id),
                "pipe diameter must be positive".into(),
            ));
        }
        if !positive(p.roughness) {
            issues.push(issue(
                "pipe_roughness",
                Some(&p.id),
                "pipe roughness must be positive".into(),
            ));
        }
    }
    for p in &network.pumps {
        check_link(&mut issues, &p.id, &p.from, &p.to);
        match curves.get(p.curve.as_str()) {
            None => issues.push(issue(
                "pump_unknown_curve",
                Some(&p.id),
                format!(
                    "pump {} names curve {}, which does not exist",
                    p.id, p.curve
                ),
            )),
            Some(&n) if n != 1 && n != 3 => issues.push(issue(
                "pump_curve_points",
                Some(&p.id),
                format!(
                    "pump curve {} has {n} points; one or three are supported",
                    p.curve
                ),
            )),
            Some(_) => {}
        }
        check_pattern(&mut issues, &patterns, &p.id, p.pattern.as_deref());
    }
    for v in &network.valves {
        check_link(&mut issues, &v.id, &v.from, &v.to);
        if !positive(v.diameter_mm) {
            issues.push(issue(
                "valve_diameter",
                Some(&v.id),
                "valve diameter must be positive".into(),
            ));
        }
    }
    issues
}

fn check_number(issues: &mut Vec<ValidationIssue>, id: &str, field: &str, value: f64) {
    if !value.is_finite() {
        issues.push(issue(
            "not_finite",
            Some(id),
            format!("{id}.{field} is not a finite number"),
        ));
    }
}

fn check_pattern(
    issues: &mut Vec<ValidationIssue>,
    patterns: &HashSet<&str>,
    id: &str,
    pattern: Option<&str>,
) {
    if let Some(p) = pattern
        && !patterns.contains(p)
    {
        issues.push(issue(
            "unknown_pattern",
            Some(id),
            format!("{id} names pattern {p}, which does not exist"),
        ));
    }
}

/// True for a strictly positive number; NaN is not positive.
fn positive(value: f64) -> bool {
    value > 0.0
}
