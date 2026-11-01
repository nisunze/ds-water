//! `ds-water`: the headless Water engine.
//!
//! Each command reads files, builds one request and calls the shared
//! command surface (`ds_water_ops`), the same one the WASM unit and the
//! Kernel use; then it writes the answer. Networks are `.json` in the DS
//! model or EPANET `.inp`.

mod surface;

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use ds_water_hydraulics::Hydraulics;
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
    /// The engine's identity, as Core admits an installed component by it.
    #[command(name = "build-info")]
    BuildInfo,
    /// Run one operation on the JSON request read from stdin (Core's door:
    /// the same `{operation, request}` the WASM unit takes).
    Invoke { operation: String },
    /// Check a network for structural problems.
    Validate { network: PathBuf },
    /// Solve a snapshot (or the extended period with --eps).
    Simulate {
        network: PathBuf,
        #[arg(long)]
        eps: bool,
        /// Write full per-element results here as JSON.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Convert between EPANET INP and the DS Water JSON model.
    Convert { input: PathBuf, output: PathBuf },
    /// Lay out a network from GeoJSON features (drawn pipes, roads or points).
    Layout {
        features: PathBuf,
        /// LayoutOptions as JSON.
        #[arg(long)]
        options: Option<PathBuf>,
        /// Fixture ground: an ESRI ASCII grid in the features' coordinates.
        #[arg(long)]
        surface: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Zone and size every pipe from the catalogue to the design criteria.
    Design {
        network: PathBuf,
        #[arg(long)]
        criteria: Option<PathBuf>,
        #[arg(long)]
        catalogue: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
        /// Full design report (pipe schedule, BOQ, findings).
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// Profile one path: a layout path label (the trunk by default), or links.
    Profile {
        network: PathBuf,
        #[arg(long)]
        path: Option<String>,
        #[arg(long, value_delimiter = ',')]
        links: Vec<String>,
        #[arg(long)]
        surface: Option<PathBuf>,
        #[arg(long)]
        options: Option<PathBuf>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Draw the sheet set of a network as it is.
    Draw {
        network: PathBuf,
        #[arg(long)]
        out_dir: PathBuf,
        #[command(flatten)]
        sheets: SheetArgs,
    },
    /// Everything from features: layout, zoning, design, profiles, drawings.
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

fn read_json(path: Option<&Path>) -> Result<Value> {
    match path {
        Some(p) => Ok(serde_json::from_str(
            &std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?,
        )?),
        None => Ok(json!({})),
    }
}

fn surface_json(path: Option<&Path>) -> Result<Value> {
    Ok(match path {
        Some(p) => serde_json::to_value(surface::read(p)?)?,
        None => Value::Null,
    })
}

fn sheet_fields(args: &SheetArgs) -> Result<Value> {
    Ok(json!({
        "title": read_json(args.title.as_deref())?,
        "paths": args.paths,
        "min_path_m": args.min_path_m,
        "surface": surface_json(args.surface.as_deref())?,
    }))
}

fn merge(mut a: Value, b: Value) -> Value {
    if let (Some(a), Value::Object(b)) = (a.as_object_mut(), b) {
        a.extend(b);
    }
    a
}

/// The engine identity Core admits: the engine, its operations and schemas,
/// and the release a stamped build names (`ds-water@<version>+<source>`,
/// what a saved network records); a development build names none.
fn build_info() -> Value {
    let version = env!("CARGO_PKG_VERSION");
    let source = Some(env!("DS_WATER_SOURCE_SHA")).filter(|sha| *sha != "development");
    let engine = ds_water_ops::invoke("identity", Value::Null).expect("identity never fails");
    let mut identity = json!({
        "schema": "ds.engine-build/v1",
        "name": "ds-water-engine",
        "version": version,
        "source_sha": env!("DS_WATER_SOURCE_SHA"),
        "target": env!("DS_WATER_TARGET"),
        "profile": env!("DS_WATER_PROFILE"),
        "rustc": env!("DS_WATER_RUSTC"),
        "cargo_lock_sha256": env!("DS_WATER_CARGO_LOCK_SHA256"),
        "build_manifest_sha256": env!("DS_WATER_BUILD_MANIFEST_SHA256"),
        "schemas": ["ds.water.network/v1"],
        "capabilities": engine["operations"],
    });
    if let Some(source) = source {
        identity["release"] = format!("ds-water@{version}+{source}").into();
    }
    identity
}

fn invoke(operation: &str, request: Value) -> Result<Value> {
    ds_water_ops::invoke(operation, request).map_err(|e| anyhow::anyhow!("{e}"))
}

/// Write the sheets (SVG) and profiles (JSON) of an answer; list them.
fn write_sheets(answer: &Value, out_dir: &Path) -> Result<Value> {
    std::fs::create_dir_all(out_dir)?;
    let mut listed = Vec::new();
    for sheet in answer["sheets"].as_array().into_iter().flatten() {
        let number = sheet["number"].as_str().unwrap_or("sheet");
        std::fs::write(
            out_dir.join(format!("{number}.svg")),
            sheet["svg"].as_str().unwrap_or(""),
        )?;
        listed.push(json!({ "number": number, "title": sheet["title"] }));
    }
    let mut profiles = Vec::new();
    for pair in answer["profiles"].as_array().into_iter().flatten() {
        let (label, p) = (pair[0].as_str().unwrap_or("path"), &pair[1]);
        std::fs::write(
            out_dir.join(format!("profile-{label}.json")),
            serde_json::to_vec_pretty(p)?,
        )?;
        profiles.push(json!({
            "path": label,
            "length_m": p["length_m"],
            "fittings": p["fittings"].as_array().map_or(0, Vec::len),
            "findings": p["findings"].as_array().map_or(0, Vec::len),
        }));
    }
    Ok(json!({ "sheets": listed, "profiles": profiles }))
}

fn run(command: Command) -> Result<Value> {
    match command {
        Command::BuildInfo => Ok(build_info()),
        Command::Invoke { operation } => {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
                .context("reading the request from stdin")?;
            let request = if text.trim().is_empty() {
                Value::Null
            } else {
                serde_json::from_str(&text).context("the request is not JSON")?
            };
            invoke(&operation, request)
        }
        Command::Validate { network } => {
            let n = read_network(&network)?;
            let answer = invoke("validate", json!({ "network": n }))?;
            Ok(merge(
                answer,
                json!({ "junctions": n.junctions.len(), "links": n.link_count() }),
            ))
        }
        Command::Simulate { network, eps, out } => {
            let n = read_network(&network)?;
            let started = Instant::now();
            let steps = ds_water_ops::simulate(&n, eps).map_err(|e| anyhow::anyhow!("{e}"))?;
            let elapsed = started.elapsed();
            if let Some(out) = out {
                let node_ids: Vec<&str> = n.node_refs().map(|(id, _)| id).collect();
                let link_ids: Vec<&str> = n
                    .pipes
                    .iter()
                    .map(|p| p.id.as_str())
                    .chain(n.pumps.iter().map(|p| p.id.as_str()))
                    .chain(n.valves.iter().map(|v| v.id.as_str()))
                    .collect();
                let doc = json!({ "schema": "ds.water.results/v1", "network": n.name, "node_ids": node_ids, "link_ids": link_ids, "steps": steps });
                std::fs::write(&out, serde_json::to_vec(&doc)?)?;
            }
            Ok(json!({
                "ok": steps.iter().all(|s| s.converged),
                "solve_ms": elapsed.as_secs_f64() * 1e3,
                "steps": steps.iter().map(|s| summary(&n, s)).collect::<Vec<_>>(),
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
        Command::Layout {
            features,
            options,
            surface,
            out,
        } => {
            let answer = invoke(
                "layout",
                json!({ "features": read_json(Some(&features))?, "options": read_json(options.as_deref())?, "surface": surface_json(surface.as_deref())? }),
            )?;
            std::fs::write(&out, serde_json::to_string_pretty(&answer["network"])?)?;
            let n: WaterNetwork = serde_json::from_value(answer["network"].clone())?;
            Ok(json!({
                "ok": true,
                "mode": answer["mode"],
                "junctions": n.junctions.len(),
                "pipes": n.pipes.len(),
                "length_m": n.pipes.iter().map(|p| p.length_m).sum::<f64>(),
                "demand_lps": n.total_base_demand_lps(),
                "paths": answer["paths"].as_array().map(|p| p.iter().take(10).map(|p| json!({ "label": p["label"], "tip": p["tip"], "length_m": p["length_m"] })).collect::<Vec<_>>()),
                "unserved": answer["unserved"],
                "notes": answer["notes"],
                "out": out,
            }))
        }
        Command::Design {
            network,
            criteria,
            catalogue,
            out,
            report,
        } => {
            let mut request = json!({ "network": read_network(&network)?, "criteria": read_json(criteria.as_deref())? });
            if let Some(c) = catalogue {
                request["catalogue"] = read_json(Some(&c))?;
            }
            let answer = invoke("design", request)?;
            std::fs::write(&out, serde_json::to_string_pretty(&answer["network"])?)?;
            if let Some(report) = report {
                std::fs::write(report, serde_json::to_vec_pretty(&answer)?)?;
            }
            let r: ds_water_design::DesignResult = serde_json::from_value(answer)?;
            Ok(ds_water_ops::summary(&r))
        }
        Command::Profile {
            network,
            path,
            links,
            surface,
            options,
            out,
        } => {
            let answer = invoke(
                "profile",
                json!({
                    "network": read_network(&network)?,
                    "path": path,
                    "links": links,
                    "options": read_json(options.as_deref())?,
                    "surface": surface_json(surface.as_deref())?,
                }),
            )?;
            if let Some(out) = &out {
                std::fs::write(out, serde_json::to_vec_pretty(&answer)?)?;
            }
            Ok(
                json!({ "ok": true, "length_m": answer["length_m"], "fittings": answer["fittings"], "findings": answer["findings"], "ground": answer["ground_source"], "out": out }),
            )
        }
        Command::Draw {
            network,
            out_dir,
            sheets,
        } => {
            let request = merge(
                json!({ "network": read_network(&network)? }),
                sheet_fields(&sheets)?,
            );
            write_sheets(&invoke("draw", request)?, &out_dir)
        }
        Command::Scheme {
            features,
            out_dir,
            options,
            criteria,
            sheets,
        } => {
            let request = merge(
                json!({ "features": read_json(Some(&features))?, "layout": read_json(options.as_deref())?, "criteria": read_json(criteria.as_deref())? }),
                sheet_fields(&sheets)?,
            );
            let answer = invoke("scheme", request)?;
            std::fs::create_dir_all(&out_dir)?;
            std::fs::write(
                out_dir.join("network.json"),
                serde_json::to_string_pretty(&answer["network"])?,
            )?;
            std::fs::write(
                out_dir.join("network.inp"),
                answer["inp"].as_str().unwrap_or(""),
            )?;
            std::fs::write(
                out_dir.join("design.json"),
                serde_json::to_vec_pretty(&answer["design_report"])?,
            )?;
            let drawings = write_sheets(&answer, &out_dir)?;
            Ok(
                json!({ "ok": true, "layout": answer["layout"], "design": answer["design"], "drawings": drawings, "out_dir": out_dir }),
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

fn summary(n: &WaterNetwork, s: &ds_water_hydraulics::Snapshot) -> Value {
    let nj = n.junctions.len();
    let connected: Vec<usize> = (0..nj).filter(|j| !s.disconnected.contains(j)).collect();
    let pick = |lower: bool| {
        connected.iter().copied().reduce(|a, b| {
            let better = if lower {
                s.pressure_m[b] < s.pressure_m[a]
            } else {
                s.pressure_m[b] > s.pressure_m[a]
            };
            if better { b } else { a }
        })
    };
    let point = |j: Option<usize>| {
        j.map(|j| json!({ "junction": n.junctions[j].id, "m": s.pressure_m[j] }))
    };
    json!({
        "time_s": s.time_s,
        "converged": s.converged,
        "iterations": s.iterations,
        "demand_lps": s.delivered_lps.iter().sum::<f64>(),
        "min_pressure": point(pick(true)),
        "max_pressure": point(pick(false)),
        "junctions_below_required": connected.iter().filter(|&&j| s.pressure_m[j] < n.options.required_pressure_m).count(),
        "max_pipe_velocity_ms": (0..n.pipes.len()).map(|k| s.velocity_ms[k]).fold(0.0, f64::max),
        "disconnected": s.disconnected.len(),
        "warnings": s.warnings,
    })
}
