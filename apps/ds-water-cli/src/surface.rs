//! Fixture-only ground: an ESRI ASCII grid (.asc) with bilinear
//! interpolation, so the repository's test schemes run headless without the
//! product. The product never reads rasters here: ground reaches the engine
//! as an injected surface from the existing raster/terrain plugin, the same
//! evaluator the MV profile samples.

use anyhow::{Context, Result, bail};
use ds_water_ops::GridSurface;

/// Read an ESRI ASCII grid into the engine's [`GridSurface`].
pub fn read(path: &std::path::Path) -> Result<GridSurface> {
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
    Ok(GridSurface {
        x0,
        y_top: y0 + rows as f64 * cell,
        cell,
        cols,
        rows,
        nodata: Some(get("nodata_value").unwrap_or(-9999.0)),
        z: values,
    })
}
