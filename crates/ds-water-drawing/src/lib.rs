//! Water engineering drawings.
//!
//! A drawing set for a scheme is:
//! - one **network plan** (every pipe coloured and weighted by diameter,
//!   sources, tanks and served junctions, north arrow, scale bar),
//! - **plan-and-profile sheets** for each profiled path, cut into chainage
//!   windows at a standard horizontal scale: a plan strip of the alignment
//!   on top, the profile in the middle (ground, pipe, hydraulic and static
//!   grade lines, fittings, nodes, findings; vertical exaggeration stated),
//!   and data bands below (chainage, ground level, pipe level, HGL,
//!   pressure, pipe),
//! - a **schedules** sheet (bill of quantities by material, size and class;
//!   fittings by path; design criteria).
//!
//! Sheets are ISO A-size SVG in millimetres with an ISO 5457 frame (20 mm
//! binding margin) and a title block, so they print at scale and open in any
//! drawing tool.

mod svg;

use ds_water_design::{BoqLine, DesignCriteria};
use ds_water_layout::geo::Projection;
use ds_water_model::WaterNetwork;
use ds_water_profile::{FindingKind, FittingKind, ProfileResult};
use serde::{Deserialize, Serialize};

use svg::{Anchor, HEAVY, MEDIUM, Svg, THICK, THIN};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TitleInfo {
    pub project: String,
    pub client: String,
    pub location: String,
    pub drawing_prefix: String,
    pub revision: String,
    pub date: String,
    pub drawn_by: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Paper {
    A3,
    A1,
}

impl Paper {
    fn size(self) -> (f64, f64) {
        match self {
            Paper::A3 => (420.0, 297.0),
            Paper::A1 => (841.0, 594.0),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DrawingOptions {
    pub paper: Paper,
    /// Horizontal scale denominator of plan-and-profile sheets; 0 picks the
    /// largest standard scale that fits the path on one sheet, else 1:2500.
    pub horizontal_scale: u32,
    /// Vertical scale denominator; 0 picks one that fits each window.
    pub vertical_scale: u32,
}

impl Default for DrawingOptions {
    fn default() -> Self {
        Self {
            paper: Paper::A3,
            horizontal_scale: 0,
            vertical_scale: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sheet {
    pub number: String,
    pub title: String,
    pub svg: String,
}

const H_SCALES: [u32; 8] = [500, 1000, 1250, 2000, 2500, 5000, 10_000, 20_000];
const V_SCALES: [u32; 8] = [50, 100, 200, 250, 500, 1000, 2000, 5000];
const BLUE: &str = "#0b5cad";
const SKY: &str = "#1f9bd7";
const RED: &str = "#c8102e";
const BROWN: &str = "#8a5a2b";
const GREY: &str = "#9aa3ab";

/// ISO 5457 frame and the title-block cells; returns the drawing area
/// (x0, y0, x1, y1) inside the frame.
fn frame(
    doc: &mut Svg,
    paper: Paper,
    info: &TitleInfo,
    title: &str,
    number: &str,
    scale: &str,
) -> (f64, f64, f64, f64) {
    let (w, h) = paper.size();
    let (x0, y0, x1, y1) = (20.0, 10.0, w - 10.0, h - 10.0);
    doc.rect(x0, y0, x1 - x0, y1 - y0, "none", "#111", HEAVY);
    // Centring marks.
    for (ax, ay, bx, by) in [
        (w / 2.0, 0.0, w / 2.0, y0),
        (w / 2.0, y1, w / 2.0, h),
        (0.0, h / 2.0, x0, h / 2.0),
        (x1, h / 2.0, w, h / 2.0),
    ] {
        doc.line(ax, ay, bx, by, "#111", THICK, None);
    }
    let (tw, th) = (180.0, 52.0);
    let (tx, ty) = (x1 - tw, y1 - th);
    doc.rect(tx, ty, tw, th, "#fff", "#111", THICK);
    let row = |doc: &mut Svg, y: f64| doc.line(tx, y, x1, y, "#111", THIN, None);
    row(doc, ty + 10.0);
    row(doc, ty + 24.0);
    row(doc, ty + 38.0);
    doc.line(tx + 90.0, ty + 24.0, tx + 90.0, y1, "#111", THIN, None);
    doc.line(tx + 135.0, ty + 38.0, tx + 135.0, y1, "#111", THIN, None);
    doc.text(tx + 3.0, ty + 4.2, 2.0, Anchor::Start, "normal", "PROJECT");
    doc.text(
        tx + 3.0,
        ty + 8.6,
        3.2,
        Anchor::Start,
        "bold",
        &info.project,
    );
    doc.text(
        tx + tw - 3.0,
        ty + 8.6,
        2.5,
        Anchor::End,
        "normal",
        &info.client,
    );
    doc.text(tx + 3.0, ty + 14.2, 2.0, Anchor::Start, "normal", "DRAWING");
    doc.text(tx + 3.0, ty + 20.5, 3.5, Anchor::Start, "bold", title);
    doc.text(
        tx + 3.0,
        ty + 28.2,
        2.0,
        Anchor::Start,
        "normal",
        "LOCATION",
    );
    doc.text(
        tx + 3.0,
        ty + 34.0,
        2.8,
        Anchor::Start,
        "normal",
        &info.location,
    );
    doc.text(tx + 93.0, ty + 28.2, 2.0, Anchor::Start, "normal", "SCALE");
    doc.text(tx + 93.0, ty + 34.0, 2.8, Anchor::Start, "normal", scale);
    doc.text(
        tx + 3.0,
        ty + 42.2,
        2.0,
        Anchor::Start,
        "normal",
        "DRAWING No.",
    );
    doc.text(tx + 3.0, ty + 48.5, 3.2, Anchor::Start, "bold", number);
    doc.text(
        tx + 93.0,
        ty + 42.2,
        2.0,
        Anchor::Start,
        "normal",
        "DATE / DRAWN",
    );
    doc.text(
        tx + 93.0,
        ty + 48.5,
        2.5,
        Anchor::Start,
        "normal",
        &format!("{} {}", info.date, info.drawn_by),
    );
    doc.text(tx + 138.0, ty + 42.2, 2.0, Anchor::Start, "normal", "REV");
    doc.text(
        tx + 138.0,
        ty + 48.5,
        3.2,
        Anchor::Start,
        "bold",
        &info.revision,
    );
    doc.text(
        x1 - 2.0,
        y1 + 6.0,
        2.0,
        Anchor::End,
        "normal",
        "Data Solutions · Water",
    );
    (x0, y0, x1, y1)
}

fn chainage_label(c: f64) -> String {
    let km = (c / 1000.0).floor();
    format!("{}+{:05.1}", km as i64, c - km * 1000.0)
}

fn tick_step(span_m: f64, target: f64) -> f64 {
    let raw = span_m / target;
    [
        10.0, 20.0, 25.0, 50.0, 100.0, 200.0, 250.0, 500.0, 1000.0, 2000.0, 5000.0,
    ]
    .into_iter()
    .find(|&s| s >= raw)
    .unwrap_or(10_000.0)
}

/// Plan-and-profile sheets for one path.
pub fn plan_profile_sheets(
    net: &WaterNetwork,
    profile: &ProfileResult,
    path_label: &str,
    info: &TitleInfo,
    opts: &DrawingOptions,
    first_number: usize,
) -> Vec<Sheet> {
    let (w, _) = opts.paper.size();
    let left = 64.0;
    let right = w - 14.0;
    let width_mm = right - left;
    let h_scale = if opts.horizontal_scale > 0 {
        opts.horizontal_scale
    } else {
        H_SCALES
            .into_iter()
            .find(|&s| width_mm * s as f64 / 1000.0 >= profile.length_m)
            .unwrap_or(2500)
            .min(2500)
    } as f64;
    let window_m = width_mm * h_scale / 1000.0;
    let count = ((profile.length_m / window_m).ceil() as usize).max(1);
    let all_xy: Vec<[f64; 2]> = profile.points.iter().map(|p| p.at).collect();
    let proj = Projection::for_points(&all_xy, net.crs.as_deref());
    (0..count)
        .map(|i| {
            let c0 = i as f64 * window_m;
            let c1 = (c0 + window_m).min(profile.length_m);
            let number = format!(
                "{}-{:03}",
                if info.drawing_prefix.is_empty() {
                    "WS"
                } else {
                    &info.drawing_prefix
                },
                first_number + i
            );
            let title = format!(
                "Path {path_label} plan & profile, Ch {} – {}",
                chainage_label(c0),
                chainage_label(c1)
            );
            let svg = window_sheet(
                net,
                profile,
                &proj,
                info,
                opts,
                h_scale,
                c0,
                c1,
                &title,
                &number,
                i + 1,
                count,
            );
            Sheet { number, title, svg }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn window_sheet(
    net: &WaterNetwork,
    profile: &ProfileResult,
    proj: &Projection,
    info: &TitleInfo,
    opts: &DrawingOptions,
    h_scale: f64,
    c0: f64,
    c1: f64,
    title: &str,
    number: &str,
    sheet: usize,
    sheets: usize,
) -> String {
    let (w, h) = opts.paper.size();
    let mut doc = Svg::new(w, h);
    let left = 64.0;
    let right = w - 14.0;
    let x_of = |c: f64| left + (c - c0) * 1000.0 / h_scale;
    let in_window: Vec<&ds_water_profile::ProfilePoint> = profile
        .points
        .iter()
        .filter(|p| p.chainage_m >= c0 - 1e-6 && p.chainage_m <= c1 + 1e-6)
        .collect();

    // Vertical scale and datum for this window.
    let zs = in_window
        .iter()
        .flat_map(|p| [p.ground_m, p.centre_m, p.hgl_m.max(p.centre_m - 50.0)]);
    let (zmin, zmax) = zs.fold((f64::MAX, f64::MIN), |(a, b), z| (a.min(z), b.max(z)));
    let (p_top, p_bottom) = (92.0, 176.0);
    let usable = p_bottom - p_top - 8.0;
    let v_scale = if opts.vertical_scale > 0 {
        opts.vertical_scale as f64
    } else {
        V_SCALES
            .into_iter()
            .find(|&s| usable * s as f64 / 1000.0 >= (zmax - zmin) * 1.05)
            .unwrap_or(5000) as f64
    };
    let z_step = tick_step(usable * v_scale / 1000.0, 8.0).max(1.0);
    let datum = (zmin / z_step).floor() * z_step - z_step * 0.0;
    let y_of = |z: f64| p_bottom - 4.0 - (z - datum) * 1000.0 / v_scale;

    let scale_text = format!(
        "H 1:{} · V 1:{} (×{:.0})",
        h_scale as u32,
        v_scale as u32,
        h_scale / v_scale
    );
    let (_, fy0, _, _) = frame(&mut doc, opts.paper, info, title, number, &scale_text);
    doc.text(
        right,
        fy0 + 5.0,
        2.5,
        Anchor::End,
        "normal",
        &format!("Sheet {sheet} of {sheets}"),
    );

    // ── Plan strip ─────────────────────────────────────────────────────
    let (s_top, s_bottom) = (fy0 + 7.0, 84.0);
    doc.rect(
        left,
        s_top,
        right - left,
        s_bottom - s_top,
        "none",
        "#111",
        THIN,
    );
    doc.text(left - 2.0, s_top + 4.0, 2.5, Anchor::End, "bold", "PLAN");
    let metric: Vec<[f64; 2]> = in_window.iter().map(|p| proj.forward(p.at)).collect();
    if metric.len() >= 2 {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in &metric {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        let (bw, bh) = (right - left - 20.0, s_bottom - s_top - 10.0);
        let need = ((hi[0] - lo[0]) / bw).max((hi[1] - lo[1]) / bh) * 1000.0;
        let plan_scale = H_SCALES
            .into_iter()
            .map(f64::from)
            .find(|&s| s >= need)
            .unwrap_or(need.ceil().max(1.0));
        let (cx, cy) = ((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0);
        let (mx, my) = ((left + right) / 2.0, (s_top + s_bottom) / 2.0);
        let to_paper = |p: [f64; 2]| {
            [
                mx + (p[0] - cx) * 1000.0 / plan_scale,
                my - (p[1] - cy) * 1000.0 / plan_scale,
            ]
        };
        doc.clip_rect(
            &format!("plan{sheet}"),
            left,
            s_top,
            right - left,
            s_bottom - s_top,
        );
        doc.open_group(&format!(r#"clip-path="url(#plan{sheet})""#));
        // Neighbouring network in grey.
        for pipe in &net.pipes {
            let ends: Vec<[f64; 2]> = [pipe.from.as_str(), pipe.to.as_str()]
                .iter()
                .filter_map(|id| {
                    net.node_refs()
                        .find(|(n, _)| n == id)
                        .and_then(|(_, r)| net.node_xy(r))
                })
                .collect();
            if ends.len() == 2 {
                let mut line = vec![ends[0]];
                line.extend(pipe.vertices.iter().copied());
                line.push(ends[1]);
                let paper: Vec<[f64; 2]> =
                    line.iter().map(|&p| to_paper(proj.forward(p))).collect();
                doc.polyline(&paper, GREY, THIN, None);
            }
        }
        let route: Vec<[f64; 2]> = metric.iter().map(|&p| to_paper(p)).collect();
        doc.polyline(&route, BLUE, HEAVY, None);
        let step = tick_step(c1 - c0, 10.0);
        let mut c = (c0 / step).ceil() * step;
        while c <= c1 + 1e-6 {
            if let Some(i) = in_window.iter().position(|p| p.chainage_m >= c - 1e-6) {
                let p = route[i];
                doc.circle(p[0], p[1], 0.5, "#111", "none", 0.0);
                doc.text(
                    p[0],
                    p[1] - 1.5,
                    1.8,
                    Anchor::Middle,
                    "normal",
                    &chainage_label(c),
                );
            }
            c += step;
        }
        for node in profile
            .nodes
            .iter()
            .filter(|n| n.chainage_m >= c0 - 1e-6 && n.chainage_m <= c1 + 1e-6)
        {
            if let Some(i) = in_window
                .iter()
                .position(|p| (p.chainage_m - node.chainage_m).abs() < 1e-6)
            {
                let p = route[i];
                doc.circle(p[0], p[1], 1.0, "#fff", BLUE, MEDIUM);
                doc.text(p[0] + 1.6, p[1] + 3.0, 2.0, Anchor::Start, "bold", &node.id);
            }
        }
        doc.close_group();
        north_arrow(&mut doc, right - 8.0, s_top + 6.0);
        doc.text(
            left + 2.0,
            s_bottom - 2.0,
            2.0,
            Anchor::Start,
            "normal",
            &format!("Plan 1:{}", plan_scale as u32),
        );
    }

    // ── Profile ────────────────────────────────────────────────────────
    doc.rect(
        left,
        p_top,
        right - left,
        p_bottom - p_top,
        "none",
        "#111",
        THIN,
    );
    doc.text(left - 2.0, p_top + 4.0, 2.5, Anchor::End, "bold", "PROFILE");
    let mut z = datum;
    while y_of(z) >= p_top + 2.0 {
        doc.line(left, y_of(z), right, y_of(z), "#d5dade", THIN, None);
        doc.text(
            left - 2.0,
            y_of(z) + 0.8,
            2.0,
            Anchor::End,
            "normal",
            &format!("{z:.0}"),
        );
        z += z_step;
    }
    doc.text(
        left - 2.0,
        p_bottom - 0.5,
        1.8,
        Anchor::End,
        "normal",
        &format!("Datum {datum:.0}"),
    );
    let step = tick_step(c1 - c0, 10.0);
    let mut c = (c0 / step).ceil() * step;
    while c <= c1 + 1e-6 {
        doc.line(x_of(c), p_top, x_of(c), p_bottom, "#d5dade", THIN, None);
        c += step;
    }
    doc.clip_rect(
        &format!("prof{sheet}"),
        left,
        p_top,
        right - left,
        p_bottom - p_top,
    );
    doc.open_group(&format!(r#"clip-path="url(#prof{sheet})""#));
    for f in profile
        .findings
        .iter()
        .filter(|f| f.to_m >= c0 && f.from_m <= c1)
    {
        let color = match f.kind {
            FindingKind::NegativePressure => "#f5b7b1",
            FindingKind::LowPressure => "#fde2b8",
            FindingKind::OverClass | FindingKind::OverStaticLimit => "#f9d3e0",
        };
        let (a, b) = (
            x_of(f.from_m.max(c0)),
            x_of(f.to_m.min(c1)).max(x_of(f.from_m.max(c0)) + 0.6),
        );
        doc.rect(a, p_top, b - a, 3.0, color, "none", 0.0);
    }
    let line = |doc: &mut Svg,
                z: &dyn Fn(&ds_water_profile::ProfilePoint) -> f64,
                color: &str,
                width: f64,
                dash: Option<&str>| {
        let pts: Vec<[f64; 2]> = in_window
            .iter()
            .map(|p| [x_of(p.chainage_m), y_of(z(p))])
            .collect();
        doc.polyline(&pts, color, width, dash);
    };
    line(&mut doc, &|p| p.ground_m, BROWN, MEDIUM, None);
    line(&mut doc, &|p| p.static_m, RED, THIN, Some("4 1 1 1"));
    line(&mut doc, &|p| p.hgl_m, SKY, MEDIUM, Some("3 1.5"));
    line(&mut doc, &|p| p.centre_m, BLUE, HEAVY, None);
    for node in profile
        .nodes
        .iter()
        .filter(|n| n.chainage_m >= c0 - 1e-6 && n.chainage_m <= c1 + 1e-6)
    {
        let x = x_of(node.chainage_m);
        doc.line(x, p_top + 3.0, x, p_bottom, "#555", THIN, Some("1 1"));
        doc.text_up(
            x - 0.8,
            p_top + 4.0,
            2.0,
            Anchor::End,
            &format!("{}{}", node.id, if node.tee { " (tee)" } else { "" }),
        );
    }
    for f in profile
        .fittings
        .iter()
        .filter(|f| f.chainage_m >= c0 - 1e-6 && f.chainage_m <= c1 + 1e-6)
    {
        let x = x_of(f.chainage_m);
        let at = in_window
            .iter()
            .min_by(|a, b| {
                (a.chainage_m - f.chainage_m)
                    .abs()
                    .total_cmp(&(b.chainage_m - f.chainage_m).abs())
            })
            .map(|p| y_of(p.centre_m))
            .unwrap_or(p_bottom);
        fitting_symbol(&mut doc, f.kind, x, at);
    }
    doc.close_group();
    profile_legend(&mut doc, left, p_bottom + 1.0);

    // ── Data bands ─────────────────────────────────────────────────────
    let rows = [
        "CHAINAGE",
        "GROUND LEVEL",
        "PIPE LEVEL (C/L)",
        "HGL (PEAK)",
        "PRESSURE m",
        "PIPE",
    ];
    let (b_top, row_h) = (p_bottom + 5.0, 8.0);
    for (r, name) in rows.iter().enumerate() {
        let y = b_top + r as f64 * row_h;
        doc.rect(20.0 + 4.0, y, right - 24.0, row_h, "none", "#111", THIN);
        doc.text(26.0, y + 5.2, 2.2, Anchor::Start, "bold", name);
    }
    doc.line(
        left,
        b_top,
        left,
        b_top + rows.len() as f64 * row_h,
        "#111",
        THIN,
        None,
    );
    let mut stations: Vec<f64> = Vec::new();
    let mut c = (c0 / step).ceil() * step;
    while c <= c1 + 1e-6 {
        stations.push(c);
        c += step;
    }
    stations.extend(
        profile
            .nodes
            .iter()
            .map(|n| n.chainage_m)
            .filter(|&c| c >= c0 - 1e-6 && c <= c1 + 1e-6),
    );
    stations.sort_by(f64::total_cmp);
    stations.dedup_by(|a, b| (*a - *b).abs() * 1000.0 / h_scale < 3.2);
    let at = |c: f64| {
        in_window
            .iter()
            .min_by(|a, b| {
                (a.chainage_m - c)
                    .abs()
                    .total_cmp(&(b.chainage_m - c).abs())
            })
            .copied()
    };
    for &c in &stations {
        let Some(p) = at(c) else { continue };
        let x = x_of(c);
        let values = [
            chainage_label(c),
            format!("{:.2}", p.ground_m),
            format!("{:.2}", p.centre_m),
            format!("{:.2}", p.hgl_m),
            format!("{:.1}", p.pressure_m),
        ];
        for (r, v) in values.iter().enumerate() {
            let y = b_top + (r as f64 + 1.0) * row_h - 0.8;
            doc.text_up(x + 0.7, y, 1.8, Anchor::Start, v);
        }
    }
    let pipe_row = b_top + 5.0 * row_h;
    for seg in profile
        .segments
        .iter()
        .filter(|s| s.end_m > c0 && s.start_m < c1)
    {
        let (a, b) = (x_of(seg.start_m.max(c0)), x_of(seg.end_m.min(c1)));
        doc.line(a, pipe_row, a, pipe_row + row_h, "#111", THIN, None);
        let label = format!(
            "{} ID{:.0} {} · {:.2} L/s · {:.2} m/s",
            seg.material.as_deref().unwrap_or("pipe"),
            seg.internal_mm,
            seg.pn_bar.map(|p| format!("PN{p:.0}")).unwrap_or_default(),
            seg.flow_lps,
            seg.velocity_ms
        );
        if b - a > 30.0 {
            doc.text(
                (a + b) / 2.0,
                pipe_row + 5.2,
                2.0,
                Anchor::Middle,
                "normal",
                &label,
            );
        } else if b - a > 6.0 {
            doc.text(
                (a + b) / 2.0,
                pipe_row + 5.2,
                1.8,
                Anchor::Middle,
                "normal",
                &format!("ID{:.0}", seg.internal_mm),
            );
        }
    }

    // Notes beside the title block.
    let notes_y = b_top + rows.len() as f64 * row_h + 5.0;
    doc.text(26.0, notes_y, 2.2, Anchor::Start, "bold", "NOTES");
    let notes = [
        "1. Levels in metres above the project datum; chainage along the pipe in metres.",
        "2. Pipe level is the pipe centreline at the stated cover; HGL at peak demand, static line at zero demand.",
        "3. AV air valve at summits, WO washout at sags, IV isolation valve, BPT break-pressure tank.",
        &format!("4. Ground from {}.", profile.ground_source),
    ];
    for (i, note) in notes.iter().enumerate() {
        doc.text(
            26.0,
            notes_y + 4.5 + i as f64 * 4.0,
            2.0,
            Anchor::Start,
            "normal",
            note,
        );
    }
    doc.finish(title)
}

fn north_arrow(doc: &mut Svg, x: f64, y: f64) {
    doc.polygon(
        &[
            [x, y - 4.0],
            [x + 2.0, y + 3.0],
            [x, y + 1.5],
            [x - 2.0, y + 3.0],
        ],
        "#111",
        "#111",
        THIN,
    );
    doc.text(x, y - 5.0, 2.5, Anchor::Middle, "bold", "N");
}

fn fitting_symbol(doc: &mut Svg, kind: FittingKind, x: f64, y: f64) {
    match kind {
        FittingKind::AirValve => {
            doc.line(x, y, x, y - 6.0, "#111", THIN, None);
            doc.circle(x, y - 7.2, 1.2, "#fff", "#111", MEDIUM);
            doc.text(x, y - 9.4, 2.0, Anchor::Middle, "bold", "AV");
        }
        FittingKind::Washout => {
            doc.line(x, y, x, y + 4.0, "#111", THIN, None);
            doc.polygon(
                &[[x - 1.3, y + 4.0], [x + 1.3, y + 4.0], [x, y + 6.0]],
                "#111",
                "#111",
                THIN,
            );
            doc.text(x, y + 8.6, 2.0, Anchor::Middle, "bold", "WO");
        }
        FittingKind::IsolationValve => {
            doc.polygon(
                &[
                    [x - 1.4, y - 1.0],
                    [x + 1.4, y + 1.0],
                    [x + 1.4, y - 1.0],
                    [x - 1.4, y + 1.0],
                ],
                "#fff",
                "#111",
                MEDIUM,
            );
            doc.text(x, y + 3.6, 1.8, Anchor::Middle, "normal", "IV");
        }
        FittingKind::BreakPressureTank => {
            doc.rect(x - 2.0, y - 4.0, 4.0, 4.0, "#e8f3fb", "#111", MEDIUM);
            doc.text(x, y - 5.0, 2.0, Anchor::Middle, "bold", "BPT");
        }
    }
}

fn profile_legend(doc: &mut Svg, x: f64, y: f64) {
    let mut at = x;
    for (label, color, width, dash) in [
        ("Ground", BROWN, MEDIUM, None),
        ("Pipe centreline", BLUE, HEAVY, None),
        ("HGL (peak)", SKY, MEDIUM, Some("3 1.5")),
        ("Static line", RED, THIN, Some("4 1 1 1")),
    ] {
        doc.line(at, y + 1.5, at + 8.0, y + 1.5, color, width, dash);
        doc.text(at + 9.5, y + 2.3, 2.0, Anchor::Start, "normal", label);
        at += 38.0;
    }
}

/// Diameter classes for the network plan: (up to mm, colour, line weight).
const CLASSES: [(f64, &str, f64); 6] = [
    (50.0, "#7fb3d5", 0.25),
    (80.0, "#2e86c1", 0.35),
    (150.0, "#1a5276", 0.5),
    (250.0, "#7d3c98", 0.7),
    (400.0, "#c0392b", 1.0),
    (f64::MAX, "#17202a", 1.4),
];

/// The whole network on one sheet.
pub fn network_plan_sheet(
    net: &WaterNetwork,
    info: &TitleInfo,
    opts: &DrawingOptions,
    number: &str,
) -> Sheet {
    let (w, h) = opts.paper.size();
    let mut doc = Svg::new(w, h);
    let title = "Water network general layout";
    let refs: Vec<_> = net.node_refs().collect();
    let all: Vec<[f64; 2]> = refs.iter().filter_map(|(_, r)| net.node_xy(*r)).collect();
    let proj = Projection::for_points(&all, net.crs.as_deref());
    let metric: Vec<[f64; 2]> = all.iter().map(|&p| proj.forward(p)).collect();
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in &metric {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    let (bx0, by0, bx1, by1) = (26.0, 16.0, w - 16.0, h - 72.0);
    let need = ((hi[0] - lo[0]).max(1.0) / (bx1 - bx0)).max((hi[1] - lo[1]).max(1.0) / (by1 - by0))
        * 1000.0;
    let nice = [
        500.0,
        1000.0,
        2000.0,
        2500.0,
        5000.0,
        10_000.0,
        25_000.0,
        50_000.0,
        100_000.0,
        250_000.0,
        500_000.0,
        1_000_000.0,
    ];
    let scale = nice.into_iter().find(|&s| s >= need).unwrap_or(need.ceil());
    let scale_text = format!("1:{}", scale as u64);
    frame(&mut doc, opts.paper, info, title, number, &scale_text);
    let (cx, cy) = ((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0);
    let (mx, my) = ((bx0 + bx1) / 2.0, (by0 + by1) / 2.0);
    let to_paper = |p: [f64; 2]| {
        let q = proj.forward(p);
        [
            mx + (q[0] - cx) * 1000.0 / scale,
            my - (q[1] - cy) * 1000.0 / scale,
        ]
    };
    let index: std::collections::HashMap<&str, [f64; 2]> = refs
        .iter()
        .filter_map(|(id, r)| net.node_xy(*r).map(|p| (*id, p)))
        .collect();
    for pipe in &net.pipes {
        let (Some(&a), Some(&b)) = (index.get(pipe.from.as_str()), index.get(pipe.to.as_str()))
        else {
            continue;
        };
        let mut line = vec![a];
        line.extend(pipe.vertices.iter().copied());
        line.push(b);
        let paper: Vec<[f64; 2]> = line.into_iter().map(to_paper).collect();
        let (_, color, weight) = CLASSES
            .iter()
            .find(|c| pipe.diameter_mm <= c.0)
            .copied()
            .unwrap_or(CLASSES[5]);
        doc.polyline(&paper, color, weight, None);
    }
    for j in &net.junctions {
        if j.demand_lps > 0.0
            && let (Some(x), Some(y)) = (j.x, j.y)
        {
            let p = to_paper([x, y]);
            doc.circle(p[0], p[1], 0.45, "#111", "none", 0.0);
        }
    }
    for r in &net.reservoirs {
        if let (Some(x), Some(y)) = (r.x, r.y) {
            let p = to_paper([x, y]);
            doc.polygon(
                &[
                    [p[0], p[1] - 2.2],
                    [p[0] + 2.0, p[1] + 1.4],
                    [p[0] - 2.0, p[1] + 1.4],
                ],
                "#0b5cad",
                "#111",
                THIN,
            );
            doc.text(
                p[0] + 2.6,
                p[1] + 1.0,
                2.2,
                Anchor::Start,
                "bold",
                &format!("{} ({:.1} m)", r.id, r.head_m),
            );
        }
    }
    for t in &net.tanks {
        if let (Some(x), Some(y)) = (t.x, t.y) {
            let p = to_paper([x, y]);
            doc.rect(p[0] - 1.8, p[1] - 1.8, 3.6, 3.6, "#d6eaf8", "#111", MEDIUM);
            doc.text(p[0] + 2.6, p[1] + 1.0, 2.2, Anchor::Start, "bold", &t.id);
        }
    }
    north_arrow(&mut doc, bx1 - 6.0, by0 + 8.0);
    // Scale bar: a round length near 50 mm of paper.
    let bar_m = tick_step(50.0 * scale / 1000.0, 1.0);
    let bar_mm = bar_m * 1000.0 / scale;
    let (sx, sy) = (bx0 + 2.0, by1 + 6.0);
    for i in 0..4 {
        let seg = bar_mm / 4.0;
        doc.rect(
            sx + i as f64 * seg,
            sy,
            seg,
            1.5,
            if i % 2 == 0 { "#111" } else { "#fff" },
            "#111",
            THIN,
        );
    }
    doc.text(sx, sy + 5.0, 2.0, Anchor::Middle, "normal", "0");
    doc.text(
        sx + bar_mm,
        sy + 5.0,
        2.0,
        Anchor::Middle,
        "normal",
        &format!("{bar_m:.0} m"),
    );
    // Legend.
    let (lx, mut ly) = (sx, sy + 12.0);
    doc.text(
        lx,
        ly,
        2.5,
        Anchor::Start,
        "bold",
        "PIPES BY INTERNAL DIAMETER",
    );
    let mut lower = 0.0;
    for (upper, color, weight) in CLASSES {
        ly += 4.5;
        doc.line(lx, ly - 0.8, lx + 10.0, ly - 0.8, color, weight, None);
        let label = if upper == f64::MAX {
            format!("over {lower:.0} mm")
        } else {
            format!("{lower:.0} – {upper:.0} mm")
        };
        doc.text(lx + 12.0, ly, 2.0, Anchor::Start, "normal", &label);
        lower = upper;
    }
    let total: f64 = net.pipes.iter().map(|p| p.length_m).sum();
    doc.text(
        lx + 70.0,
        sy + 16.5,
        2.2,
        Anchor::Start,
        "normal",
        &format!("{} pipes, {:.2} km", net.pipes.len(), total / 1000.0),
    );
    doc.text(
        lx + 70.0,
        sy + 21.0,
        2.2,
        Anchor::Start,
        "normal",
        &format!(
            "{} junctions, demand {:.2} L/s",
            net.junctions.len(),
            net.total_base_demand_lps()
        ),
    );
    doc.text(
        lx + 70.0,
        sy + 25.5,
        2.2,
        Anchor::Start,
        "normal",
        &format!(
            "{} sources, {} tanks",
            net.reservoirs.len(),
            net.tanks.len()
        ),
    );
    Sheet {
        number: number.to_owned(),
        title: title.to_owned(),
        svg: doc.finish(title),
    }
}

/// Bill of quantities, fittings and design criteria.
pub fn schedules_sheet(
    boq: &[BoqLine],
    fittings: &[(String, &ProfileResult)],
    criteria: &DesignCriteria,
    info: &TitleInfo,
    opts: &DrawingOptions,
    number: &str,
) -> Sheet {
    let (w, h) = opts.paper.size();
    let mut doc = Svg::new(w, h);
    let title = "Schedules: pipes, fittings, design criteria";
    frame(&mut doc, opts.paper, info, title, number, "NTS");
    let (x, mut y) = (28.0, 22.0);
    doc.text(
        x,
        y,
        3.0,
        Anchor::Start,
        "bold",
        "BILL OF QUANTITIES — PIPES",
    );
    y += 6.0;
    let cols = [0.0, 60.0, 90.0, 110.0, 140.0];
    for (c, head) in cols.iter().zip([
        "Material",
        "Nominal mm",
        "PN bar",
        "Length m",
        "Cost (indicative)",
    ]) {
        doc.text(x + c, y, 2.2, Anchor::Start, "bold", head);
    }
    doc.line(x, y + 1.5, x + 175.0, y + 1.5, "#111", THIN, None);
    let mut total_len = 0.0;
    let mut total_cost = 0.0;
    for line in boq {
        y += 4.5;
        let cells = [
            line.material.clone(),
            format!("{:.0}", line.nominal_mm),
            format!("{:.0}", line.pn_bar),
            format!("{:.1}", line.length_m),
            format!("{:.0}", line.cost),
        ];
        for (c, v) in cols.iter().zip(cells) {
            doc.text(x + c, y, 2.2, Anchor::Start, "normal", &v);
        }
        total_len += line.length_m;
        total_cost += line.cost;
    }
    doc.line(x, y + 1.5, x + 175.0, y + 1.5, "#111", THIN, None);
    y += 5.0;
    doc.text(x, y, 2.2, Anchor::Start, "bold", "Total");
    doc.text(
        x + cols[3],
        y,
        2.2,
        Anchor::Start,
        "bold",
        &format!("{total_len:.1}"),
    );
    doc.text(
        x + cols[4],
        y,
        2.2,
        Anchor::Start,
        "bold",
        &format!("{total_cost:.0}"),
    );

    let (fx, mut fy) = (x + 200.0, 22.0);
    doc.text(fx, fy, 3.0, Anchor::Start, "bold", "FITTINGS BY PATH");
    fy += 6.0;
    for (c, head) in [0.0, 22.0, 52.0, 80.0]
        .iter()
        .zip(["Path", "Fitting", "Chainage", "Level"])
    {
        doc.text(fx + c, fy, 2.2, Anchor::Start, "bold", head);
    }
    doc.line(fx, fy + 1.5, fx + 170.0, fy + 1.5, "#111", THIN, None);
    'paths: for (label, profile) in fittings {
        for f in &profile.fittings {
            fy += 4.2;
            if fy > h - 72.0 {
                doc.text(
                    fx,
                    fy,
                    2.0,
                    Anchor::Start,
                    "normal",
                    "… continued in the data file",
                );
                break 'paths;
            }
            let kind = match f.kind {
                FittingKind::AirValve => "Air valve",
                FittingKind::Washout => "Washout",
                FittingKind::IsolationValve => "Isolation valve",
                FittingKind::BreakPressureTank => "Break-pressure tank",
            };
            for (c, v) in [0.0, 22.0, 52.0, 80.0].iter().zip([
                label.clone(),
                kind.to_owned(),
                chainage_label(f.chainage_m),
                format!("{:.2}", f.elevation_m),
            ]) {
                doc.text(fx + c, fy, 2.0, Anchor::Start, "normal", &v);
            }
        }
    }

    let mut cy = y + 16.0;
    doc.text(x, cy, 3.0, Anchor::Start, "bold", "DESIGN CRITERIA");
    for line in [
        format!(
            "Minimum residual pressure at peak: {:.0} m",
            criteria.min_pressure_m
        ),
        format!(
            "Maximum static pressure before PRV/BPT: {:.0} m",
            criteria.max_static_pressure_m
        ),
        format!(
            "Velocity: at most {:.1} m/s; flagged below {:.1} m/s",
            criteria.max_velocity_ms, criteria.min_velocity_ms
        ),
        format!(
            "Pressure class: static pressure + {:.0} % surge allowance",
            criteria.surge_allowance * 100.0
        ),
        "Head loss: Hazen-Williams (or Darcy-Weisbach where the model says so)".to_owned(),
        "Sizing: cost-ordered upgrades along each critical supply path".to_owned(),
    ] {
        cy += 4.5;
        doc.text(x, cy, 2.2, Anchor::Start, "normal", &line);
    }
    Sheet {
        number: number.to_owned(),
        title: title.to_owned(),
        svg: doc.finish(title),
    }
}

#[cfg(test)]
mod tests;
