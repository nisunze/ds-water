//! Water horizontal layout: from map features to a hydraulic network.
//!
//! Input is a GeoJSON FeatureCollection (WGS84 lon/lat or a projected CRS).
//! A feature's `water_kind` property says what it is:
//!
//! | Geometry | `water_kind` | Becomes |
//! | --- | --- | --- |
//! | Point | `source`, `reservoir`, `spring`, `borehole`, `intake` | Reservoir (head `head_m`, else ground + `source_height_m`) |
//! | Point | `tank` | Tank on the ground at that point |
//! | Point | `standpost`, `kiosk`, `institution`, `junction` | A junction of its own, on the nearest pipe |
//! | Point | anything else (`customer`, `household`, none) | Demand at its tap on the nearest pipe (pipes), or a junction with a service pipe (roads) |
//! | Line | `pipe` or none | A pipe (drawn network) |
//! | Line | `road`, `track`, `path` | A corridor pipes may follow (automatic layout) |
//!
//! Demand is `demand_lps`, else the national standard for the point's kind
//! (MININFRA Table 1-3, see [`demand`]: people, beds, students, head...),
//! else `population` × litres per capita per day; both × the peak factor. Elevation is the geometry's Z, else `elevation_m`, else the
//! caller's elevation source, else interpolated from known neighbours. Pipe
//! length is measured along the slope between its end nodes.
//!
//! Three modes, chosen from the input when not forced: **pipes** turns a
//! drawn network into nodes and links (endpoints snapped, T-junctions split,
//! demand allocated); **roads** grows a source-rooted shortest-path tree along
//! the corridors to every demand point; **spanning** joins the points with a
//! minimum spanning tree over nearest neighbours. Every result is oriented
//! from its sources with `ds-graph`, the primitive LV uses, so trunk and
//! branch paths ("01", "02", …) mean the same thing in both verticals.

pub mod demand;
pub mod geo;

use std::collections::{HashMap, HashSet};

