//! Water vertical profile along one path of the network (a source-to-tip
//! path from the layout, trunk "01" first).
//!
//! Along chainage it carries the ground, the pipe centreline at its cover,
//! the hydraulic grade line (HGL) at peak demand and the static grade line at
//! zero demand, and from those places what a water engineer puts on a
//! profile drawing:
//!
//! - **air valves** on every summit of the pipe that rises and falls by at
//!   least the prominence (trapped air throttles gravity mains),
//! - **washouts** in every sag of the same prominence (draining, flushing),
//! - **isolation valves** at every tee and at a maximum spacing,
//! - **break-pressure tanks** wherever static pressure would pass the limit:
//!   the classic answer on steep gravity schemes, each one resetting the
//!   static line to its own water level,
//! - **findings**: sub-atmospheric pressure (HGL under the pipe), junctions
//!   below the minimum pressure, pipes above their class, static pressure
//!   over the limit.
//!
//! Ground comes from a surface sampled along the route with the same
//! `ds-terrain-sampling` the MV profile uses, or, without a surface, from the
//! node elevations.

use std::collections::HashMap;

use ds_terrain_sampling::{RoutePoint, SamplingOptions, SurfaceEvaluator, sample_route};
use ds_water_hydraulics::Snapshot;
use ds_water_layout::geo::Projection;
use ds_water_model::WaterNetwork;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileOptions {
    /// Cover from ground to pipe crown, m.
    pub cover_m: f64,
    /// Rise-and-fall needed to call a summit or sag, m.
    pub prominence_m: f64,
    pub isolation_spacing_m: f64,
    pub min_pressure_m: f64,
    pub max_static_pressure_m: f64,
    /// Water depth kept in a break-pressure tank, m.
    pub bpt_water_depth_m: f64,
    pub sampling: SamplingOptions,
}

