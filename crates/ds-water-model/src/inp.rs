//! EPANET INP exchange.
//!
//! INP is the common text format of water-network software (EPANET, WaterGEMS
//! and most others import and export it). Reading converts every unit system
//! EPANET knows into this model's SI; writing always emits LPS / metres.
//!
//! Sections this model does not carry (controls, rules, energy, quality,
//! reactions) are skipped and named in [`InpImport::skipped_sections`], so an
//! import never drops content silently.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::{
    Curve, HeadlossFormula, Junction, LinkStatus, Pattern, Pipe, Pump, Reservoir, Tank, Valve,
    ValveKind, WaterNetwork,
};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum InpError {
    #[error("line {line}: {message}")]
    Line { line: usize, message: String },
    #[error("unsupported flow units {0}")]
    Units(String),
}

#[derive(Debug)]
pub struct InpImport {
    pub network: WaterNetwork,
    pub skipped_sections: Vec<String>,
    pub notes: Vec<String>,
}

/// Unit conversions from an INP file's declared flow units.
#[derive(Clone, Copy, Debug)]
struct UnitSystem {
    /// Flow → L/s.
    flow: f64,
    /// Length / elevation / head → m.
    length: f64,
    /// Diameter → mm.
    diameter: f64,
    /// Darcy-Weisbach roughness → mm.
    roughness_dw: f64,
}

impl UnitSystem {
    fn for_flow_units(name: &str) -> Result<Self, InpError> {
        const US_LEN: f64 = 0.3048;
        let us = |flow| UnitSystem {
            flow,
            length: US_LEN,
            diameter: 25.4,
            roughness_dw: 0.3048,
        };
        let si = |flow| UnitSystem {
            flow,
            length: 1.0,
            diameter: 1.0,
            roughness_dw: 1.0,
        };
        Ok(match name.to_ascii_uppercase().as_str() {
            "CFS" => us(28.316_846_6),
            "GPM" => us(0.063_090_196_4),
            "MGD" => us(43.812_636_4),
            "IMGD" => us(52.616_8),
            "AFD" => us(14.276_4),
            "LPS" => si(1.0),
            "LPM" => si(1.0 / 60.0),
            "MLD" => si(11.574_074_1),
            "CMH" => si(1.0 / 3.6),
            "CMD" => si(1.0 / 86.4),
            other => return Err(InpError::Units(other.to_owned())),
        })
    }
}

