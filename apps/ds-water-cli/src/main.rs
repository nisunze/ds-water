//! `ds-water`: the headless Water engine.
//!
//! Every command reads a network (`.json` in the DS model, or EPANET `.inp`)
//! and prints one JSON document on stdout. The same operations back the
//! WASM unit and, through the Kernel, the `ds water` command chapter.

mod surface;

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use ds_water_hydraulics::{Hydraulics, Snapshot};
use ds_water_model::{WaterNetwork, inp};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    name = "ds-water",
    version,
    about = "Data Solutions Water engine (headless)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check a network for structural problems.
    Validate { network: PathBuf },
    /// Solve a snapshot (or the extended period with --eps).
    Simulate {
        network: PathBuf,
        /// Run the extended period from the network's options.
        #[arg(long)]
        eps: bool,
        /// Write full per-element results here as JSON.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Convert between EPANET INP and the DS Water JSON model.
    Convert { input: PathBuf, output: PathBuf },
    /// Generate a deterministic synthetic network.
    Synth {
        #[arg(long, default_value_t = 10_000)]
        junctions: usize,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        #[arg(long, default_value_t = 160.0)]
        extent_km: f64,
        #[arg(long)]
        out: PathBuf,
    },
    /// Lay out a network from GeoJSON features (drawn pipes, roads or points).
    Layout {
        features: PathBuf,
        /// LayoutOptions as JSON.
        #[arg(long)]
        options: Option<PathBuf>,
        /// Ground as an ESRI ASCII grid in the features' coordinates.
        #[arg(long)]
        surface: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Size every pipe from the catalogue to the design criteria.
    Design {
        network: PathBuf,
        /// DesignCriteria as JSON.
        #[arg(long)]
        criteria: Option<PathBuf>,
        /// Catalogue as JSON (default: the built-in indicative catalogue).
        #[arg(long)]
        catalogue: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
        /// Full design report (pipe schedule, BOQ, findings).
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// Profile one path: a layout path label, or explicit pipes in order.
    Profile {
        network: PathBuf,
        #[arg(long, default_value = "01")]
        path: String,
        #[arg(long, value_delimiter = ',')]
        pipes: Vec<String>,
        #[arg(long)]
        surface: Option<PathBuf>,
        /// ProfileOptions as JSON.
        #[arg(long)]
        options: Option<PathBuf>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Draw the sheet set of a network as it is: plan, profiles, schedules.
    Draw {
        network: PathBuf,
        #[arg(long)]
        out_dir: PathBuf,
        #[command(flatten)]
        sheets: SheetArgs,
    },
    /// Everything from features: layout, design, profiles and drawings.
    Scheme {
        features: PathBuf,
        #[arg(long)]
        out_dir: PathBuf,
        #[arg(long)]
        options: Option<PathBuf>,
        #[arg(long)]
        criteria: Option<PathBuf>,
        #[command(flatten)]
        sheets: SheetArgs,
    },
    /// Generate and solve a synthetic network, reporting time and size.
    Bench {
        #[arg(long, default_value_t = 100_000)]
        junctions: usize,
        #[arg(long, default_value_t = 1)]
        seed: u64,
    },
}

#[derive(clap::Args)]
struct SheetArgs {
    /// Profile at most this many paths, longest first.
    #[arg(long, default_value_t = 3)]
    paths: usize,
    /// Skip paths shorter than this, m.
    #[arg(long, default_value_t = 200.0)]
    min_path_m: f64,
    #[arg(long)]
    surface: Option<PathBuf>,
    /// TitleInfo as JSON (project, client, location, drawing_prefix, ...).
    #[arg(long)]
    title: Option<PathBuf>,
}

fn main() {
    let cli = Cli::parse();
    match run(cli.command) {
        Ok(value) => println!("{}", serde_json::to_string_pretty(&value).expect("json")),
        Err(error) => {
            println!("{}", json!({ "ok": false, "error": format!("{error:#}") }));
            std::process::exit(1);
        }
    }
}

fn read_network(path: &Path) -> Result<WaterNetwork> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    if is_inp(path) {
        let import = inp::read(&text)?;
        for note in &import.notes {
            eprintln!("note: {note}");
        }
        if !import.skipped_sections.is_empty() {
            eprintln!(
                "note: sections not modelled: {}",
                import.skipped_sections.join(", ")
            );
        }
        Ok(import.network)
    } else {
        Ok(WaterNetwork::from_json(&text)?)
    }
}

fn is_inp(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("inp"))
}

fn run(command: Command) -> Result<Value> {
    match command {
        Command::Validate { network } => {
            let n = read_network(&network)?;
            let issues = ds_water_model::validate(&n);
            Ok(json!({
                "ok": issues.is_empty(),
                "junctions": n.junctions.len(),
                "links": n.link_count(),
                "issues": issues,
            }))
        }
        Command::Simulate { network, eps, out } => {
            let n = read_network(&network)?;
            let started = Instant::now();
            let mut h = Hydraulics::new(&n).map_err(|e| anyhow::anyhow!("{e}"))?;
            let compiled = started.elapsed();
            let snapshots = if eps {
                h.run(&n)
            } else {
                vec![h.solve(0, &Hydraulics::initial_tank_levels(&n))]
            };
            let solved = started.elapsed() - compiled;
            if let Some(out) = out {
                std::fs::write(&out, serde_json::to_vec(&results_document(&n, &snapshots))?)?;
            }
            Ok(json!({
                "ok": snapshots.iter().all(|s| s.converged),
                "compile_ms": compiled.as_secs_f64() * 1e3,
                "solve_ms": solved.as_secs_f64() * 1e3,
                "steps": snapshots.iter().map(|s| summary(&n, s)).collect::<Vec<_>>(),
            }))
        }
        Command::Convert { input, output } => {
            let n = read_network(&input)?;
            let text = if is_inp(&output) {
                inp::write(&n)
            } else {
                n.to_json()
            };
            std::fs::write(&output, text)?;
            Ok(
                json!({ "ok": true, "junctions": n.junctions.len(), "links": n.link_count(), "out": output }),
            )
        }
        Command::Synth {
            junctions,
            seed,
            extent_km,
            out,
        } => {
            let n = ds_water_synth::generate(&ds_water_synth::Spec {
                junctions,
                seed,
                extent_km,
            });
            std::fs::write(
                &out,
                if is_inp(&out) {
                    inp::write(&n)
                } else {
                    serde_json::to_string(&n)?
                },
            )?;
            Ok(
                json!({ "ok": true, "name": n.name, "junctions": n.junctions.len(), "links": n.link_count(), "out": out }),
            )
        }
        Command::Layout { features, options, surface, out } => {
            let opts: ds_water_layout::LayoutOptions = read_json_or_default(options.as_deref())?;
            let grid = surface.as_deref().map(surface::AsciiGrid::read).transpose()?;
            let fc: Value = serde_json::from_str(&std::fs::read_to_string(&features)?)?;
            let at = |p: [f64; 2]| grid.as_ref().and_then(|g| g.at(p));
            let r = ds_water_layout::layout(&fc, &opts, grid.as_ref().map(|_| &at as &dyn Fn([f64; 2]) -> Option<f64>))?;
            std::fs::write(&out, r.network.to_json())?;
            Ok(json!({
                "ok": true,
                "mode": r.mode,
                "junctions": r.network.junctions.len(),
                "pipes": r.network.pipes.len(),
                "length_m": r.network.pipes.iter().map(|p| p.length_m).sum::<f64>(),
                "demand_lps": r.network.total_base_demand_lps(),
                "paths": r.paths.iter().take(10).map(|p| json!({ "label": p.label, "root": p.root, "tip": p.tip, "length_m": p.length_m })).collect::<Vec<_>>(),
                "unserved": r.unserved,
                "notes": r.notes,
                "out": out,
            }))
        }
        Command::Design { network, criteria, catalogue, out, report } => {
            let n = read_network(&network)?;
            let criteria: ds_water_design::DesignCriteria = read_json_or_default(criteria.as_deref())?;
            let catalogue = match catalogue {
                Some(p) => serde_json::from_str(&std::fs::read_to_string(p)?)?,
                None => ds_water_design::Catalogue::standard(),
            };
            let r = ds_water_design::design(&n, &catalogue, &criteria)?;
            std::fs::write(&out, r.network.to_json())?;
            if let Some(report) = report {
                std::fs::write(report, serde_json::to_vec_pretty(&r)?)?;
            }
            Ok(design_summary(&r))
        }
        Command::Profile { network, path, pipes, surface, options, out } => {
            let n = read_network(&network)?;
            let opts: ds_water_profile::ProfileOptions = read_json_or_default(options.as_deref())?;
            let grid = surface.as_deref().map(surface::AsciiGrid::read).transpose()?;
            let pipes = if pipes.is_empty() { path_pipes(&n, &path)? } else { pipes };
            let p = run_profile(&n, &pipes, grid.as_ref(), &opts)?;
            if let Some(out) = &out {
                std::fs::write(out, serde_json::to_vec_pretty(&p)?)?;
            }
            Ok(json!({
                "ok": true,
                "length_m": p.length_m,
                "fittings": p.fittings,
                "findings": p.findings,
                "ground": p.ground_source,
                "out": out,
            }))
        }
        Command::Draw { network, out_dir, sheets } => {
            let n = read_network(&network)?;
            let boq = boq_as_is(&n);
            draw_set(&n, &boq, &ds_water_design::DesignCriteria::default(), &out_dir, &sheets)
        }
        Command::Scheme { features, out_dir, options, criteria, sheets } => {
            std::fs::create_dir_all(&out_dir)?;
            let opts: ds_water_layout::LayoutOptions = read_json_or_default(options.as_deref())?;
            let criteria: ds_water_design::DesignCriteria = read_json_or_default(criteria.as_deref())?;
            let grid = sheets.surface.as_deref().map(surface::AsciiGrid::read).transpose()?;
            let fc: Value = serde_json::from_str(&std::fs::read_to_string(&features)?)?;
            let at = |p: [f64; 2]| grid.as_ref().and_then(|g| g.at(p));
            let laid = ds_water_layout::layout(&fc, &opts, grid.as_ref().map(|_| &at as &dyn Fn([f64; 2]) -> Option<f64>))?;
            std::fs::write(out_dir.join("layout.json"), laid.network.to_json())?;
            let designed = ds_water_design::design(&laid.network, &ds_water_design::Catalogue::standard(), &criteria)?;
            std::fs::write(out_dir.join("network.json"), designed.network.to_json())?;
            std::fs::write(out_dir.join("network.inp"), inp::write(&designed.network))?;
            std::fs::write(out_dir.join("design.json"), serde_json::to_vec_pretty(&designed)?)?;
            let drawings = draw_set(&designed.network, &designed.boq, &criteria, &out_dir, &sheets)?;
            Ok(json!({
                "ok": true,
                "layout": { "mode": laid.mode, "unserved": laid.unserved, "notes": laid.notes },
                "design": design_summary(&designed),
                "drawings": drawings,
                "out_dir": out_dir,
            }))
        }
        Command::Bench { junctions, seed } => {
            let t0 = Instant::now();
            let n = ds_water_synth::generate(&ds_water_synth::Spec {
                junctions,
                seed,
                ..Default::default()
            });
            let generated = t0.elapsed();
            let t1 = Instant::now();
            let mut h = Hydraulics::new(&n).map_err(|e| anyhow::anyhow!("{e}"))?;
            let compiled = t1.elapsed();
            let (nj, nl, nnz) = h.size();
            let t2 = Instant::now();
            let s = h.solve(0, &[]);
            let solved = t2.elapsed();
            if !s.converged {
                bail!("benchmark did not converge: {:?}", s.warnings);
            }
            Ok(json!({
                "ok": true,
                "junctions": nj,
                "links": nl,
                "factor_nonzeros": nnz,
                "fill_ratio": nnz as f64 / nl.max(1) as f64,
                "generate_ms": generated.as_secs_f64() * 1e3,
                "compile_ms": compiled.as_secs_f64() * 1e3,
                "solve_ms": solved.as_secs_f64() * 1e3,
                "summary": summary(&n, &s),
            }))
        }
    }
}

fn summary(n: &WaterNetwork, s: &Snapshot) -> Value {
    let nj = n.junctions.len();
    let connected: Vec<usize> = (0..nj).filter(|j| !s.disconnected.contains(j)).collect();
    let extreme = |better: fn(f64, f64) -> bool| {
        connected
            .iter()
            .copied()
            .fold(None::<usize>, |best, j| match best {
                Some(b) if !better(s.pressure_m[j], s.pressure_m[b]) => Some(b),
                _ => Some(j),
            })
    };
    let low = extreme(|a, b| a < b);
    let high = extreme(|a, b| a > b);
    let below = connected
        .iter()
        .filter(|&&j| s.pressure_m[j] < n.options.required_pressure_m)
        .count();
    let max_velocity = (0..n.pipes.len())
        .map(|k| s.velocity_ms[k])
        .fold(0.0, f64::max);
    json!({
        "time_s": s.time_s,
        "converged": s.converged,
        "iterations": s.iterations,
        "relative_change": s.relative_change,
        "demand_lps": s.delivered_lps.iter().sum::<f64>(),
        "min_pressure": low.map(|j| json!({ "junction": n.junctions[j].id, "m": s.pressure_m[j] })),
        "max_pressure": high.map(|j| json!({ "junction": n.junctions[j].id, "m": s.pressure_m[j] })),
        "junctions_below_required": below,
        "max_pipe_velocity_ms": max_velocity,
        "disconnected": s.disconnected.len(),
        "warnings": s.warnings,
    })
}

/// Per-element results keyed by id, for drawings, tables and the UI.
fn results_document(n: &WaterNetwork, snapshots: &[Snapshot]) -> Value {
    let node_ids: Vec<&str> = n.node_refs().map(|(id, _)| id).collect();
    let link_ids: Vec<&str> = n
        .pipes
        .iter()
        .map(|p| p.id.as_str())
        .chain(n.pumps.iter().map(|p| p.id.as_str()))
        .chain(n.valves.iter().map(|v| v.id.as_str()))
        .collect();
    json!({
        "schema": "ds.water.results/v1",
        "network": n.name,
        "node_ids": node_ids,
        "link_ids": link_ids,
        "steps": snapshots,
    })
}

fn read_json_or_default<T: serde::de::DeserializeOwned + Default>(path: Option<&Path>) -> Result<T> {
    match path {
        Some(p) => Ok(serde_json::from_str(&std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?)?),
        None => Ok(T::default()),
    }
}

fn design_summary(r: &ds_water_design::DesignResult) -> Value {
    json!({
        "ok": r.infeasible.is_empty(),
        "total_cost": r.total_cost,
        "boq": r.boq,
        "min_pressure": r.min_pressure,
        "infeasible": r.infeasible.len(),
        "over_pressure": r.over_pressure.len(),
        "low_velocity_pipes": r.low_velocity_pipes.len(),
        "simulations": r.simulations,
        "upgrades": r.upgrades,
        "warnings": r.warnings,
    })
}

fn path_pipes(n: &WaterNetwork, label: &str) -> Result<Vec<String>> {
    let paths = ds_water_layout::orient(n)?;
    paths
        .into_iter()
        .find(|p| p.label == label)
        .map(|p| p.pipes)
        .with_context(|| format!("no path {label}"))
}

fn run_profile(
    n: &WaterNetwork,
    pipes: &[String],
    grid: Option<&surface::AsciiGrid>,
    opts: &ds_water_profile::ProfileOptions,
) -> Result<ds_water_profile::ProfileResult> {
    let peak = Hydraulics::new(n).map_err(|e| anyhow::anyhow!("{e}"))?.solve(0, &Hydraulics::initial_tank_levels(n));
    let mut still_net = n.clone();
    still_net.options.demand_multiplier = 0.0;
    let still = Hydraulics::new(&still_net).map_err(|e| anyhow::anyhow!("{e}"))?.solve(0, &Hydraulics::initial_tank_levels(n));
    let at = |p: [f64; 2]| grid.and_then(|g| g.at(p));
    Ok(ds_water_profile::profile(n, pipes, &peak, &still, grid.map(|_| &at as &dyn Fn([f64; 2]) -> Option<f64>), opts)?)
}

/// Quantities of a network drawn as it is (no design run, no costs).
fn boq_as_is(n: &WaterNetwork) -> Vec<ds_water_design::BoqLine> {
    let mut out: Vec<ds_water_design::BoqLine> = Vec::new();
    for p in &n.pipes {
        let material = p.material.clone().unwrap_or_else(|| "pipe".into());
        let pn = p.pressure_class_bar.unwrap_or(0.0);
        match out.iter_mut().find(|b| b.material == material && b.nominal_mm == p.diameter_mm && b.pn_bar == pn) {
            Some(b) => b.length_m += p.length_m,
            None => out.push(ds_water_design::BoqLine { material, nominal_mm: p.diameter_mm, pn_bar: pn, length_m: p.length_m, cost: 0.0 }),
        }
    }
    out
}

fn draw_set(
    n: &WaterNetwork,
    boq: &[ds_water_design::BoqLine],
    criteria: &ds_water_design::DesignCriteria,
    out_dir: &Path,
    args: &SheetArgs,
) -> Result<Value> {
    use ds_water_drawing::{DrawingOptions, TitleInfo, network_plan_sheet, plan_profile_sheets, schedules_sheet};
    std::fs::create_dir_all(out_dir)?;
    let mut info: TitleInfo = read_json_or_default(args.title.as_deref())?;
    if info.project.is_empty() {
        info.project = n.name.clone();
    }
    if info.drawing_prefix.is_empty() {
        info.drawing_prefix = "WS".into();
    }
    let opts = DrawingOptions::default();
    let grid = args.surface.as_deref().map(surface::AsciiGrid::read).transpose()?;
    let profile_opts = ds_water_profile::ProfileOptions {
        min_pressure_m: criteria.min_pressure_m,
        max_static_pressure_m: criteria.max_static_pressure_m,
        ..Default::default()
    };
    let mut paths = ds_water_layout::orient(n)?;
    paths.retain(|p| p.length_m >= args.min_path_m);
    paths.sort_by(|a, b| b.length_m.total_cmp(&a.length_m));
    paths.truncate(args.paths);
    let mut sheets = vec![network_plan_sheet(n, &info, &opts, &format!("{}-001", info.drawing_prefix))];
    let mut profiles = Vec::new();
    for path in &paths {
        let p = run_profile(n, &path.pipes, grid.as_ref(), &profile_opts)?;
        sheets.extend(plan_profile_sheets(n, &p, &path.label, &info, &opts, sheets.len() + 1));
        profiles.push((path.label.clone(), p));
    }
    let refs: Vec<(String, &ds_water_profile::ProfileResult)> = profiles.iter().map(|(l, p)| (l.clone(), p)).collect();
    sheets.push(schedules_sheet(boq, &refs, criteria, &info, &opts, &format!("{}-{:03}", info.drawing_prefix, sheets.len() + 1)));
    for s in &sheets {
        std::fs::write(out_dir.join(format!("{}.svg", s.number)), &s.svg)?;
    }
    for (label, p) in &profiles {
        std::fs::write(out_dir.join(format!("profile-{label}.json")), serde_json::to_vec_pretty(p)?)?;
    }
    Ok(json!({
        "sheets": sheets.iter().map(|s| json!({ "number": s.number, "title": s.title })).collect::<Vec<_>>(),
        "profiles": profiles.iter().map(|(l, p)| json!({ "path": l, "length_m": p.length_m, "fittings": p.fittings.len(), "findings": p.findings.len() })).collect::<Vec<_>>(),
    }))
}