impl Default for ProfileOptions {
    fn default() -> Self {
        Self {
            cover_m: 1.0,
            prominence_m: 1.0,
            isolation_spacing_m: 1000.0,
            min_pressure_m: 10.0,
            max_static_pressure_m: 80.0,
            bpt_water_depth_m: 1.5,
            sampling: SamplingOptions {
                maximum_gap_m: 20.0,
                station_jitter_m: 0.0,
                ..SamplingOptions::default()
            },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProfilePoint {
    pub chainage_m: f64,
    pub ground_m: f64,
    pub centre_m: f64,
    pub hgl_m: f64,
    pub static_m: f64,
    pub pressure_m: f64,
    pub static_pressure_m: f64,
    /// Source coordinate (lon/lat or projected, as the network's CRS).
    pub at: [f64; 2],
    /// Index into [`ProfileResult::segments`].
    pub segment: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileSegment {
    pub pipe: String,
    pub from_node: String,
    pub to_node: String,
    pub start_m: f64,
    pub end_m: f64,
    pub internal_mm: f64,
    pub material: Option<String>,
    pub pn_bar: Option<f64>,
    pub flow_lps: f64,
    pub velocity_ms: f64,
    pub hgl_gradient_m_per_km: f64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FittingKind {
    AirValve,
    Washout,
    IsolationValve,
    BreakPressureTank,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Fitting {
    pub kind: FittingKind,
    pub chainage_m: f64,
    pub elevation_m: f64,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    NegativePressure,
    LowPressure,
    OverClass,
    OverStaticLimit,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Finding {
    pub kind: FindingKind,
    pub from_m: f64,
    pub to_m: f64,
    /// Worst pressure in the range, m.
    pub worst_m: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeMark {
    pub id: String,
    pub chainage_m: f64,
    pub elevation_m: f64,
    pub tee: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileResult {
    pub length_m: f64,
    pub points: Vec<ProfilePoint>,
    pub segments: Vec<ProfileSegment>,
    pub nodes: Vec<NodeMark>,
    pub fittings: Vec<Fitting>,
    pub findings: Vec<Finding>,
    pub ground_source: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("pipe {0} is not in the network")]
    UnknownPipe(String),
    #[error("pipes {0} and {1} do not share a node")]
    Discontinuous(String, String),
    #[error("empty path")]
    Empty,
    #[error("terrain sampling: {0}")]
    Sampling(String),
}

/// Ground elevation at a source coordinate.
pub type Surface<'a> = &'a dyn Fn([f64; 2]) -> Option<f64>;

/// Build the profile of `pipes` (in order) from the peak and static solves.
pub fn profile(
    net: &WaterNetwork,
    pipes: &[String],
    peak: &Snapshot,
    still: &Snapshot,
    surface: Option<Surface<'_>>,
    opts: &ProfileOptions,
) -> Result<ProfileResult, ProfileError> {
    if pipes.is_empty() {
        return Err(ProfileError::Empty);
    }
    let node_index: HashMap<&str, usize> = net
        .node_refs()
        .enumerate()
        .map(|(i, (id, _))| (id, i))
        .collect();
    let refs: Vec<_> = net.node_refs().map(|(_, r)| r).collect();
    let pipe_index: HashMap<&str, usize> = net
        .pipes
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id.as_str(), i))
        .collect();
    let ks: Vec<usize> = pipes
        .iter()
        .map(|id| {
            pipe_index
                .get(id.as_str())
                .copied()
                .ok_or_else(|| ProfileError::UnknownPipe(id.clone()))
        })
        .collect::<Result<_, _>>()?;

    // Orient: the first pipe starts at the end it does not share with the
    // second; a lone pipe starts at its higher static head.
    let ends = |k: usize| {
        (
            node_index[net.pipes[k].from.as_str()],
            node_index[net.pipes[k].to.as_str()],
        )
    };
    let mut start = {
        let (a, b) = ends(ks[0]);
        match ks.get(1).map(|&k2| ends(k2)) {
            Some((c, d)) if a == c || a == d => b,
            Some(_) => a,
            None if still.head_m[b] > still.head_m[a] => b,
            None => a,
        }
    };
    let mut degree = vec![0usize; refs.len()];
    for p in &net.pipes {
        degree[node_index[p.from.as_str()]] += 1;
        degree[node_index[p.to.as_str()]] += 1;
    }

    // Route polyline in source coordinates, with node positions.
    let xy = |node: usize| net.node_xy(refs[node]).unwrap_or([0.0, 0.0]);
    let mut route: Vec<[f64; 2]> = vec![xy(start)];
    let mut route_node: Vec<Option<usize>> = vec![Some(start)];
    let mut legs: Vec<(usize, usize, usize, usize, usize)> = Vec::new(); // pipe, from, to, first vertex, last vertex
    for (i, &k) in ks.iter().enumerate() {
        let (a, b) = ends(k);
        let (from, to, forward) = if a == start {
            (a, b, true)
        } else if b == start {
            (b, a, false)
        } else {
            return Err(ProfileError::Discontinuous(
                pipes[i - 1].clone(),
                pipes[i].clone(),
            ));
        };
        let first = route.len() - 1;
        let mut verts = net.pipes[k].vertices.clone();
        if !forward {
            verts.reverse();
        }
        for v in verts {
            route.push(v);
            route_node.push(None);
        }
        route.push(xy(to));
        route_node.push(Some(to));
        legs.push((k, from, to, first, route.len() - 1));
        start = to;
    }
    let proj = Projection::for_points(&route, net.crs.as_deref());
    let metric: Vec<[f64; 2]> = route.iter().map(|&p| proj.forward(p)).collect();
    let mut chain = vec![0.0; route.len()];
    for i in 1..route.len() {
        chain[i] =
            chain[i - 1] + (metric[i][0] - metric[i - 1][0]).hypot(metric[i][1] - metric[i - 1][1]);
    }
    let length = *chain.last().unwrap_or(&0.0);

    // Ground stations.
    let elevation = |node: usize| net.node_elevation(refs[node]);
    let mut stations: Vec<(f64, f64)> = Vec::new();
    let ground_source = match surface {
        Some(surface) if length > 0.0 => {
            struct Eval<'a> {
                proj: Projection,
                surface: Surface<'a>,
            }
            impl SurfaceEvaluator for Eval<'_> {
                fn elevations(&mut self, points: &[[f64; 2]]) -> Result<Vec<Option<f64>>, String> {
                    Ok(points
                        .iter()
                        .map(|&p| (self.surface)(self.proj.inverse(p)))
                        .collect())
                }
            }
            let mut eval = Eval { proj, surface };
            let route_points: Vec<RoutePoint> = dedup(&metric)
                .iter()
                .map(|p| RoutePoint {
                    x_m: p[0],
                    y_m: p[1],
                })
                .collect();
            let sampled = sample_route(&route_points, &mut eval, &opts.sampling)
                .map_err(|e| ProfileError::Sampling(format!("{e:?}")))?;
            for s in &sampled.centerline.samples {
                if let Some(z) = s.elevation_m {
                    stations.push((s.station_m, z));
                }
            }
            "surface sampled with ds-terrain-sampling".to_owned()
        }
        _ => "node elevations (no surface given)".to_owned(),
    };
    // Without a surface (or where it has gaps), interpolate along each leg
    // between its end nodes' elevations.
    let leg_ground = |c: f64| -> f64 {
        for &(_, from, to, first, last) in &legs {
            if c <= chain[last] + 1e-9 {
                let span = (chain[last] - chain[first]).max(1e-9);
                let t = ((c - chain[first]) / span).clamp(0.0, 1.0);
                return elevation(from) + t * (elevation(to) - elevation(from));
            }
        }
        legs.last().map(|l| elevation(l.2)).unwrap_or(0.0)
    };
    let mut chainages: Vec<f64> = chain.clone();
    chainages.extend(stations.iter().map(|s| s.0));
    chainages.sort_by(f64::total_cmp);
    chainages.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    let ground_at = |c: f64| -> f64 {
        if stations.is_empty() {
            return leg_ground(c);
        }
        let i = stations.partition_point(|s| s.0 < c);
        match (stations.get(i.wrapping_sub(1)), stations.get(i)) {
            (Some(a), Some(b)) if b.0 > a.0 => a.1 + (b.1 - a.1) * (c - a.0) / (b.0 - a.0),
            (_, Some(b)) => b.1,
            (Some(a), _) => a.1,
            _ => leg_ground(c),
        }
    };

    let mut segments = Vec::new();
    for &(k, from, to, first, last) in &legs {
        let p = &net.pipes[k];
        let forward = node_index[p.from.as_str()] == from;
        let q = if forward {
            peak.flow_lps[k]
        } else {
            -peak.flow_lps[k]
        };
        let span = (chain[last] - chain[first]).max(1e-9);
        segments.push(ProfileSegment {
            pipe: p.id.clone(),
            from_node: id_of(net, from),
            to_node: id_of(net, to),
            start_m: chain[first],
            end_m: chain[last],
            internal_mm: p.diameter_mm,
            material: p.material.clone(),
            pn_bar: p.pressure_class_bar,
            flow_lps: q,
            velocity_ms: peak.velocity_ms[k],
            hgl_gradient_m_per_km: (peak.head_m[from] - peak.head_m[to]) / span * 1000.0,
        });
    }

    let mut points = Vec::with_capacity(chainages.len());
    for &c in &chainages {
        let s = legs
            .iter()
            .position(|&(_, _, _, _, last)| c <= chain[last] + 1e-9)
            .unwrap_or(legs.len() - 1);
        let (k, from, to, first, last) = legs[s];
        let span = (chain[last] - chain[first]).max(1e-9);
        let t = ((c - chain[first]) / span).clamp(0.0, 1.0);
        let ground = ground_at(c);
        let outside = net.pipes[k].diameter_mm * 1.12 / 1000.0;
        let centre = ground - opts.cover_m - outside / 2.0;
        let hgl = peak.head_m[from] + t * (peak.head_m[to] - peak.head_m[from]);
        let stat = still.head_m[from] + t * (still.head_m[to] - still.head_m[from]);
        let at = interpolate(&route, &chain, c);
        points.push(ProfilePoint {
            chainage_m: c,
            ground_m: ground,
            centre_m: centre,
            hgl_m: hgl,
            static_m: stat,
            pressure_m: hgl - centre,
            static_pressure_m: stat - centre,
            at,
            segment: s,
        });
    }

    let nodes: Vec<NodeMark> = route_node
        .iter()
        .enumerate()
        .filter_map(|(i, n)| {
            n.map(|n| NodeMark {
                id: id_of(net, n),
                chainage_m: chain[i],
                elevation_m: elevation(n),
                tee: degree[n] >= 3,
            })
        })
        .collect();

    let mut fittings = Vec::new();
    peaks_and_sags(&points, opts.prominence_m, &mut fittings);
    for node in &nodes {
        if node.tee {
            fittings.push(Fitting {
                kind: FittingKind::IsolationValve,
                chainage_m: node.chainage_m,
                elevation_m: ground_at(node.chainage_m),
                reason: format!("tee at {}", node.id),
            });
        }
    }
    if opts.isolation_spacing_m > 0.0 {
        let mut next = opts.isolation_spacing_m;
        while next < length {
            if !fittings.iter().any(|f| {
                f.kind == FittingKind::IsolationValve
                    && (f.chainage_m - next).abs() < opts.isolation_spacing_m / 4.0
            }) {
                fittings.push(Fitting {
                    kind: FittingKind::IsolationValve,
                    chainage_m: next,
                    elevation_m: ground_at(next),
                    reason: format!("spacing {:.0} m", opts.isolation_spacing_m),
                });
            }
            next += opts.isolation_spacing_m;
        }
    }
    break_pressure_tanks(&points, opts, &mut fittings);
    fittings.sort_by(|a, b| a.chainage_m.total_cmp(&b.chainage_m));

    let mut findings = Vec::new();
    let junction_at: Vec<bool> = chainages
        .iter()
        .map(|&c| {
            nodes.iter().any(|n| {
                (n.chainage_m - c).abs() < 1e-6 && net.junctions.iter().any(|j| j.id == n.id)
            })
        })
        .collect();
    collect(
        &points,
        &mut findings,
        FindingKind::NegativePressure,
        |_, p| (p.pressure_m < 0.0).then_some(p.pressure_m),
    );
    collect(&points, &mut findings, FindingKind::LowPressure, |i, p| {
        (junction_at[i] && p.pressure_m < opts.min_pressure_m).then_some(p.pressure_m)
    });
    collect(&points, &mut findings, FindingKind::OverClass, |_, p| {
        let pn = segments[p.segment].pn_bar?;
        (p.static_pressure_m.max(p.pressure_m) > pn * 10.197)
            .then_some(p.static_pressure_m.max(p.pressure_m))
    });
    collect(
        &points,
        &mut findings,
        FindingKind::OverStaticLimit,
        |_, p| (p.static_pressure_m > opts.max_static_pressure_m).then_some(p.static_pressure_m),
    );

    Ok(ProfileResult {
        length_m: length,
        points,
        segments,
        nodes,
        fittings,
        findings,
        ground_source,
    })
}

fn id_of(net: &WaterNetwork, node: usize) -> String {
    net.node_refs()
        .nth(node)
        .map(|(id, _)| id.to_owned())
        .unwrap_or_default()
}

fn dedup(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut out: Vec<[f64; 2]> = Vec::with_capacity(points.len());
    for &p in points {
        if out
            .last()
            .is_none_or(|q| (q[0] - p[0]).hypot(q[1] - p[1]) > 1e-6)
        {
            out.push(p);
        }
    }
    out
}

fn interpolate(route: &[[f64; 2]], chain: &[f64], c: f64) -> [f64; 2] {
    let i = chain
        .partition_point(|&x| x < c)
        .clamp(1, route.len().max(2) - 1);
    if route.len() < 2 {
        return route.first().copied().unwrap_or([0.0, 0.0]);
    }
    let span = (chain[i] - chain[i - 1]).max(1e-9);
    let t = ((c - chain[i - 1]) / span).clamp(0.0, 1.0);
    [
        route[i - 1][0] + t * (route[i][0] - route[i - 1][0]),
        route[i - 1][1] + t * (route[i][1] - route[i - 1][1]),
    ]
}

/// Summits and sags of the pipe centreline with hysteresis: a summit counts
/// once the line has risen into it and fallen out of it by the prominence.
fn peaks_and_sags(points: &[ProfilePoint], prominence: f64, out: &mut Vec<Fitting>) {
    if points.len() < 3 {
        return;
    }
    let mut low = points[0].centre_m;
    let mut high = points[0].centre_m;
    let mut high_at = 0;
    let mut low_at = 0;
    // +1 rising (looking for a summit), -1 falling (looking for a sag).
    let mut trend = 0i8;
    for (i, p) in points.iter().enumerate() {
        let z = p.centre_m;
        match trend {
            0 => {
                if z >= low + prominence {
                    trend = 1;
                    high = z;
                    high_at = i;
                } else if z <= high - prominence {
                    trend = -1;
                    low = z;
                    low_at = i;
                }
                if z < low {
                    low = z;
                    low_at = i;
                }
                if z > high {
                    high = z;
                    high_at = i;
                }
            }
            1 => {
                if z > high {
                    high = z;
                    high_at = i;
                } else if z <= high - prominence {
                    out.push(Fitting {
                        kind: FittingKind::AirValve,
                        chainage_m: points[high_at].chainage_m,
                        elevation_m: points[high_at].ground_m,
                        reason: format!("summit, {prominence:.1} m prominence"),
                    });
                    trend = -1;
                    low = z;
                    low_at = i;
                }
            }
            _ => {
                if z < low {
                    low = z;
                    low_at = i;
                } else if z >= low + prominence {
                    out.push(Fitting {
                        kind: FittingKind::Washout,
                        chainage_m: points[low_at].chainage_m,
                        elevation_m: points[low_at].ground_m,
                        reason: format!("sag, {prominence:.1} m prominence"),
                    });
                    trend = 1;
                    high = z;
                    high_at = i;
                }
            }
        }
    }
}

/// Walk downstream; where static pressure would pass the limit, place a
/// break-pressure tank at the last point within it, and carry on from that
/// tank's water level.
fn break_pressure_tanks(points: &[ProfilePoint], opts: &ProfileOptions, out: &mut Vec<Fitting>) {
    let Some(first) = points.first() else { return };
    let mut level = first.static_m;
    let mut previous: Option<&ProfilePoint> = None;
    for p in points {
        if level - p.centre_m > opts.max_static_pressure_m
            && let Some(prev) = previous
        {
            // The tank sits where the static pressure is still within limit.
            out.push(Fitting {
                kind: FittingKind::BreakPressureTank,
                chainage_m: prev.chainage_m,
                elevation_m: prev.ground_m,
                reason: format!(
                    "static pressure would exceed {:.0} m; resets the line to {:.1} m",
                    opts.max_static_pressure_m,
                    prev.ground_m + opts.bpt_water_depth_m
                ),
            });
            level = prev.ground_m + opts.bpt_water_depth_m;
        }
        // The static line never rises above the network's own static head.
        level = level.min(p.static_m);
        previous = Some(p);
    }
}

fn collect(
    points: &[ProfilePoint],
    out: &mut Vec<Finding>,
    kind: FindingKind,
    test: impl Fn(usize, &ProfilePoint) -> Option<f64>,
) {
    let mut open: Option<Finding> = None;
    for (i, p) in points.iter().enumerate() {
        match (test(i, p), open.as_mut()) {
            (Some(v), Some(f)) => {
                f.to_m = p.chainage_m;
                f.worst_m =
                    if kind == FindingKind::OverClass || kind == FindingKind::OverStaticLimit {
                        f.worst_m.max(v)
                    } else {
                        f.worst_m.min(v)
                    };
            }
            (Some(v), None) => {
                open = Some(Finding {
                    kind,
                    from_m: p.chainage_m,
                    to_m: p.chainage_m,
                    worst_m: v,
                })
            }
            (None, _) => {
                if let Some(f) = open.take() {
                    out.push(f);
                }
            }
        }
    }
    if let Some(f) = open {
        out.push(f);
    }
}

#[cfg(test)]
mod tests;
