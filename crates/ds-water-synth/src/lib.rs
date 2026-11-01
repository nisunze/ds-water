//! Deterministic synthetic networks shaped like a national rural/urban
//! supply: hill-top sources, transmission mains, looped town grids, branched
//! village schemes and a few interconnections between neighbouring towns.
//!
//! Terrain is a smooth sum of waves between roughly 1,000 and 2,400 m, like
//! Rwanda's. Coordinates are local metres. The same seed always gives the
//! same network, so a benchmark can be repeated exactly.

pub mod village;

use ds_water_model::{Junction, Pipe, Reservoir, WaterNetwork};

#[derive(Clone, Debug)]
pub struct Spec {
    /// Approximate number of junctions to generate.
    pub junctions: usize,
    pub seed: u64,
    /// Side of the square country, km.
    pub extent_km: f64,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            junctions: 10_000,
            seed: 1,
            extent_km: 160.0,
        }
    }
}

/// xorshift64*: tiny, deterministic, and enough for layout noise.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
}

pub fn terrain(x: f64, y: f64) -> f64 {
    let (x, y) = (x / 1000.0, y / 1000.0);
    1700.0
        + 450.0 * (x / 23.0).sin() * (y / 31.0).cos()
        + 180.0 * (x / 7.3 + 1.1).sin() * (y / 5.9).sin()
        + 60.0 * ((x + y) / 1.7).sin()
}

const TOWN_SIDE: usize = 24;
const VILLAGE_NODES: usize = 1_200;
const SPACING_M: f64 = 90.0;

pub fn generate(spec: &Spec) -> WaterNetwork {
    let mut rng = Rng::new(spec.seed);
    let per_town = TOWN_SIDE * TOWN_SIDE + VILLAGE_NODES;
    let towns = spec.junctions.div_ceil(per_town).max(1);
    let extent = spec.extent_km * 1000.0;
    let mut net = WaterNetwork::new(format!(
        "synthetic country, {towns} towns, seed {}",
        spec.seed
    ));
    let mut centres: Vec<(f64, f64, usize)> = Vec::with_capacity(towns);
    let mut pipe_id = 0usize;
    let mut add_pipe = |net: &mut WaterNetwork, a: &str, b: &str, length: f64, d: f64| {
        pipe_id += 1;
        net.pipes.push(Pipe {
            id: format!("P{pipe_id}"),
            from: a.to_owned(),
            to: b.to_owned(),
            length_m: length.max(1.0),
            diameter_mm: d,
            roughness: 140.0,
            ..Pipe::default()
        });
    };
    let junction = |net: &mut WaterNetwork, id: String, x: f64, y: f64, demand: f64| {
        net.junctions.push(Junction {
            id,
            elevation_m: terrain(x, y),
            demand_lps: demand,
            x: Some(x),
            y: Some(y),
            ..Junction::default()
        });
    };

    for t in 0..towns {
        let (cx, cy) = (
            rng.range(0.05, 0.95) * extent,
            rng.range(0.05, 0.95) * extent,
        );
        let first = net.junctions.len();
        centres.push((cx, cy, first));
        // Looped town grid.
        let mut town_max: f64 = f64::MIN;
        for r in 0..TOWN_SIDE {
            for c in 0..TOWN_SIDE {
                let x = cx + (c as f64 - TOWN_SIDE as f64 / 2.0) * SPACING_M;
                let y = cy + (r as f64 - TOWN_SIDE as f64 / 2.0) * SPACING_M;
                junction(
                    &mut net,
                    format!("T{t}_{r}_{c}"),
                    x,
                    y,
                    rng.range(0.05, 0.25),
                );
                town_max = town_max.max(terrain(x, y));
            }
        }
        for r in 0..TOWN_SIDE {
            for c in 0..TOWN_SIDE {
                let trunk = r == TOWN_SIDE / 2 || c == TOWN_SIDE / 2;
                if c + 1 < TOWN_SIDE {
                    let d = if r == TOWN_SIDE / 2 { 250.0 } else { 90.0 };
                    add_pipe(
                        &mut net,
                        &format!("T{t}_{r}_{c}"),
                        &format!("T{t}_{r}_{}", c + 1),
                        SPACING_M,
                        d,
                    );
                }
                if r + 1 < TOWN_SIDE {
                    let d = if trunk && c == TOWN_SIDE / 2 {
                        250.0
                    } else {
                        90.0
                    };
                    add_pipe(
                        &mut net,
                        &format!("T{t}_{r}_{c}"),
                        &format!("T{t}_{}_{c}", r + 1),
                        SPACING_M,
                        d,
                    );
                }
            }
        }
        // Hill-top source a few km out, feeding the town centre by gravity.
        let angle = rng.range(0.0, std::f64::consts::TAU);
        let distance = rng.range(2_000.0, 5_000.0);
        let (sx, sy) = (cx + distance * angle.cos(), cy + distance * angle.sin());
        let source = format!("S{t}");
        net.reservoirs.push(Reservoir {
            id: source.clone(),
            head_m: town_max.max(terrain(sx, sy)) + rng.range(45.0, 80.0),
            x: Some(sx),
            y: Some(sy),
            ..Reservoir::default()
        });
        let centre = format!("T{t}_{}_{}", TOWN_SIDE / 2, TOWN_SIDE / 2);
        add_pipe(&mut net, &source, &centre, distance * 1.15, 350.0);

        // Branched village scheme grown from the town edge.
        let mut frontier: Vec<(String, f64, f64, f64)> = vec![(
            format!("T{t}_0_0"),
            cx - TOWN_SIDE as f64 / 2.0 * SPACING_M,
            cy - TOWN_SIDE as f64 / 2.0 * SPACING_M,
            160.0,
        )];
        for v in 0..VILLAGE_NODES {
            let pick = (rng.next() as usize) % frontier.len();
            let (parent, px, py, pd) = frontier[pick].clone();
            let heading = rng.range(0.0, std::f64::consts::TAU);
            let step = rng.range(80.0, 250.0);
            let (x, y) = (px + step * heading.cos(), py + step * heading.sin());
            let id = format!("V{t}_{v}");
            junction(&mut net, id.clone(), x, y, rng.range(0.01, 0.06));
            let d = (pd * 0.97).max(50.0);
            add_pipe(&mut net, &parent, &id, step * 1.05, d);
            frontier.push((id, x, y, d));
            if frontier.len() > 40 {
                frontier.swap_remove(0);
            }
        }
    }
    // Interconnect each town with its nearest neighbour: the national grid's
    // loops between schemes.
    for t in 0..towns {
        let (x, y, first) = centres[t];
        let nearest = (0..towns).filter(|&u| u != t).min_by(|&a, &b| {
            let da = (centres[a].0 - x).hypot(centres[a].1 - y);
            let db = (centres[b].0 - x).hypot(centres[b].1 - y);
            da.total_cmp(&db)
        });
        if let Some(u) = nearest
            && u > t
        {
            let length = (centres[u].0 - x).hypot(centres[u].1 - y) * 1.2;
            let a = &net.junctions[first + TOWN_SIDE - 1].id.clone();
            let b = &net.junctions[centres[u].2].id.clone();
            add_pipe(&mut net, a, b, length, 200.0);
        }
    }
    net
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_deterministic_and_valid() {
        let spec = Spec {
            junctions: 3_000,
            seed: 7,
            extent_km: 40.0,
        };
        let a = generate(&spec);
        let b = generate(&spec);
        assert_eq!(a, b);
        assert!(a.junctions.len() >= 3_000);
        assert!(ds_water_model::validate(&a).is_empty());
    }
}
