use super::*;

fn ids(entries: &Value) -> Vec<&str> {
    entries
        .as_array()
        .expect("entries")
        .iter()
        .map(|entry| entry["page"].as_str().expect("a page command"))
        .collect()
}

#[test]
fn the_commands_are_cores_water_door_commands_without_the_doors_own_fields() {
    let manifest = manifest();
    assert_eq!(manifest["namespace"], NAMESPACE);
    let commands = manifest["commands"].as_array().unwrap();
    let words: Vec<String> = commands
        .iter()
        .map(|command| {
            command["words"]
                .as_array()
                .unwrap()
                .iter()
                .map(|word| word.as_str().unwrap())
                .collect::<Vec<_>>()
                .join(".")
        })
        .collect();
    // `water.models` is the Kernel's own read by the model kind.
    assert_eq!(words, ["water.run", "water.save"]);
    for command in commands {
        for member in command["members"].as_array().unwrap() {
            let name = member["name"].as_str().unwrap();
            assert!(
                !matches!(name, "project" | "lane" | "output" | "confirm" | "yes"),
                "{name} is the door's or Core's own"
            );
        }
    }
    assert_eq!(
        commands[1]["effect"], "project_write",
        "a save asks first; the Kernel publishes"
    );
    for command in commands {
        for member in command["members"].as_array().unwrap() {
            if matches!(member["type"].as_str(), Some("model" | "model_name")) {
                assert_eq!(
                    member["kind"], MODEL_KIND,
                    "a model member names the declared kind"
                );
            }
        }
    }
}

/// The Kernel's Water arm at ds-cli e09cbcea2, group by group.
#[test]
fn the_area_is_the_kernels_water_arm_moved_as_it_is() {
    let ribbon = ribbon();
    let area = &ribbon["area"];
    assert_eq!(
        (
            area["id"].as_str(),
            area["kind"].as_str(),
            area["frame"].as_str()
        ),
        (Some("water"), Some("water"), Some("water_network"))
    );
    assert!(ribbon.get("anchor").is_none(), "an area, not a tab");
    assert_eq!(ids(&area["lead"]), ["water_open"]);
    let groups = ribbon["groups"].as_array().unwrap();
    let group_ids: Vec<&str> = groups
        .iter()
        .map(|group| group["id"].as_str().unwrap())
        .collect();
    assert_eq!(group_ids, ["settings", "engineer", "output"]);
    assert_eq!(
        ids(&groups[0]["entries"]),
        [
            "water_library",
            "water_layout",
            "water_import",
            "water_refresh"
        ]
    );
    assert_eq!(
        ids(&groups[1]["entries"]),
        [
            "water_process",
            "water_autoprocess",
            "water_design",
            "water_simulate"
        ]
    );
    assert_eq!(ids(&groups[2]["entries"]), ["water_draw", "water_export"]);
}

#[test]
fn every_word_fits_the_ribbon_in_both_languages() {
    let ribbon = ribbon();
    let mut labels = vec![&ribbon["label"]];
    for group in ribbon["groups"].as_array().unwrap() {
        labels.push(&group["label"]);
        labels.extend(
            group["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| &e["label"]),
        );
    }
    labels.extend(
        ribbon["area"]["lead"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| &e["label"]),
    );
    for label in labels {
        for language in ["en", "fr"] {
            let text = label[language].as_str().unwrap();
            assert!(
                !text.trim().is_empty() && text.chars().count() <= 40,
                "{text}"
            );
        }
    }
}

#[test]
fn the_model_kind_is_waters_own() {
    let manifest = manifest();
    let models = manifest["models"].as_array().unwrap();
    assert_eq!(models.len(), 1);
    let record = models[0].as_object().unwrap();
    assert_eq!(record.len(), 10);
    assert_eq!(record["id_prefix"], format!("{NAMESPACE}_"));
    assert!(
        record["source_prefix"]
            .as_str()
            .unwrap()
            .starts_with(&format!("{NAMESPACE}-"))
    );
    for member in ["kind", "input_invalid", "model_invalid", "source_changed"] {
        assert!(
            record[member]
                .as_str()
                .unwrap()
                .starts_with(&format!("{NAMESPACE}_")),
            "{member}"
        );
    }
}

#[test]
fn the_plug_claims_no_standing_area_while_the_kernels_water_arm_serves() {
    // web/plug.json (product scope §8) names no standing tab yet: until the
    // Kernel's built-in Water area is deleted (W2-switch), a placed area would
    // supersede it greyed wherever the lane's Water unit has no door. The
    // W2-switch adds `"tab"`: this unit's own ribbon, word for word.
    let plug: Value = serde_json::from_str(include_str!("../../../web/plug.json")).unwrap();
    assert!(plug.get("tab").is_none());
    assert!(ribbon().is_object());
    assert_eq!(plug["namespace"], NAMESPACE);
    assert_eq!(plug["component"], NAMESPACE);
}
