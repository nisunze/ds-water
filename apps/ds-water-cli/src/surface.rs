//! Fixture-only ground: an ESRI ASCII grid (.asc) with bilinear
//! interpolation, so the repository's test schemes run headless without the
//! product. The product never reads rasters here: ground reaches the engine
//! as an injected surface from the existing raster/terrain plugin, the same
//! evaluator the MV profile samples.

use anyhow::{Context, Result, bail};

pub struct AsciiGrid {
    cols: usize,
    rows: usize,
    x0: f64,
    /// Top edge (y of the first row's upper boundary).
    y_top: f64,
    cell: f64,
    nodata: f64,
    z: Vec<f64>,
}

impl AsciiGrid {
    pub fn read(path: &std::path::Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut header = std::collections::HashMap::new();
        let mut values = Vec::new();
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            let Some(first) = parts.next() else { continue };
            if first
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
            {
                let value: f64 = parts.next().context("header value")?.parse()?;
                header.insert(first.to_ascii_lowercase(), value);
            } else {
                values.push(first.parse::<f64>()?);
                for p in parts {
                    values.push(p.parse::<f64>()?);
                }
            }
        }
        let get = |k: &str| header.get(k).copied();
        let cols = get("ncols").context("ncols")? as usize;
        let rows = get("nrows").context("nrows")? as usize;
        let cell = get("cellsize").context("cellsize")?;
        let x0 = get("xllcorner")
            .or_else(|| get("xllcenter").map(|c| c - cell / 2.0))
            .context("xllcorner")?;
        let y0 = get("yllcorner")
            .or_else(|| get("yllcenter").map(|c| c - cell / 2.0))
            .context("yllcorner")?;
        if values.len() != cols * rows {
            bail!("grid holds {} values for {cols}×{rows}", values.len());
        }
        Ok(Self {
            cols,
            rows,
            x0,
            y_top: y0 + rows as f64 * cell,
            cell,
            nodata: get("nodata_value").unwrap_or(-9999.0),
            z: values,
        })
    }

    pub fn at(&self, p: [f64; 2]) -> Option<f64> {
        let fx = (p[0] - self.x0) / self.cell - 0.5;
        let fy = (self.y_top - p[1]) / self.cell - 0.5;
        if fx < -0.5 || fy < -0.5 || fx > self.cols as f64 - 0.5 || fy > self.rows as f64 - 0.5 {
            return None;
        }
        let (c0, r0) = (fx.floor().max(0.0) as usize, fy.floor().max(0.0) as usize);
        let (c1, r1) = ((c0 + 1).min(self.cols - 1), (r0 + 1).min(self.rows - 1));
        let (tx, ty) = (
            (fx - c0 as f64).clamp(0.0, 1.0),
            (fy - r0 as f64).clamp(0.0, 1.0),
        );
        let v = |c: usize, r: usize| {
            let z = self.z[r * self.cols + c];
            (z != self.nodata).then_some(z)
        };
        let (a, b, c, d) = (v(c0, r0)?, v(c1, r0)?, v(c0, r1)?, v(c1, r1)?);
        Some((a * (1.0 - tx) + b * tx) * (1.0 - ty) + (c * (1.0 - tx) + d * tx) * ty)
    }
}
