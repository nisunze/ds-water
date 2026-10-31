//! Plane geometry for layout: a local metric projection and a uniform grid
//! index. Networks span at most a country, so an equirectangular projection
//! about the data's mean latitude keeps lengths within 0.1 % for Rwanda-sized
//! extents; pipe lengths themselves are measured on the sphere.

use std::collections::HashMap;

const EARTH_RADIUS_M: f64 = 6_371_008.8;

#[derive(Clone, Copy, Debug)]
pub struct Projection {
    geographic: bool,
    lon0: f64,
    lat0: f64,
    cos_lat0: f64,
}

impl Projection {
    /// Geographic when every coordinate is a plausible lon/lat and the CRS
    /// does not say otherwise.
    pub fn for_points(points: &[[f64; 2]], crs: Option<&str>) -> Self {
        let looks_geographic = points
            .iter()
            .all(|p| p[0].abs() <= 180.0 && p[1].abs() <= 90.0);
        let declared = crs.map(|c| c.to_ascii_uppercase());
        let geographic = match declared.as_deref() {
            Some("EPSG:4326" | "OGC:CRS84" | "WGS84") => true,
            Some(_) => false,
            None => looks_geographic,
        };
        let n = points.len().max(1) as f64;
        let lon0 = points.iter().map(|p| p[0]).sum::<f64>() / n;
        let lat0 = points.iter().map(|p| p[1]).sum::<f64>() / n;
        Self {
            geographic,
            lon0,
            lat0,
            cos_lat0: lat0.to_radians().cos(),
        }
    }

    pub fn is_geographic(&self) -> bool {
        self.geographic
    }

    pub fn forward(&self, p: [f64; 2]) -> [f64; 2] {
        if !self.geographic {
            return p;
        }
        [
            EARTH_RADIUS_M * (p[0] - self.lon0).to_radians() * self.cos_lat0,
            EARTH_RADIUS_M * (p[1] - self.lat0).to_radians(),
        ]
    }

    pub fn inverse(&self, p: [f64; 2]) -> [f64; 2] {
        if !self.geographic {
            return p;
        }
        [
            self.lon0 + (p[0] / (EARTH_RADIUS_M * self.cos_lat0)).to_degrees(),
            self.lat0 + (p[1] / EARTH_RADIUS_M).to_degrees(),
        ]
    }

    /// Ground distance between two source coordinates, metres.
    pub fn distance(&self, a: [f64; 2], b: [f64; 2]) -> f64 {
        if !self.geographic {
            return (a[0] - b[0]).hypot(a[1] - b[1]);
        }
        let (la, lb) = (a[1].to_radians(), b[1].to_radians());
        let dlat = lb - la;
        let dlon = (b[0] - a[0]).to_radians();
        let h = (dlat / 2.0).sin().powi(2) + la.cos() * lb.cos() * (dlon / 2.0).sin().powi(2);
        2.0 * EARTH_RADIUS_M * h.sqrt().asin()
    }
}

pub fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Closest point on segment `ab` to `p`, as (parameter t ∈ [0,1], distance).
pub fn project_on_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> (f64, f64) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    };
    let q = [a[0] + t * dx, a[1] + t * dy];
    (t, dist(p, q))
}

/// Uniform grid of item ids by cell, for radius and nearest queries.
pub struct Grid {
    cell: f64,
    cells: HashMap<(i64, i64), Vec<usize>>,
}

impl Grid {
    pub fn new(cell: f64) -> Self {
        Self {
            cell: cell.max(1e-6),
            cells: HashMap::new(),
        }
    }

    fn key(&self, p: [f64; 2]) -> (i64, i64) {
        (
            (p[0] / self.cell).floor() as i64,
            (p[1] / self.cell).floor() as i64,
        )
    }

    pub fn insert(&mut self, p: [f64; 2], id: usize) {
        let key = self.key(p);
        self.cells.entry(key).or_default().push(id);
    }

    /// Insert an item covering a bounding box (segments).
    pub fn insert_box(&mut self, lo: [f64; 2], hi: [f64; 2], id: usize) {
        let (a, b) = (self.key(lo), self.key(hi));
        for x in a.0..=b.0 {
            for y in a.1..=b.1 {
                self.cells.entry((x, y)).or_default().push(id);
            }
        }
    }

    /// Ids in cells overlapping the square of half-side `r` around `p`.
    pub fn near(&self, p: [f64; 2], r: f64) -> impl Iterator<Item = usize> + '_ {
        let (a, b) = (
            self.key([p[0] - r, p[1] - r]),
            self.key([p[0] + r, p[1] + r]),
        );
        (a.0..=b.0)
            .flat_map(move |x| (a.1..=b.1).map(move |y| (x, y)))
            .filter_map(|k| self.cells.get(&k))
            .flatten()
            .copied()
    }

    /// The `k` nearest of `points` to `p` (excluding `skip`), searching rings
    /// outward until `k` are found or `max_r` is passed.
    pub fn k_nearest(
        &self,
        points: &[[f64; 2]],
        p: [f64; 2],
        k: usize,
        skip: usize,
        max_r: f64,
    ) -> Vec<(usize, f64)> {
        let mut r = self.cell;
        loop {
            let mut found: Vec<(usize, f64)> = self
                .near(p, r)
                .filter(|&i| i != skip)
                .map(|i| (i, dist(p, points[i])))
                .filter(|&(_, d)| d <= r)
                .collect();
            if found.len() >= k || r >= max_r {
                found.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
                found.dedup_by_key(|f| f.0);
                found.truncate(k);
                return found;
            }
            r *= 2.0;
        }
    }
}
