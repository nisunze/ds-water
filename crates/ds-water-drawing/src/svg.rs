//! A small SVG writer in paper millimetres. Every sheet is one `<svg>` whose
//! user unit is 1 mm, so line weights and text heights read as drawing
//! standards state them (ISO 128 line widths, ISO 3098 lettering).

// Drawing primitives take the SVG attributes they write, positionally.
#![allow(clippy::too_many_arguments)]

use std::fmt::Write as _;

pub struct Svg {
    body: String,
    defs: String,
    width_mm: f64,
    height_mm: f64,
}

/// Line weights, mm (ISO 128 series).
pub const THIN: f64 = 0.18;
pub const MEDIUM: f64 = 0.35;
pub const THICK: f64 = 0.5;
pub const HEAVY: f64 = 0.7;

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn n(x: f64) -> String {
    let r = (x * 100.0).round() / 100.0;
    if r == r.trunc() {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

#[derive(Clone, Copy)]
pub enum Anchor {
    Start,
    Middle,
    End,
}

impl Anchor {
    fn as_str(self) -> &'static str {
        match self {
            Anchor::Start => "start",
            Anchor::Middle => "middle",
            Anchor::End => "end",
        }
    }
}

impl Svg {
    pub fn new(width_mm: f64, height_mm: f64) -> Self {
        Self {
            body: String::new(),
            defs: String::new(),
            width_mm,
            height_mm,
        }
    }

    pub fn clip_rect(&mut self, id: &str, x: f64, y: f64, w: f64, h: f64) {
        let _ = write!(
            self.defs,
            r#"<clipPath id="{id}"><rect x="{}" y="{}" width="{}" height="{}"/></clipPath>"#,
            n(x),
            n(y),
            n(w),
            n(h)
        );
    }

    pub fn open_group(&mut self, attrs: &str) {
        let _ = write!(self.body, "<g {attrs}>");
    }

    pub fn close_group(&mut self) {
        self.body.push_str("</g>");
    }

    pub fn line(
        &mut self,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        stroke: &str,
        width: f64,
        dash: Option<&str>,
    ) {
        let dash = dash
            .map(|d| format!(r#" stroke-dasharray="{d}""#))
            .unwrap_or_default();
        let _ = write!(
            self.body,
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{stroke}" stroke-width="{}"{dash}/>"#,
            n(x1),
            n(y1),
            n(x2),
            n(y2),
            n(width)
        );
    }

    pub fn polyline(&mut self, points: &[[f64; 2]], stroke: &str, width: f64, dash: Option<&str>) {
        if points.len() < 2 {
            return;
        }
        let pts: Vec<String> = points
            .iter()
            .map(|p| format!("{},{}", n(p[0]), n(p[1])))
            .collect();
        let dash = dash
            .map(|d| format!(r#" stroke-dasharray="{d}""#))
            .unwrap_or_default();
        let _ = write!(
            self.body,
            r#"<polyline points="{}" fill="none" stroke="{stroke}" stroke-width="{}" stroke-linejoin="round"{dash}/>"#,
            pts.join(" "),
            n(width)
        );
    }

    pub fn polygon(&mut self, points: &[[f64; 2]], fill: &str, stroke: &str, width: f64) {
        let pts: Vec<String> = points
            .iter()
            .map(|p| format!("{},{}", n(p[0]), n(p[1])))
            .collect();
        let _ = write!(
            self.body,
            r#"<polygon points="{}" fill="{fill}" stroke="{stroke}" stroke-width="{}"/>"#,
            pts.join(" "),
            n(width)
        );
    }

    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, fill: &str, stroke: &str, width: f64) {
        let _ = write!(
            self.body,
            r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{fill}" stroke="{stroke}" stroke-width="{}"/>"#,
            n(x),
            n(y),
            n(w),
            n(h),
            n(width)
        );
    }

    pub fn circle(&mut self, cx: f64, cy: f64, r: f64, fill: &str, stroke: &str, width: f64) {
        let _ = write!(
            self.body,
            r#"<circle cx="{}" cy="{}" r="{}" fill="{fill}" stroke="{stroke}" stroke-width="{}"/>"#,
            n(cx),
            n(cy),
            n(r),
            n(width)
        );
    }

    /// Text with its baseline at `y`; `size` is the cap-ish height in mm.
    pub fn text(&mut self, x: f64, y: f64, size: f64, anchor: Anchor, weight: &str, content: &str) {
        let _ = write!(
            self.body,
            r#"<text x="{}" y="{}" font-size="{}" text-anchor="{}" font-weight="{weight}">{}</text>"#,
            n(x),
            n(y),
            n(size),
            anchor.as_str(),
            esc(content)
        );
    }

    /// Text rotated 90° anticlockwise about its anchor, reading upwards.
    pub fn text_up(&mut self, x: f64, y: f64, size: f64, anchor: Anchor, content: &str) {
        let _ = write!(
            self.body,
            r#"<text transform="translate({},{}) rotate(-90)" font-size="{}" text-anchor="{}">{}</text>"#,
            n(x),
            n(y),
            n(size),
            anchor.as_str(),
            esc(content)
        );
    }

    pub fn finish(self, title: &str) -> String {
        format!(
            concat!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}mm" height="{h}mm" viewBox="0 0 {w} {h}" "#,
                r##"font-family="DejaVu Sans, sans-serif" fill="#111"><title>{t}</title><defs>{d}</defs>"##,
                r##"<rect width="{w}" height="{h}" fill="#fff"/>{b}</svg>"##
            ),
            w = n(self.width_mm),
            h = n(self.height_mm),
            t = esc(title),
            d = self.defs,
            b = self.body
        )
    }
}