/// Parse INP text into a network.
pub fn read(text: &str) -> Result<InpImport, InpError> {
    // Two passes: OPTIONS fixes the unit system that every other section's
    // numbers are expressed in, and it may appear anywhere in the file.
    let lines: Vec<(usize, String, Vec<String>)> = text
        .lines()
        .enumerate()
        .map(|(i, raw)| {
            let content = raw.split(';').next().unwrap_or("").trim().to_owned();
            let tokens = content.split_whitespace().map(str::to_owned).collect();
            (i + 1, content, tokens)
        })
        .collect();

    let mut flow_units = "GPM".to_owned();
    let mut headloss = HeadlossFormula::HazenWilliams;
    let mut section = String::new();
    for (line, content, tokens) in &lines {
        if content.starts_with('[') {
            section = content.to_ascii_uppercase();
            continue;
        }
        if section == "[OPTIONS]" && tokens.len() >= 2 {
            match tokens[0].to_ascii_uppercase().as_str() {
                "UNITS" => flow_units = tokens[1].clone(),
                "HEADLOSS" => {
                    headloss = match tokens[1].to_ascii_uppercase().as_str() {
                        "H-W" => HeadlossFormula::HazenWilliams,
                        "D-W" => HeadlossFormula::DarcyWeisbach,
                        other => {
                            return Err(InpError::Line {
                                line: *line,
                                message: format!("headloss formula {other} is not supported"),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let units = UnitSystem::for_flow_units(&flow_units)?;

    let mut network = WaterNetwork::new("");
    network.options.headloss = headloss;
    let mut skipped: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut coordinates: HashMap<String, [f64; 2]> = HashMap::new();
    let mut vertices: HashMap<String, Vec<[f64; 2]>> = HashMap::new();
    let mut statuses: Vec<(String, String)> = Vec::new();
    let mut extra_demands: Vec<(String, f64, Option<String>)> = Vec::new();
    let mut title: Vec<String> = Vec::new();
    section.clear();

    for (line, content, t) in &lines {
        let line = *line;
        if content.is_empty() {
            continue;
        }
        if content.starts_with('[') {
            section = content.to_ascii_uppercase();
            continue;
        }
        let num = |i: usize| -> Result<f64, InpError> {
            t.get(i)
                .and_then(|s| s.parse::<f64>().ok())
                .ok_or_else(|| InpError::Line {
                    line,
                    message: format!("expected a number in field {}", i + 1),
                })
        };
        let need = |n: usize| -> Result<(), InpError> {
            if t.len() < n {
                Err(InpError::Line {
                    line,
                    message: format!("expected at least {n} fields"),
                })
            } else {
                Ok(())
            }
        };
        match section.as_str() {
            "[TITLE]" => title.push(content.clone()),
            "[JUNCTIONS]" => {
                need(2)?;
                network.junctions.push(Junction {
                    id: t[0].clone(),
                    elevation_m: num(1)? * units.length,
                    demand_lps: if t.len() > 2 {
                        num(2)? * units.flow
                    } else {
                        0.0
                    },
                    pattern: t.get(3).cloned(),
                    x: None,
                    y: None,
                });
            }
            "[RESERVOIRS]" => {
                need(2)?;
                network.reservoirs.push(Reservoir {
                    id: t[0].clone(),
                    head_m: num(1)? * units.length,
                    pattern: t.get(2).cloned(),
                    x: None,
                    y: None,
                });
            }
            "[TANKS]" => {
                need(6)?;
                network.tanks.push(Tank {
                    id: t[0].clone(),
                    elevation_m: num(1)? * units.length,
                    initial_level_m: num(2)? * units.length,
                    min_level_m: num(3)? * units.length,
                    max_level_m: num(4)? * units.length,
                    diameter_m: num(5)? * units.length,
                    x: None,
                    y: None,
                });
            }
            "[PIPES]" => {
                need(6)?;
                let roughness = num(5)?;
                let status = match t.get(7).map(|s| s.to_ascii_uppercase()).as_deref() {
                    Some("CLOSED") => LinkStatus::Closed,
                    Some("CV") => LinkStatus::CheckValve,
                    _ => LinkStatus::Open,
                };
                network.pipes.push(Pipe {
                    id: t[0].clone(),
                    from: t[1].clone(),
                    to: t[2].clone(),
                    length_m: num(3)? * units.length,
                    diameter_mm: num(4)? * units.diameter,
                    roughness: match headloss {
                        HeadlossFormula::HazenWilliams => roughness,
                        // INP's D-W roughness is millifeet (US) or mm (SI).
                        HeadlossFormula::DarcyWeisbach => roughness * units.roughness_dw,
                    },
                    minor_loss: if t.len() > 6 { num(6)? } else { 0.0 },
                    status,
                    ..Pipe::default()
                });
            }
            "[PUMPS]" => {
                need(5)?;
                let mut pump = Pump {
                    id: t[0].clone(),
                    from: t[1].clone(),
                    to: t[2].clone(),
                    speed: 1.0,
                    ..Pump::default()
                };
                let mut i = 3;
                while i + 1 < t.len() {
                    match t[i].to_ascii_uppercase().as_str() {
                        "HEAD" => pump.curve = t[i + 1].clone(),
                        "SPEED" => pump.speed = num(i + 1)?,
                        "PATTERN" => pump.pattern = Some(t[i + 1].clone()),
                        "POWER" => notes.push(format!(
                            "pump {}: constant-power pumps are not modelled; give it a HEAD curve",
                            pump.id
                        )),
                        _ => {}
                    }
                    i += 2;
                }
                network.pumps.push(pump);
            }
            "[VALVES]" => {
                need(6)?;
                let kind = match t[4].to_ascii_uppercase().as_str() {
                    "PRV" => ValveKind::Prv,
                    "PSV" => ValveKind::Psv,
                    "FCV" => ValveKind::Fcv,
                    "TCV" => ValveKind::Tcv,
                    other => {
                        notes.push(format!("valve {}: type {other} imported as TCV", t[0]));
                        ValveKind::Tcv
                    }
                };
                let raw = num(5)?;
                let setting = match kind {
                    ValveKind::Prv | ValveKind::Psv => {
                        raw * units.length * pressure_to_head(&flow_units)
                    }
                    ValveKind::Fcv => raw * units.flow,
                    ValveKind::Tcv => raw,
                };
                network.valves.push(Valve {
                    id: t[0].clone(),
                    from: t[1].clone(),
                    to: t[2].clone(),
                    diameter_mm: num(3)? * units.diameter,
                    kind,
                    setting,
                    minor_loss: if t.len() > 6 { num(6)? } else { 0.0 },
                    status: None,
                });
            }
            "[DEMANDS]" => {
                need(2)?;
                extra_demands.push((t[0].clone(), num(1)? * units.flow, t.get(2).cloned()));
            }
            "[STATUS]" => {
                need(2)?;
                statuses.push((t[0].clone(), t[1].to_ascii_uppercase()));
            }
            "[PATTERNS]" => {
                need(2)?;
                let values: Result<Vec<f64>, _> = (1..t.len()).map(num).collect();
                match network.patterns.iter_mut().find(|p| p.id == t[0]) {
                    Some(p) => p.multipliers.extend(values?),
                    None => network.patterns.push(Pattern {
                        id: t[0].clone(),
                        multipliers: values?,
                    }),
                }
            }
            "[CURVES]" => {
                need(3)?;
                let point = [num(1)? * units.flow, num(2)? * units.length];
                match network.curves.iter_mut().find(|c| c.id == t[0]) {
                    Some(c) => c.points.push(point),
                    None => network.curves.push(Curve {
                        id: t[0].clone(),
                        points: vec![point],
                    }),
                }
            }
            "[COORDINATES]" => {
                need(3)?;
                coordinates.insert(t[0].clone(), [num(1)?, num(2)?]);
            }
            "[VERTICES]" => {
                need(3)?;
                vertices
                    .entry(t[0].clone())
                    .or_default()
                    .push([num(1)?, num(2)?]);
            }
            "[TIMES]" => read_time_option(&mut network, t),
            "[OPTIONS]" => read_option(&mut network, t, line)?,
            "[END]" => {}
            other => {
                let name = other.trim_matches(|c| c == '[' || c == ']').to_owned();
                if !skipped.contains(&name) {
                    skipped.push(name);
                }
            }
        }
    }

    network.name = title.first().cloned().unwrap_or_default();
    // [DEMANDS] replaces the junction's base demand when present (EPANET).
    let mut replaced: Vec<String> = Vec::new();
    for (id, demand, pattern) in extra_demands {
        if let Some(j) = network.junctions.iter_mut().find(|j| j.id == id) {
            if !replaced.contains(&id) {
                j.demand_lps = 0.0;
                replaced.push(id.clone());
            }
            j.demand_lps += demand;
            if j.pattern.is_none() {
                j.pattern = pattern;
            }
        }
    }
    for (id, status) in statuses {
        let parsed = match status.as_str() {
            "CLOSED" => Some(LinkStatus::Closed),
            "OPEN" => Some(LinkStatus::Open),
            _ => None,
        };
        if let Some(pipe) = network.pipes.iter_mut().find(|p| p.id == id) {
            if let Some(s) = parsed {
                pipe.status = s;
            }
        } else if let Some(pump) = network.pumps.iter_mut().find(|p| p.id == id) {
            match parsed {
                Some(s) => pump.status = s,
                None => {
                    if let Ok(speed) = status.parse::<f64>() {
                        pump.speed = speed;
                    }
                }
            }
        } else if let Some(valve) = network.valves.iter_mut().find(|v| v.id == id) {
            valve.status = parsed;
        }
    }
    for j in &mut network.junctions {
        if let Some([x, y]) = coordinates.get(&j.id) {
            (j.x, j.y) = (Some(*x), Some(*y));
        }
    }
    for r in &mut network.reservoirs {
        if let Some([x, y]) = coordinates.get(&r.id) {
            (r.x, r.y) = (Some(*x), Some(*y));
        }
    }
    for tk in &mut network.tanks {
        if let Some([x, y]) = coordinates.get(&tk.id) {
            (tk.x, tk.y) = (Some(*x), Some(*y));
        }
    }
    for p in &mut network.pipes {
        if let Some(v) = vertices.remove(&p.id) {
            p.vertices = v;
        }
    }
    Ok(InpImport {
        network,
        skipped_sections: skipped,
        notes,
    })
}

/// EPANET states PRV/PSV settings as pressure in psi (US) or metres (SI);
/// `units.length` already turned feet into metres, so psi needs feet first.
fn pressure_to_head(flow_units: &str) -> f64 {
    match flow_units.to_ascii_uppercase().as_str() {
        "CFS" | "GPM" | "MGD" | "IMGD" | "AFD" => 2.306_659,
        _ => 1.0,
    }
}

fn read_option(network: &mut WaterNetwork, t: &[String], line: usize) -> Result<(), InpError> {
    let key = t
        .first()
        .map(|s| s.to_ascii_uppercase())
        .unwrap_or_default();
    let value = |i: usize| t.get(i).and_then(|s| s.parse::<f64>().ok());
    match key.as_str() {
        "ACCURACY" => {
            if let Some(v) = value(1) {
                network.options.accuracy = v;
            }
        }
        "TRIALS" => {
            if let Some(v) = value(1) {
                network.options.max_iterations = v as u32;
            }
        }
        "DEMAND"
            if t.get(1)
                .is_some_and(|s| s.eq_ignore_ascii_case("MULTIPLIER")) =>
        {
            network.options.demand_multiplier = value(2).ok_or_else(|| InpError::Line {
                line,
                message: "DEMAND MULTIPLIER needs a number".into(),
            })?;
        }
        _ => {}
    }
    Ok(())
}

fn read_time_option(network: &mut WaterNetwork, t: &[String]) {
    let key: Vec<String> = t.iter().map(|s| s.to_ascii_uppercase()).collect();
    let (field, rest) = match key.first().map(String::as_str) {
        Some("DURATION") => ("duration", &t[1..]),
        Some("HYDRAULIC") if key.get(1).is_some_and(|s| s == "TIMESTEP") => ("hydraulic", &t[2..]),
        Some("PATTERN") if key.get(1).is_some_and(|s| s == "TIMESTEP") => ("pattern", &t[2..]),
        _ => return,
    };
    let Some(seconds) = parse_clock(rest) else {
        return;
    };
    match field {
        "duration" => network.options.duration_s = seconds,
        "hydraulic" => network.options.hydraulic_step_s = seconds,
        _ => network.options.pattern_step_s = seconds,
    }
}

/// `hh:mm[:ss]`, or a number with an optional unit (hours by default).
fn parse_clock(tokens: &[String]) -> Option<u64> {
    let first = tokens.first()?;
    if first.contains(':') {
        let parts: Vec<f64> = first
            .split(':')
            .map(|p| p.parse().ok())
            .collect::<Option<_>>()?;
        let seconds = parts
            .iter()
            .zip([3600.0, 60.0, 1.0])
            .map(|(v, s)| v * s)
            .sum::<f64>();
        return Some(seconds.round() as u64);
    }
    let value: f64 = first.parse().ok()?;
    let scale = match tokens.get(1).map(|s| s.to_ascii_uppercase()).as_deref() {
        Some("SEC" | "SECONDS") => 1.0,
        Some("MIN" | "MINUTES") => 60.0,
        Some("DAY" | "DAYS") => 86_400.0,
        _ => 3600.0,
    };
    Some((value * scale).round() as u64)
}

/// Write the network as INP text in LPS / metres.
pub fn write(network: &WaterNetwork) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "[TITLE]\n{}\n", network.name);
    let _ = writeln!(s, "[JUNCTIONS]\n;ID\tElev\tDemand\tPattern");
    for j in &network.junctions {
        let _ = writeln!(
            s,
            "{}\t{}\t{}\t{}",
            j.id,
            j.elevation_m,
            j.demand_lps,
            j.pattern.as_deref().unwrap_or("")
        );
    }
    let _ = writeln!(s, "\n[RESERVOIRS]\n;ID\tHead\tPattern");
    for r in &network.reservoirs {
        let _ = writeln!(
            s,
            "{}\t{}\t{}",
            r.id,
            r.head_m,
            r.pattern.as_deref().unwrap_or("")
        );
    }
    let _ = writeln!(
        s,
        "\n[TANKS]\n;ID\tElev\tInitLvl\tMinLvl\tMaxLvl\tDiam\tMinVol"
    );
    for t in &network.tanks {
        let _ = writeln!(
            s,
            "{}\t{}\t{}\t{}\t{}\t{}\t0",
            t.id, t.elevation_m, t.initial_level_m, t.min_level_m, t.max_level_m, t.diameter_m
        );
    }
    let _ = writeln!(
        s,
        "\n[PIPES]\n;ID\tNode1\tNode2\tLength\tDiam\tRough\tMLoss\tStatus"
    );
    for p in &network.pipes {
        let status = match p.status {
            LinkStatus::Open => "Open",
            LinkStatus::Closed => "Closed",
            LinkStatus::CheckValve => "CV",
        };
        let _ = writeln!(
            s,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            p.id, p.from, p.to, p.length_m, p.diameter_mm, p.roughness, p.minor_loss, status
        );
    }
    let _ = writeln!(s, "\n[PUMPS]\n;ID\tNode1\tNode2\tParameters");
    for p in &network.pumps {
        let _ = write!(
            s,
            "{}\t{}\t{}\tHEAD {}\tSPEED {}",
            p.id, p.from, p.to, p.curve, p.speed
        );
        if let Some(pat) = &p.pattern {
            let _ = write!(s, "\tPATTERN {pat}");
        }
        s.push('\n');
    }
    let _ = writeln!(
        s,
        "\n[VALVES]\n;ID\tNode1\tNode2\tDiam\tType\tSetting\tMLoss"
    );
    for v in &network.valves {
        let kind = match v.kind {
            ValveKind::Prv => "PRV",
            ValveKind::Psv => "PSV",
            ValveKind::Fcv => "FCV",
            ValveKind::Tcv => "TCV",
        };
        let _ = writeln!(
            s,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            v.id, v.from, v.to, v.diameter_mm, kind, v.setting, v.minor_loss
        );
    }
    let closed: Vec<&str> = network
        .pumps
        .iter()
        .filter(|p| p.status == LinkStatus::Closed)
        .map(|p| p.id.as_str())
        .chain(
            network
                .valves
                .iter()
                .filter(|v| v.status == Some(LinkStatus::Closed))
                .map(|v| v.id.as_str()),
        )
        .collect();
    let _ = writeln!(s, "\n[STATUS]");
    for id in closed {
        let _ = writeln!(s, "{id}\tClosed");
    }
    let _ = writeln!(s, "\n[PATTERNS]");
    for p in &network.patterns {
        for chunk in p.multipliers.chunks(6) {
            let values: Vec<String> = chunk.iter().map(f64::to_string).collect();
            let _ = writeln!(s, "{}\t{}", p.id, values.join("\t"));
        }
    }
    let _ = writeln!(s, "\n[CURVES]");
    for c in &network.curves {
        for [x, y] in &c.points {
            let _ = writeln!(s, "{}\t{}\t{}", c.id, x, y);
        }
    }
    let o = &network.options;
    let headloss = match o.headloss {
        HeadlossFormula::HazenWilliams => "H-W",
        HeadlossFormula::DarcyWeisbach => "D-W",
    };
    let _ = writeln!(
        s,
        "\n[OPTIONS]\nUnits\tLPS\nHeadloss\t{headloss}\nAccuracy\t{}\nTrials\t{}\nDemand Multiplier\t{}",
        o.accuracy, o.max_iterations, o.demand_multiplier
    );
    let clock = |secs: u64| format!("{}:{:02}", secs / 3600, (secs % 3600) / 60);
    let _ = writeln!(
        s,
        "\n[TIMES]\nDuration\t{}\nHydraulic Timestep\t{}\nPattern Timestep\t{}",
        clock(o.duration_s),
        clock(o.hydraulic_step_s),
        clock(o.pattern_step_s)
    );
    let _ = writeln!(s, "\n[COORDINATES]");
    for (id, node) in network.node_refs() {
        if let Some([x, y]) = network.node_xy(node) {
            let _ = writeln!(s, "{id}\t{x}\t{y}");
        }
    }
    let _ = writeln!(s, "\n[VERTICES]");
    for p in &network.pipes {
        for [x, y] in &p.vertices {
            let _ = writeln!(s, "{}\t{}\t{}", p.id, x, y);
        }
    }
    s.push_str("\n[END]\n");
    s
}