use ds_water_model::{Junction, Pipe, Reservoir, Tank, WaterNetwork};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use geo::{Grid, Projection, dist, project_on_segment};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayoutMode {
    /// Pipes if any pipe line is given, else roads if any road, else spanning.
    #[default]
    Auto,
    Pipes,
    Roads,
    Spanning,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutOptions {
    pub mode: LayoutMode,
    pub crs: Option<String>,
    /// Ends and vertices closer than this are one node.
    pub snap_tolerance_m: f64,
    /// Demand points farther than this from the network are left unserved.
    pub max_service_m: f64,
    /// Litres per person per day (Rwanda rural standpost norm: 20).
    pub litres_per_capita_day: f64,
    pub peak_factor: f64,
    pub default_diameter_mm: f64,
    pub roughness: f64,
    /// Water level above ground of a source given without `head_m`.
    pub source_height_m: f64,
    /// Candidate neighbours per point for the spanning layout.
    pub neighbours: usize,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            mode: LayoutMode::Auto,
            crs: None,
            snap_tolerance_m: 0.5,
            max_service_m: 500.0,
            litres_per_capita_day: 20.0,
            peak_factor: 2.0,
            default_diameter_mm: 110.0,
            roughness: 140.0,
            source_height_m: 3.0,
            neighbours: 8,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PathSummary {
    /// "01" is the trunk: source to the farthest terminal.
    pub label: String,
    pub root: String,
    pub tip: String,
    pub length_m: f64,
    pub nodes: Vec<String>,
    /// Links root → tip: pipes, and any valves or pumps on the way.
    pub pipes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayoutResult {
    pub mode: LayoutMode,
    pub network: WaterNetwork,
    pub paths: Vec<PathSummary>,
    /// Ids (or indices) of demand points the layout could not reach.
    pub unserved: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("input is not a GeoJSON FeatureCollection")]
    NotFeatureCollection,
    #[error("no source: add a Point with water_kind source, reservoir or tank")]
    NoSource,
    #[error("orientation failed: {0}")]
    Graph(String),
}

/// Ground elevation for a source coordinate, when the caller has a surface.
pub type ElevationSource<'a> = &'a dyn Fn([f64; 2]) -> Option<f64>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointRole {
    Source,
    Tank,
    OwnJunction,
    Demand,
}

#[derive(Clone, Debug)]
struct PointFeature {
    id: Option<String>,
    at: [f64; 2],
    z: Option<f64>,
    role: PointRole,
    demand_lps: f64,
    head_m: Option<f64>,
    props: serde_json::Map<String, Value>,
}

#[derive(Clone, Debug)]
struct LineFeature {
    id: Option<String>,
    coords: Vec<[f64; 2]>,
    z: Vec<Option<f64>>,
    road: bool,
    props: serde_json::Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum NodeRole {
    Plain,
    Demand,
    Source(usize),
    Tank(usize),
    Junction,
}

#[derive(Clone, Debug)]
struct Node {
    xy: [f64; 2],
    src: [f64; 2],
    z: Option<f64>,
    role: NodeRole,
    demand_lps: f64,
    id: Option<String>,
}

#[derive(Clone, Debug)]
struct Edge {
    a: usize,
    b: usize,
    /// Source coordinates of interior vertices, a → b.
    interior: Vec<[f64; 2]>,
    line: Option<usize>,
    length_m: f64,
    /// False once split: edges are tombstoned, never moved, so the segment
    /// index stays valid while points are attached.
    alive: bool,
}

/// A line vertex: projected metres, source coordinate, optional Z.
type Vertex = ([f64; 2], [f64; 2], Option<f64>);

struct Builder<'a> {
    opts: &'a LayoutOptions,
    proj: Projection,
    nodes: Vec<Node>,
    grid: Grid,
    edges: Vec<Edge>,
    segments: Grid,
    notes: Vec<String>,
}

impl Builder<'_> {
    /// The node within snap tolerance of `src`, or a new one.
    fn node_at(&mut self, src: [f64; 2], z: Option<f64>) -> usize {
        let xy = self.proj.forward(src);
        let tol = self.opts.snap_tolerance_m;
        let found = self
            .grid
            .near(xy, tol)
            .map(|i| (i, dist(xy, self.nodes[i].xy)))
            .filter(|&(_, d)| d <= tol)
            .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        if let Some((i, _)) = found {
            if self.nodes[i].z.is_none() {
                self.nodes[i].z = z;
            }
            return i;
        }
        self.nodes.push(Node {
            xy,
            src,
            z,
            role: NodeRole::Plain,
            demand_lps: 0.0,
            id: None,
        });
        let i = self.nodes.len() - 1;
        self.grid.insert(xy, i);
        i
    }

    fn edge(&mut self, a: usize, b: usize, interior: Vec<[f64; 2]>, line: Option<usize>) {
        if a == b {
            return;
        }
        let mut chain = vec![self.nodes[a].src];
        chain.extend(interior.iter().copied());
        chain.push(self.nodes[b].src);
        let plan: f64 = chain
            .windows(2)
            .map(|w| self.proj.distance(w[0], w[1]))
            .sum();
        self.edges.push(Edge {
            a,
            b,
            interior,
            line,
            length_m: plan,
            alive: true,
        });
        let e = self.edges.len() - 1;
        for (s, w) in self.edge_xy(e).windows(2).enumerate() {
            let lo = [w[0][0].min(w[1][0]), w[0][1].min(w[1][1])];
            let hi = [w[0][0].max(w[1][0]), w[0][1].max(w[1][1])];
            self.segments.insert_box(lo, hi, e << 20 | s);
        }
    }

    /// Split lines into edges at every shared vertex and T-junction.
    fn add_lines(&mut self, lines: &[LineFeature], indices: &[usize]) {
        let tol = self.opts.snap_tolerance_m;
        // Projected copies, with T-junction vertices inserted where another
        // line's end lands on a segment's interior.
        let mut projected: Vec<Vec<Vertex>> = indices
            .iter()
            .map(|&l| {
                lines[l]
                    .coords
                    .iter()
                    .zip(&lines[l].z)
                    .map(|(&c, &z)| (self.proj.forward(c), c, z))
                    .collect()
            })
            .collect();
        let mut seg_grid = Grid::new(50.0);
        for (li, line) in projected.iter().enumerate() {
            for (si, w) in line.windows(2).enumerate() {
                let lo = [
                    w[0].0[0].min(w[1].0[0]) - tol,
                    w[0].0[1].min(w[1].0[1]) - tol,
                ];
                let hi = [
                    w[0].0[0].max(w[1].0[0]) + tol,
                    w[0].0[1].max(w[1].0[1]) + tol,
                ];
                seg_grid.insert_box(lo, hi, li << 20 | si);
            }
        }
        let mut inserts: Vec<(usize, usize, f64, Vertex)> = Vec::new();
        for (li, line) in projected.iter().enumerate() {
            for end in [line[0], line[line.len() - 1]] {
                for key in seg_grid.near(end.0, tol) {
                    let (lj, sj) = (key >> 20, key & 0xF_FFFF);
                    if lj == li {
                        continue;
                    }
                    let (a, b) = (projected[lj][sj], projected[lj][sj + 1]);
                    let (t, d) = project_on_segment(end.0, a.0, b.0);
                    if d <= tol && t > 1e-9 && t < 1.0 - 1e-9 {
                        inserts.push((lj, sj, t, end));
                    }
                }
            }
        }
        inserts.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(b.2.total_cmp(&a.2)));
        inserts.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1 && (a.2 - b.2).abs() < 1e-12);
        for (lj, sj, _, vertex) in inserts {
            projected[lj].insert(sj + 1, vertex);
        }

        // A vertex is a node when it ends a line or is shared by two lines.
        let mut use_count: HashMap<(i64, i64), usize> = HashMap::new();
        let key = |p: [f64; 2]| {
            (
                (p[0] / tol.max(1e-6)).round() as i64,
                (p[1] / tol.max(1e-6)).round() as i64,
            )
        };
        for line in &projected {
            let mut seen = HashSet::new();
            for v in line {
                if seen.insert(key(v.0)) {
                    *use_count.entry(key(v.0)).or_default() += 1;
                }
            }
        }
        for (li, line) in projected.iter().enumerate() {
            let mut start = self.node_at(line[0].1, line[0].2);
            let mut interior = Vec::new();
            for (vi, v) in line.iter().enumerate().skip(1) {
                let last = vi == line.len() - 1;
                if last || use_count[&key(v.0)] > 1 {
                    let end = self.node_at(v.1, v.2);
                    self.edge(start, end, std::mem::take(&mut interior), Some(indices[li]));
                    start = end;
                } else {
                    interior.push(v.1);
                }
            }
        }
    }

    /// Nearest point on any edge to `xy`: (edge, segment, t, distance).
    fn nearest_edge(&self, xy: [f64; 2], radius: f64) -> Option<(usize, usize, f64, f64)> {
        let mut best: Option<(usize, usize, f64, f64)> = None;
        for key in self.segments.near(xy, radius) {
            let (e, s) = (key >> 20, key & 0xF_FFFF);
            if !self.edges[e].alive {
                continue;
            }
            let chain = self.edge_xy(e);
            if s + 1 >= chain.len() {
                continue;
            }
            let (t, d) = project_on_segment(xy, chain[s], chain[s + 1]);
            if d <= radius && best.is_none_or(|b| d < b.3) {
                best = Some((e, s, t, d));
            }
        }
        best
    }

    fn edge_xy(&self, e: usize) -> Vec<[f64; 2]> {
        let edge = &self.edges[e];
        let mut v = vec![self.nodes[edge.a].xy];
        v.extend(edge.interior.iter().map(|&s| self.proj.forward(s)));
        v.push(self.nodes[edge.b].xy);
        v
    }

    /// Split edge `e` at segment `s`, parameter `t`; returns the new node.
    fn split_edge(&mut self, e: usize, s: usize, t: f64) -> usize {
        let chain = self.edge_xy(e);
        if t <= 1e-9 && s == 0 {
            return self.edges[e].a;
        }
        if t >= 1.0 - 1e-9 && s + 2 == chain.len() {
            return self.edges[e].b;
        }
        let p = [
            chain[s][0] + t * (chain[s + 1][0] - chain[s][0]),
            chain[s][1] + t * (chain[s + 1][1] - chain[s][1]),
        ];
        let src = self.proj.inverse(p);
        let node = self.node_at(src, None);
        let old = self.edges[e].clone();
        let (first, second): (Vec<[f64; 2]>, Vec<[f64; 2]>) = {
            let mut first = old.interior[..s.min(old.interior.len())].to_vec();
            let mut second = old.interior[s.min(old.interior.len())..].to_vec();
            if t <= 1e-9 && s > 0 {
                first.pop();
            }
            if t >= 1.0 - 1e-9 && !second.is_empty() {
                second.remove(0);
            }
            (first, second)
        };
        self.edges[e].alive = false;
        self.edge(old.a, node, first, old.line);
        self.edge(node, old.b, second, old.line);
        node
    }
}

