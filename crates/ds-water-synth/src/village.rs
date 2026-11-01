//! A hypothetical village scheme as map features: a spring on high ground, a
//! main road running downhill with a few branch roads, households along the
//! roads and standposts on the main road. Coordinates are WGS84 lon/lat
//! around the requested point; the same point and seed always give the same
//! village, so a test network can be made again exactly.

use serde_json::{Value, json};

use crate::{Rng, terrain};

/// What village to make.
#[derive(Clone, Debug)]
pub struct Village {
    /// Where the spring is, `[lon, lat]`.
    pub at: [f64; 2],
    pub seed: u64,
    /// Households to place along the roads.
    pub households: usize,
}

const METRES_PER_DEGREE_LAT: f64 = 110_540.0;
const METRES_PER_DEGREE_LON_AT_EQUATOR: f64 = 111_320.0;
const MAIN_ROAD_STEPS: usize = 24;
const MAIN_ROAD_STEP_M: f64 = 110.0;
const STANDPOST_EVERY: usize = 6;

impl Village {
    /// Local metres (east, north of the spring) to `[lon, lat]`.
    fn lonlat(&self, x: f64, y: f64) -> [f64; 2] {
        let [lon, lat] = self.at;
        let east = METRES_PER_DEGREE_LON_AT_EQUATOR * lat.to_radians().cos();
        [lon + x / east, lat + y / METRES_PER_DEGREE_LAT]
    }

    /// The village as a GeoJSON FeatureCollection with `water_kind`s.
    pub fn features(&self) -> Value {
        let mut rng = Rng::new(self.seed);
        // The terrain is sampled at an offset the seed picks, so villages differ.
        let (ox, oy) = (rng.range(0.0, 80_000.0), rng.range(0.0, 80_000.0));
        let ground = |x: f64, y: f64| terrain(x + ox, y + oy);
        let mut features = Vec::new();

        // The main road leaves the spring and winds downhill (southwards).
        let mut road = vec![(0.0_f64, 0.0_f64)];
        let mut heading = rng.range(-0.6, 0.6) - std::f64::consts::FRAC_PI_2;
        for _ in 0..MAIN_ROAD_STEPS {
            heading += rng.range(-0.35, 0.35);
            let (x, y) = *road.last().expect("the road starts at the spring");
            road.push((
                x + MAIN_ROAD_STEP_M * heading.cos(),
                y + MAIN_ROAD_STEP_M * heading.sin(),
            ));
        }
        let mut roads = vec![road.clone()];
        let branches = 2 + (rng.next() % 3) as usize;
        for _ in 0..branches {
            let from = 4 + (rng.next() as usize % (MAIN_ROAD_STEPS - 6));
            let side = if rng.unit() < 0.5 { 1.0 } else { -1.0 };
            let mut angle = heading_at(&road, from) + side * rng.range(1.0, 1.9);
            let mut branch = vec![road[from]];
            for _ in 0..(4 + rng.next() % 6) {
                angle += rng.range(-0.3, 0.3);
                let (x, y) = *branch.last().expect("a branch starts on the road");
                branch.push((x + 100.0 * angle.cos(), y + 100.0 * angle.sin()));
            }
            roads.push(branch);
        }

        let mut highest = f64::MIN;
        for (index, line) in roads.iter().enumerate() {
            let coordinates: Vec<[f64; 2]> = line.iter().map(|&(x, y)| self.lonlat(x, y)).collect();
            features.push(json!({
                "type": "Feature",
                "properties": {"water_kind": "road", "id": format!("R{index}")},
                "geometry": {"type": "LineString", "coordinates": coordinates},
            }));
            for &(x, y) in line {
                highest = highest.max(ground(x, y));
            }
        }

        for household in 0..self.households {
            let line = &roads[rng.next() as usize % roads.len()];
            let at = 1 + rng.next() as usize % (line.len() - 1);
            let ((x0, y0), (x1, y1)) = (line[at - 1], line[at]);
            let t = rng.unit();
            let (dx, dy) = (x1 - x0, y1 - y0);
            let length = (dx * dx + dy * dy).sqrt().max(1.0);
            let offset = rng.range(15.0, 60.0) * if rng.unit() < 0.5 { 1.0 } else { -1.0 };
            let (x, y) = (
                x0 + t * dx - offset * dy / length,
                y0 + t * dy + offset * dx / length,
            );
            let elevation = ground(x, y);
            highest = highest.max(elevation);
            features.push(json!({
                "type": "Feature",
                "properties": {
                    "water_kind": "household",
                    "id": format!("H{household}"),
                    "population": 4 + rng.next() % 5,
                    "elevation_m": round(elevation),
                },
                "geometry": {"type": "Point", "coordinates": self.lonlat(x, y)},
            }));
        }

        for (index, &(x, y)) in road.iter().enumerate().skip(STANDPOST_EVERY) {
            if index % STANDPOST_EVERY == 0 {
                features.push(json!({
                    "type": "Feature",
                    "properties": {
                        "water_kind": "standpost",
                        "id": format!("S{}", index / STANDPOST_EVERY),
                        "elevation_m": round(ground(x, y)),
                    },
                    "geometry": {"type": "Point", "coordinates": self.lonlat(x, y)},
                }));
            }
        }

        // The spring sits above everything it serves.
        features.push(json!({
            "type": "Feature",
            "properties": {"water_kind": "spring", "id": "Spring", "head_m": round(highest + 45.0)},
            "geometry": {"type": "Point", "coordinates": self.at},
        }));
        json!({"type": "FeatureCollection", "features": features})
    }
}

fn heading_at(line: &[(f64, f64)], at: usize) -> f64 {
    let ((x0, y0), (x1, y1)) = (line[at - 1], line[at]);
    (y1 - y0).atan2(x1 - x0)
}

fn round(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
