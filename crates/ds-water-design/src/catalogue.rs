//! Pipe catalogue: what can be bought, its bore, its pressure rating and an
//! indicative installed cost.
//!
//! The built-in catalogue follows ISO 4427 (PE100 polyethylene by SDR),
//! ISO 1452 (PVC-U) and ISO 2531 (ductile iron, class K9 here). Costs are
//! indicative, USD per metre installed (pipe by mass plus trenching), only
//! there to rank alternatives; a project replaces them with its own rates
//! through a catalogue JSON.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CatalogueItem {
    pub material: String,
    /// Nominal outside diameter (plastics) or DN (iron), mm.
    pub nominal_mm: f64,
    pub internal_mm: f64,
    /// Pressure rating, bar.
    pub pn_bar: f64,
    /// Hazen-Williams C for new pipe.
    pub hazen_c: f64,
    pub cost_per_m: f64,
    /// Standard dimension ratio, for plastics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdr: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Catalogue {
    pub name: String,
    pub items: Vec<CatalogueItem>,
}

const PE_OD: [f64; 22] = [
    20.0, 25.0, 32.0, 40.0, 50.0, 63.0, 75.0, 90.0, 110.0, 125.0, 140.0, 160.0, 180.0, 200.0,
    225.0, 250.0, 280.0, 315.0, 355.0, 400.0, 450.0, 500.0,
];
const PVC_OD: [f64; 12] = [
    63.0, 75.0, 90.0, 110.0, 125.0, 140.0, 160.0, 200.0, 250.0, 315.0, 400.0, 500.0,
];
/// DN and ISO 2531 K9 bore (mm) for ductile iron with cement lining.
const DI_DN: [(f64, f64); 9] = [
    (100.0, 104.0),
    (150.0, 155.0),
    (200.0, 205.0),
    (250.0, 256.0),
    (300.0, 306.0),
    (400.0, 406.0),
    (500.0, 506.0),
    (600.0, 606.0),
    (800.0, 806.0),
];

fn trench(nominal_mm: f64) -> f64 {
    9.0 + 0.04 * nominal_mm
}

impl Catalogue {
    /// The built-in catalogue (indicative costs).
    pub fn standard() -> Self {
        let mut items = Vec::new();
        // PE100: SDR 17 = PN10, SDR 11 = PN16, SDR 9 = PN20, SDR 7.4 = PN25.
        for (sdr, pn) in [(17.0, 10.0), (11.0, 16.0), (9.0, 20.0), (7.4, 25.0)] {
            for od in PE_OD {
                let wall = (od / sdr).max(2.0);
                if od < 32.0 && sdr > 11.0 {
                    continue; // small PE is made from SDR 11 up
                }
                let mass_kg_m = std::f64::consts::PI * (od - wall) * wall * 1e-6 * 960.0;
                items.push(CatalogueItem {
                    material: "HDPE PE100".into(),
                    nominal_mm: od,
                    internal_mm: od - 2.0 * wall,
                    pn_bar: pn,
                    hazen_c: 150.0,
                    cost_per_m: round2(mass_kg_m * 2.6 + trench(od)),
                    sdr: Some(sdr),
                });
            }
        }
        // PVC-U: SDR 26 = PN10, SDR 17 = PN16.
        for (sdr, pn) in [(26.0, 10.0), (17.0, 16.0)] {
            for od in PVC_OD {
                let wall = od / sdr;
                let mass_kg_m = std::f64::consts::PI * (od - wall) * wall * 1e-6 * 1400.0;
                items.push(CatalogueItem {
                    material: "PVC-U".into(),
                    nominal_mm: od,
                    internal_mm: od - 2.0 * wall,
                    pn_bar: pn,
                    hazen_c: 145.0,
                    cost_per_m: round2(mass_kg_m * 1.8 + trench(od)),
                    sdr: Some(sdr),
                });
            }
        }
        for (dn, bore) in DI_DN {
            items.push(CatalogueItem {
                material: "Ductile iron K9".into(),
                nominal_mm: dn,
                internal_mm: bore,
                pn_bar: if dn <= 300.0 { 40.0 } else { 30.0 },
                hazen_c: 140.0,
                cost_per_m: round2(0.9 * dn + trench(dn) + 20.0),
                sdr: None,
            });
        }
        Self {
            name: "DS standard (ISO 4427 / 1452 / 2531, indicative USD)".into(),
            items,
        }
    }

    /// Items of the allowed materials rated for at least `head_m` of water,
    /// cheapest first per bore.
    pub fn rated(&self, materials: &[String], head_m: f64) -> Vec<&CatalogueItem> {
        let need_bar = head_m / 10.197;
        let mut out: Vec<&CatalogueItem> = self
            .items
            .iter()
            .filter(|i| materials.is_empty() || materials.iter().any(|m| m == &i.material))
            .filter(|i| i.pn_bar + 1e-9 >= need_bar)
            .collect();
        out.sort_by(|a, b| {
            a.internal_mm
                .total_cmp(&b.internal_mm)
                .then(a.cost_per_m.total_cmp(&b.cost_per_m))
        });
        // Keep only bores where nothing cheaper has an equal or larger bore:
        // the efficient frontier of bore against cost.
        let mut frontier: Vec<&CatalogueItem> = Vec::new();
        for item in out.into_iter().rev() {
            if frontier
                .last()
                .is_none_or(|f| item.cost_per_m < f.cost_per_m)
            {
                frontier.push(item);
            }
        }
        frontier.reverse();
        frontier
    }
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}