/// Lay out a network from GeoJSON features.
pub fn layout(
    features: &Value,
    opts: &LayoutOptions,
    elevation: Option<ElevationSource<'_>>,
) -> Result<LayoutResult, LayoutError> {
    let (points, lines) = read_features(features, opts)?;
    if !points
        .iter()
        .any(|p| matches!(p.role, PointRole::Source | PointRole::Tank))
    {
        return Err(LayoutError::NoSource);
    }
    let all: Vec<[f64; 2]> = points
        .iter()
        .map(|p| p.at)
        .chain(lines.iter().flat_map(|l| l.coords.iter().copied()))
        .collect();
    let proj = Projection::for_points(&all, opts.crs.as_deref());
    let mut b = Builder {
        opts,
        proj,
        nodes: Vec::new(),
        grid: Grid::new(opts.snap_tolerance_m.max(1.0) * 4.0),
        edges: Vec::new(),
        segments: Grid::new(100.0),
        notes: Vec::new(),
    };
    let pipes: Vec<usize> = (0..lines.len()).filter(|&i| !lines[i].road).collect();
    let roads: Vec<usize> = (0..lines.len()).filter(|&i| lines[i].road).collect();
    let mode = match opts.mode {
        LayoutMode::Auto if !pipes.is_empty() => LayoutMode::Pipes,
        LayoutMode::Auto if !roads.is_empty() => LayoutMode::Roads,
        LayoutMode::Auto => LayoutMode::Spanning,
        m => m,
    };
    let mut unserved = Vec::new();
    let mut sources = 0;
    let mut tanks = 0;
    match mode {
        LayoutMode::Pipes | LayoutMode::Roads => {
            let corridor = if mode == LayoutMode::Pipes {
                &pipes
            } else {
                &roads
            };
            b.add_lines(&lines, corridor);
            attach_points(
                &mut b,
                &points,
                mode,
                &mut unserved,
                &mut sources,
                &mut tanks,
            );
            if mode == LayoutMode::Roads {
                keep_supply_tree(&mut b, &mut unserved)?;
            }
        }
        LayoutMode::Spanning => {
            for p in &points {
                let n = b.nodes.len();
                b.nodes.push(Node {
                    xy: b.proj.forward(p.at),
                    src: p.at,
                    z: p.z,
                    role: NodeRole::Plain,
                    demand_lps: 0.0,
                    id: p.id.clone(),
                });
                set_role(&mut b.nodes[n], p, &mut sources, &mut tanks);
            }
            spanning_tree(&mut b);
        }
        LayoutMode::Auto => unreachable!("resolved above"),
    }
    fill_elevations(&mut b, elevation);
    let network = to_network(&b, &points, &lines, opts);
    let paths = orient(&network)?;
    let mut notes = b.notes;
    notes.push(format!(
        "{mode:?} layout: {} junctions, {} pipes, {:.0} m of pipe",
        network.junctions.len(),
        network.pipes.len(),
        network.pipes.iter().map(|p| p.length_m).sum::<f64>()
    ));
    Ok(LayoutResult {
        mode,
        network,
        paths,
        unserved,
        notes,
    })
}

