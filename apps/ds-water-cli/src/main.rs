//! `ds-water`: the headless Water engine.
//!
//! Every command reads a network (`.json` in the DS model, or EPANET `.inp`)
//! and prints one JSON document on stdout. The same operations back the
//! WASM unit and, through the Kernel, the `ds water` command chapter.

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
    /// Generate and solve a synthetic network, reporting time and size.
    Bench {
        #[arg(long, default_value_t = 100_000)]
        junctions: usize,
        #[arg(long, default_value_t = 1)]
        seed: u64,
    },
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
