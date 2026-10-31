//! Water demand standards: what one point on the map draws per day.
//!
//! Rwanda's planning standard is MININFRA's *Manual for Rural Water Supply
//! Project Cycle Management*, Table 1-3 "Standard of Daily Supply Volume"
//! (from the GTZ *Study Manual for Gravity-Fed Systems*, 1996). A point's
//! `water_kind` picks the row; the property named by the row gives the count
//! (people, beds, students, head of cattle...). Average daily demand is then
//! multiplied by the peak factor of [`crate::LayoutOptions`]: the manual sets
//! no peak factor, so that one is a stated engineering choice.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct DemandStandard {
    pub kind: &'static str,
    /// Litres per day per unit (or per site when `per` is empty).
    pub litres_per_day: f64,
    /// Property holding the unit count; empty for a fixed per-site demand.
    pub per: &'static str,
}

const fn row(kind: &'static str, litres_per_day: f64, per: &'static str) -> DemandStandard {
    DemandStandard {
        kind,
        litres_per_day,
        per,
    }
}

/// MININFRA Table 1-3 rows, keyed by `water_kind`.
pub const RWANDA_STANDARD: [DemandStandard; 23] = [
    row("standpost", 20.0, "population"),
    row("kiosk", 20.0, "population"),
    row("household", 20.0, "population"),
    row("urban_household", 80.0, "population"),
    row("cattle", 40.0, "head"),
    row("goats", 5.0, "head"),
    row("hospital", 150.0, "beds"),
    row("health_centre", 8_000.0, ""),
    row("primary_school", 5.0, "students"),
    row("high_school", 15.0, "students"),
    row("boarding_school", 40.0, "students"),
    row("youth_centre", 50.0, "population"),
    row("orphanage", 30.0, "population"),
    row("district_office", 500.0, ""),
    row("sector_office", 100.0, ""),
    row("court", 200.0, ""),
    row("parish", 80.0, "population"),
    row("prison", 70.0, "population"),
    row("military_camp", 100.0, "population"),
    row("trading_centre", 120.0, ""),
    row("market", 1_000.0, ""),
    row("slaughter_house", 350.0, "head"),
    row("tea_factory", 5_000.0, "tonnes"),
];

pub fn standard(kind: &str) -> Option<&'static DemandStandard> {
    RWANDA_STANDARD.iter().find(|s| s.kind == kind)
}

/// Average daily demand in litres for a point, or `None` when its kind has
/// no standard row.
pub fn daily_litres(kind: &str, count: impl Fn(&str) -> Option<f64>) -> Option<f64> {
    let s = standard(kind)?;
    if s.per.is_empty() {
        Some(s.litres_per_day)
    } else {
        Some(s.litres_per_day * count(s.per).unwrap_or(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_follow_the_national_table() {
        assert_eq!(daily_litres("standpost", |_| Some(250.0)), Some(5_000.0));
        assert_eq!(
            daily_litres("hospital", |p| (p == "beds").then_some(40.0)),
            Some(6_000.0)
        );
        assert_eq!(daily_litres("market", |_| None), Some(1_000.0));
        assert_eq!(daily_litres("spaceport", |_| Some(1.0)), None);
    }
}