fn read_features(
    fc: &Value,
    opts: &LayoutOptions,
) -> Result<(Vec<PointFeature>, Vec<LineFeature>), LayoutError> {
    let features = fc
        .get("features")
        .and_then(Value::as_array)
        .ok_or(LayoutError::NotFeatureCollection)?;
    let mut points = Vec::new();
    let mut lines = Vec::new();
    for f in features {
        let props = f
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let kind = props
            .get("water_kind")
            .or_else(|| props.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let id = props.get("id").or_else(|| f.get("id")).and_then(|v| {
            v.as_str()
                .map(str::to_owned)
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        });
        let number = |k: &str| props.get(k).and_then(Value::as_f64);
        let Some(geometry) = f.get("geometry") else {
            continue;
        };
        let gtype = geometry.get("type").and_then(Value::as_str).unwrap_or("");
        let coords = geometry.get("coordinates");
        let pos = |v: &Value| -> Option<([f64; 2], Option<f64>)> {
            let a = v.as_array()?;
            Some((
                [a.first()?.as_f64()?, a.get(1)?.as_f64()?],
                a.get(2).and_then(Value::as_f64),
            ))
        };
        match gtype {
            "Point" => {
                let Some((at, z)) = coords.and_then(pos) else {
                    continue;
                };
                let role = match kind.as_str() {
                    "source" | "reservoir" | "spring" | "borehole" | "intake" => PointRole::Source,
                    "tank" => PointRole::Tank,
                    "standpost" | "kiosk" | "institution" | "junction" => PointRole::OwnJunction,
                    // Institutions in the national table get their own connection.
                    k if demand::standard(k)
                        .is_some_and(|s| !matches!(s.kind, "household" | "urban_household")) =>
                    {
                        PointRole::OwnJunction
                    }
                    _ => PointRole::Demand,
                };
                // Explicit demand wins; otherwise the national standard row
                // for the kind; otherwise people at the configured l/c/d.
                let per_day = |litres: f64| litres * opts.peak_factor / 86_400.0;
                let demand_lps = number("demand_lps")
                    .or_else(|| demand::daily_litres(&kind, |p| number(p)).map(per_day))
                    .unwrap_or_else(|| {
                        per_day(number("population").unwrap_or(0.0) * opts.litres_per_capita_day)
                    });
                points.push(PointFeature {
                    id,
                    at,
                    z: z.or_else(|| number("elevation_m")),
                    role,
                    demand_lps,
                    head_m: number("head_m"),
                    props,
                });
            }
            "LineString" | "MultiLineString" => {
                let parts: Vec<&Value> = if gtype == "LineString" {
                    coords.into_iter().collect()
                } else {
                    coords
                        .and_then(Value::as_array)
                        .map(|a| a.iter().collect())
                        .unwrap_or_default()
                };
                for (pi, part) in parts.into_iter().enumerate() {
                    let verts: Vec<([f64; 2], Option<f64>)> = part
                        .as_array()
                        .map(|a| a.iter().filter_map(pos).collect())
                        .unwrap_or_default();
                    if verts.len() < 2 {
                        continue;
                    }
                    lines.push(LineFeature {
                        id: id
                            .clone()
                            .map(|i| if pi == 0 { i } else { format!("{i}.{pi}") }),
                        coords: verts.iter().map(|v| v.0).collect(),
                        z: verts.iter().map(|v| v.1).collect(),
                        road: matches!(kind.as_str(), "road" | "track" | "path"),
                        props: props.clone(),
                    });
                }
            }
            _ => {}
        }
    }
    Ok((points, lines))
}

fn set_role(node: &mut Node, p: &PointFeature, sources: &mut usize, tanks: &mut usize) {
    node.id = p.id.clone().or(node.id.take());
    if node.z.is_none() {
        node.z = p.z;
    }
    match p.role {
        PointRole::Source => {
            node.role = NodeRole::Source(*sources);
            *sources += 1;
        }
        PointRole::Tank => {
            node.role = NodeRole::Tank(*tanks);
            *tanks += 1;
        }
        PointRole::OwnJunction => {
            node.role = NodeRole::Junction;
            node.demand_lps += p.demand_lps;
        }
        PointRole::Demand => {
            if node.role == NodeRole::Plain {
                node.role = NodeRole::Demand;
            }
            node.demand_lps += p.demand_lps;
        }
    }
}

/// Put sources, tanks and demand on the corridor network.
fn attach_points(
    b: &mut Builder<'_>,
    points: &[PointFeature],
    mode: LayoutMode,
    unserved: &mut Vec<String>,
    sources: &mut usize,
    tanks: &mut usize,
) {
    let max = b.opts.max_service_m;
    for (pi, p) in points.iter().enumerate() {
        let xy = b.proj.forward(p.at);
        let Some((e, s, t, d)) = b.nearest_edge(xy, max) else {
            if matches!(p.role, PointRole::Source | PointRole::Tank) {
                // A source off the corridor still gets its own node and a
                // straight main to the nearest node.
                let n = b.node_at(p.at, p.z);
                set_role(&mut b.nodes[n], p, sources, tanks);
                if let Some(nearest) = (0..b.nodes.len())
                    .filter(|&i| i != n)
                    .min_by(|&i, &j| dist(b.nodes[i].xy, xy).total_cmp(&dist(b.nodes[j].xy, xy)))
                {
                    b.edge(n, nearest, Vec::new(), None);
                    b.notes.push(format!(
                        "source {} joined by a straight main",
                        p.id.as_deref().unwrap_or("?")
                    ));
                }
            } else {
                unserved.push(p.id.clone().unwrap_or_else(|| format!("point#{pi}")));
            }
            continue;
        };
        let tap = b.split_edge(e, s, t);
        let household = p.role == PointRole::Demand && mode == LayoutMode::Pipes;
        if household || d <= b.opts.snap_tolerance_m {
            // On the line (or a customer of a drawn network): demand lands on
            // the tap itself.
            set_role(&mut b.nodes[tap], p, sources, tanks);
        } else {
            let n = b.nodes.len();
            b.nodes.push(Node {
                xy,
                src: p.at,
                z: p.z,
                role: NodeRole::Plain,
                demand_lps: 0.0,
                id: None,
            });
            b.grid.insert(xy, n);
            set_role(&mut b.nodes[n], p, sources, tanks);
            b.edge(tap, n, Vec::new(), None);
        }
    }
}

/// Keep only corridor edges on a source's shortest path to some demand.
fn keep_supply_tree(b: &mut Builder<'_>, unserved: &mut Vec<String>) -> Result<(), LayoutError> {
    let ids: Vec<String> = (0..b.nodes.len()).map(|i| i.to_string()).collect();
    let alive: Vec<usize> = (0..b.edges.len()).filter(|&k| b.edges[k].alive).collect();
    let input = ds_graph::BuildInput {
        nodes: ids.clone(),
        edges: alive
            .iter()
            .map(|&k| {
                let e = &b.edges[k];
                ds_graph::GraphEdge {
                    a: ids[e.a].clone(),
                    b: ids[e.b].clone(),
                    length_m: e.length_m,
                }
            })
            .collect(),
        sources: (0..b.nodes.len())
            .filter(|&i| matches!(b.nodes[i].role, NodeRole::Source(_) | NodeRole::Tank(_)))
            .map(|i| ids[i].clone())
            .collect(),
    };
    let out = ds_graph::build_graph(input).map_err(|e| LayoutError::Graph(e.to_string()))?;
    let mut keep = vec![false; b.edges.len()];
    for (i, node) in b.nodes.iter().enumerate() {
        if !matches!(node.role, NodeRole::Demand | NodeRole::Junction) {
            continue;
        }
        if !out.distances.get(&ids[i]).is_some_and(|d| d.is_finite()) {
            unserved.push(node.id.clone().unwrap_or_else(|| format!("node#{i}")));
            continue;
        }
        let mut at = ids[i].clone();
        while let Some(&g) = out.tree_parent_edge_by_node.get(&at) {
            let e = alive[g];
            if keep[e] {
                break;
            }
            keep[e] = true;
            at = out.tree_parent[&at].clone();
        }
    }
    for (edge, k) in b.edges.iter_mut().zip(keep) {
        edge.alive &= k;
    }
    Ok(())
}

/// Minimum spanning tree over k-nearest-neighbour candidates (Kruskal), with
/// components joined by their closest pair until one tree remains.
fn spanning_tree(b: &mut Builder<'_>) {
    let n = b.nodes.len();
    let pts: Vec<[f64; 2]> = b.nodes.iter().map(|x| x.xy).collect();
    let mut grid = Grid::new(200.0);
    for (i, p) in pts.iter().enumerate() {
        grid.insert(*p, i);
    }
    let mut candidates: Vec<(f64, usize, usize)> = Vec::new();
    for i in 0..n {
        for (j, d) in grid.k_nearest(&pts, pts[i], b.opts.neighbours, i, 1e7) {
            if i < j {
                candidates.push((d, i, j));
            } else {
                candidates.push((d, j, i));
            }
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    candidates.dedup();
    let mut uf = UnionFind::new(n);
    let mut chosen = Vec::new();
    for &(_, i, j) in &candidates {
        if uf.union(i, j) {
            chosen.push((i, j));
        }
    }
    // Join remaining components (clusters farther apart than k neighbours).
    loop {
        let roots: HashSet<usize> = (0..n).map(|i| uf.find(i)).collect();
        if roots.len() <= 1 {
            break;
        }
        let mut best: Option<(f64, usize, usize)> = None;
        let anchor = uf.find(0);
        let inside: Vec<bool> = (0..n).map(|i| uf.find(i) == anchor).collect();
        for i in (0..n).filter(|&i| inside[i]) {
            for j in (0..n).filter(|&j| !inside[j]) {
                let d = dist(pts[i], pts[j]);
                if best.is_none_or(|b| d < b.0) {
                    best = Some((d, i, j));
                }
            }
        }
        let (_, i, j) = best.expect("two components exist");
        uf.union(i, j);
        chosen.push((i, j));
    }
    for (i, j) in chosen {
        b.edge(i, j, Vec::new(), None);
    }
}

struct UnionFind(Vec<usize>);

impl UnionFind {
    fn new(n: usize) -> Self {
        Self((0..n).collect())
    }
    fn find(&mut self, mut x: usize) -> usize {
        while self.0[x] != x {
            self.0[x] = self.0[self.0[x]];
            x = self.0[x];
        }
        x
    }
    fn union(&mut self, a: usize, b: usize) -> bool {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return false;
        }
        self.0[ra.max(rb)] = ra.min(rb);
        true
    }
}

/// Missing elevations: caller's surface, else inverse-distance from the
/// nearest known nodes, else zero (noted).
fn fill_elevations(b: &mut Builder<'_>, elevation: Option<ElevationSource<'_>>) {
    if let Some(surface) = elevation {
        for node in &mut b.nodes {
            if node.z.is_none() {
                node.z = surface(node.src);
            }
        }
    }
    let known: Vec<([f64; 2], f64)> = b
        .nodes
        .iter()
        .filter_map(|n| n.z.map(|z| (n.xy, z)))
        .collect();
    let missing = b.nodes.iter().filter(|n| n.z.is_none()).count();
    if missing == 0 {
        return;
    }
    if known.is_empty() {
        for node in &mut b.nodes {
            node.z = Some(0.0);
        }
        b.notes.push(format!("elevation_missing: {missing} nodes have no elevation; set to 0 m — give Z, elevation_m or a surface"));
        return;
    }
    for node in &mut b.nodes {
        if node.z.is_none() {
            let mut nearest: Vec<(f64, f64)> =
                known.iter().map(|(p, z)| (dist(*p, node.xy), *z)).collect();
            nearest.sort_by(|a, b| a.0.total_cmp(&b.0));
            nearest.truncate(4);
            let (mut w, mut s) = (0.0, 0.0);
            for (d, z) in nearest {
                let wi = 1.0 / (d * d).max(1e-6);
                w += wi;
                s += wi * z;
            }
            node.z = Some(s / w);
        }
    }
    b.notes
        .push(format!("elevation_interpolated: {missing} nodes"));
}

fn to_network(
    b: &Builder<'_>,
    points: &[PointFeature],
    lines: &[LineFeature],
    opts: &LayoutOptions,
) -> WaterNetwork {
    let mut net = WaterNetwork::new("layout");
    net.crs = Some(if b.proj.is_geographic() {
        "EPSG:4326".into()
    } else {
        opts.crs.clone().unwrap_or_default()
    });
    // Collapse plain degree-2 corridor nodes into pipe vertices.
    let mut degree = vec![0usize; b.nodes.len()];
    for e in b.edges.iter().filter(|e| e.alive) {
        degree[e.a] += 1;
        degree[e.b] += 1;
    }
    let keep_node = |i: usize| b.nodes[i].role != NodeRole::Plain || degree[i] != 2;
    let mut used_ids: HashSet<String> = HashSet::new();
    let mut node_id: Vec<Option<String>> = vec![None; b.nodes.len()];
    let mut counter = 0;
    for (i, node) in b.nodes.iter().enumerate() {
        if degree[i] == 0 && !matches!(node.role, NodeRole::Source(_) | NodeRole::Tank(_)) {
            continue;
        }
        if !keep_node(i) {
            continue;
        }
        let id = match &node.id {
            Some(id) if used_ids.insert(id.clone()) => id.clone(),
            _ => loop {
                counter += 1;
                let prefix = match node.role {
                    NodeRole::Source(_) => "R",
                    NodeRole::Tank(_) => "T",
                    _ => "J",
                };
                let candidate = format!("{prefix}{counter}");
                if used_ids.insert(candidate.clone()) {
                    break candidate;
                }
            },
        };
        let z = node.z.unwrap_or(0.0);
        let [x, y] = node.src;
        match node.role {
            NodeRole::Source(s) => {
                let p = points.iter().filter(|p| p.role == PointRole::Source).nth(s);
                net.reservoirs.push(Reservoir {
                    id: id.clone(),
                    head_m: p.and_then(|p| p.head_m).unwrap_or(z + opts.source_height_m),
                    x: Some(x),
                    y: Some(y),
                    ..Reservoir::default()
                });
            }
            NodeRole::Tank(t) => {
                let p = points.iter().filter(|p| p.role == PointRole::Tank).nth(t);
                let num = |k: &str, d: f64| {
                    p.and_then(|p| p.props.get(k))
                        .and_then(Value::as_f64)
                        .unwrap_or(d)
                };
                net.tanks.push(Tank {
                    id: id.clone(),
                    elevation_m: z,
                    initial_level_m: num("initial_level_m", 2.0),
                    min_level_m: num("min_level_m", 0.3),
                    max_level_m: num("max_level_m", 3.0),
                    diameter_m: num("tank_diameter_m", 6.0),
                    x: Some(x),
                    y: Some(y),
                });
            }
            _ => net.junctions.push(Junction {
                id: id.clone(),
                elevation_m: z,
                demand_lps: node.demand_lps,
                x: Some(x),
                y: Some(y),
                ..Junction::default()
            }),
        }
        node_id[i] = Some(id);
    }
    // Walk chains between kept nodes.
    let mut incident: Vec<Vec<usize>> = vec![Vec::new(); b.nodes.len()];
    for (k, e) in b.edges.iter().enumerate().filter(|(_, e)| e.alive) {
        incident[e.a].push(k);
        incident[e.b].push(k);
    }
    let mut used = vec![false; b.edges.len()];
    let mut pipe_n = 0;
    for start in 0..b.nodes.len() {
        if node_id[start].is_none() {
            continue;
        }
        for &first in &incident[start] {
            if used[first] {
                continue;
            }
            let mut vertices = Vec::new();
            let mut length = 0.0;
            let mut at = start;
            let mut edge = first;
            let mut line = None;
            loop {
                used[edge] = true;
                let e = &b.edges[edge];
                line = line.or(e.line);
                length += e.length_m;
                let (next, interior): (usize, Vec<[f64; 2]>) = if e.a == at {
                    (e.b, e.interior.clone())
                } else {
                    (e.a, e.interior.iter().rev().copied().collect())
                };
                vertices.extend(interior);
                at = next;
                if node_id[at].is_some() {
                    break;
                }
                vertices.push(b.nodes[at].src);
                match incident[at].iter().find(|&&k| !used[k]) {
                    Some(&k) => edge = k,
                    None => break,
                }
            }
            let (Some(from), Some(to)) = (node_id[start].clone(), node_id[at].clone()) else {
                continue;
            };
            let dz = b.nodes[start].z.unwrap_or(0.0) - b.nodes[at].z.unwrap_or(0.0);
            let props = line.map(|l| &lines[l].props);
            let num = |k: &str| props.and_then(|p| p.get(k)).and_then(Value::as_f64);
            pipe_n += 1;
            let line_id = line.and_then(|l| lines[l].id.clone());
            let id = match line_id {
                Some(lid) if !net.pipes.iter().any(|p| p.id == lid) => lid,
                Some(lid) => format!("{lid}-{pipe_n}"),
                None => format!("P{pipe_n}"),
            };
            net.pipes.push(Pipe {
                id,
                from,
                to,
                length_m: length.hypot(dz).max(0.1),
                diameter_mm: num("diameter_mm").unwrap_or(opts.default_diameter_mm),
                roughness: num("roughness").unwrap_or(opts.roughness),
                material: props
                    .and_then(|p| p.get("material"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                vertices,
                ..Pipe::default()
            });
        }
    }
    net
}

/// Orient the network from its sources and list its paths, trunk first.
pub fn orient(net: &WaterNetwork) -> Result<Vec<PathSummary>, LayoutError> {
    let nodes: Vec<String> = net.node_refs().map(|(id, _)| id.to_owned()).collect();
    let sources: Vec<String> = net
        .reservoirs
        .iter()
        .map(|r| r.id.clone())
        .chain(net.tanks.iter().map(|t| t.id.clone()))
        .collect();
    if sources.is_empty() {
        return Err(LayoutError::NoSource);
    }
    // Every link kind joins the tree, so a path runs through valves (a
    // break-pressure tank is one) and pumps as it does through pipes. They
    // get a nominal millimetre: the graph primitive treats a zero-length
    // edge as no connection.
    const POINT_LINK_M: f64 = 0.001;
    let links: Vec<(&str, &str, f64, &str)> = net
        .pipes
        .iter()
        .map(|p| (p.from.as_str(), p.to.as_str(), p.length_m, p.id.as_str()))
        .chain(
            net.pumps
                .iter()
                .map(|p| (p.from.as_str(), p.to.as_str(), POINT_LINK_M, p.id.as_str())),
        )
        .chain(
            net.valves
                .iter()
                .map(|v| (v.from.as_str(), v.to.as_str(), POINT_LINK_M, v.id.as_str())),
        )
        .collect();
    let input = ds_graph::BuildInput {
        nodes,
        edges: links
            .iter()
            .map(|&(a, b, length_m, _)| ds_graph::GraphEdge {
                a: a.to_owned(),
                b: b.to_owned(),
                length_m,
            })
            .collect(),
        sources,
    };
    let out = ds_graph::build_graph(input).map_err(|e| LayoutError::Graph(e.to_string()))?;
    Ok(out
        .paths
        .iter()
        .map(|p| PathSummary {
            label: p.label.clone(),
            root: p.root_node.clone(),
            tip: p.tip_node.clone(),
            length_m: p.tip_distance - p.root_distance,
            nodes: p.member_nodes.clone(),
            pipes: p
                .member_edges
                .iter()
                .map(|&e| links[e].3.to_owned())
                .collect(),
        })
        .collect())
}

#[cfg(test)]
mod tests;
