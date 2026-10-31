use ds_water_design::{Catalogue, DesignCriteria, design};
use ds_water_hydraulics::Hydraulics;
use ds_water_model::*;
use ds_water_profile::{ProfileOptions, profile};

use super::*;

fn hill_scheme() -> WaterNetwork {
    let ground = |x: f64| {
        if x <= 1000.0 {
            1500.0 + 0.05 * x
        } else {
            1550.0 - 0.075 * (x - 1000.0)
        }
    };
    let mut n = WaterNetwork::new("hill scheme");
    n.crs = Some("EPSG:32735".into());
    n.reservoirs.push(Reservoir {
        id: "Spring".into(),
        head_m: 1600.0,
        x: Some(0.0),
        y: Some(0.0),
        ..Reservoir::default()
    });
    let mut prev = "Spring".to_owned();
    for i in 1..=6 {
        let x = i as f64 * 500.0;
        let id = format!("J{i}");
        n.junctions.push(Junction {
            id: id.clone(),
            elevation_m: ground(x),
            demand_lps: 0.4,
            x: Some(x),
            y: Some((x / 300.0).sin() * 80.0),
            ..Junction::default()
        });
        n.pipes.push(Pipe {
            id: format!("P{i}"),
            from: prev,
            to: id.clone(),
            length_m: 500.0,
            diameter_mm: 50.0,
            roughness: 150.0,
            ..Pipe::default()
        });
        prev = id;
    }
    n
}

fn well_formed(svg: &str) {
    roxmltree::Document::parse(svg).unwrap_or_else(|e| panic!("SVG is not well-formed: {e}"));
}

#[test]
fn a_designed_scheme_draws_a_full_set() {
    let net = hill_scheme();
    let designed = design(&net, &Catalogue::standard(), &DesignCriteria::default()).unwrap();
    let n = &designed.network;
    let peak = Hydraulics::new(n).unwrap().solve(0, &[]);
    let mut still_net = n.clone();
    still_net.options.demand_multiplier = 0.0;
    let still = Hydraulics::new(&still_net).unwrap().solve(0, &[]);
    let path: Vec<String> = (1..=6).map(|i| format!("P{i}")).collect();
    let surface = |p: [f64; 2]| {
        Some(if p[0] <= 1000.0 {
            1500.0 + 0.05 * p[0]
        } else {
            1550.0 - 0.075 * (p[0] - 1000.0)
        })
    };
    let prof = profile(
        n,
        &path,
        &peak,
        &still,
        Some(&surface),
        &ProfileOptions::default(),
    )
    .unwrap();
    let info = TitleInfo {
        project: "Hill scheme".into(),
        drawing_prefix: "HS".into(),
        revision: "A".into(),
        ..TitleInfo::default()
    };
    let opts = DrawingOptions::default();

    let sheets = plan_profile_sheets(n, &prof, "01", &info, &opts, 2);
    // About 3 km at 1:2500 on A3 is four windows of 855 m.
    assert_eq!(sheets.len(), 4);
    assert_eq!(sheets[0].number, "HS-002");
    for s in &sheets {
        well_formed(&s.svg);
        assert!(s.svg.contains("H 1:2500"));
        assert!(s.svg.contains("CHAINAGE"));
    }
    assert!(
        sheets.iter().any(|s| s.svg.contains(">AV<")),
        "the summit air valve is drawn"
    );
    if let Ok(dir) = std::env::var("DS_WATER_DRAWING_OUT") {
        for s in sheets
            .iter()
            .chain([&plan_sheet_for_export(n, &info, &opts)])
        {
            std::fs::write(format!("{dir}/{}.svg", s.number), &s.svg).unwrap();
        }
    }
    assert!(sheets.iter().any(|s| s.svg.contains(">BPT<")));

    let plan = network_plan_sheet(n, &info, &opts, "HS-001");
    well_formed(&plan.svg);
    assert!(plan.svg.contains("Spring (1600.0 m)"));

    let schedules = schedules_sheet(
        &designed.boq,
        &[("01".into(), &prof)],
        &DesignCriteria::default(),
        &info,
        &opts,
        "HS-006",
    );
    well_formed(&schedules.svg);
    assert!(schedules.svg.contains("HDPE PE100"));
    assert!(schedules.svg.contains("Air valve"));
}

#[test]
fn chainage_labels_are_kilometre_plus_metres() {
    assert_eq!(chainage_label(0.0), "0+000.0");
    assert_eq!(chainage_label(1234.56), "1+234.6");
}

fn plan_sheet_for_export(n: &WaterNetwork, info: &TitleInfo, opts: &DrawingOptions) -> Sheet {
    network_plan_sheet(n, info, opts, "HS-001")
}
