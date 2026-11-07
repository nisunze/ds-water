use super::*;

const RELEASE: &str = "ds-water@0.1.0+0123456789abcdef0123456789abcdef01234567";

fn request(value: Value) -> Map<String, Value> {
    value.as_object().expect("a request object").clone()
}

fn example() -> Value {
    json!({"at": [29.70, -2.75], "seed": 1})
}

/// A saved network as the Kernel opens it for a `model` member.
fn saved(model_id: &str) -> Value {
    let network = invoke("example", example()).unwrap()["network"].clone();
    let packed = invoke(
        "pack",
        json!({"network": network, "model_id": model_id, "revision": 2, "project": "p1"}),
    )
    .unwrap();
    json!({"model_id": model_id, "display_name": "Village A", "version": 2, "revision": 3,
           "project": "p1", "dsgrid_base64": packed["dsgrid_base64"]})
}

#[test]
fn a_named_save_answers_its_data_and_the_kernels_publish_plan() {
    let answer = call(
        "water.save",
        request(json!({"operation": "example", "payload": example(),
            "name": {"name": "Village B", "model_id": "water_village_b", "revision": 1, "project": "p1"}})),
        Some(RELEASE.into()),
    );
    assert_eq!(answer["status"], "ok", "{answer}");
    let data = &answer["data"];
    assert_eq!(data["operation"], "example");
    assert!(data["network"]["junctions"].as_u64().unwrap() > 0);
    assert_eq!(data["map"]["type"], "FeatureCollection");
    let publish = &answer["publish"];
    assert_eq!(publish["target"], "name");
    assert_eq!(publish["source"], Value::Null);
    assert_eq!(publish["message"], "Saved from example");
    assert_eq!(publish["engine_release"], RELEASE);
    assert_eq!(publish["pack"]["model_kind"], "water_network");
    assert_eq!(publish["pack"]["packed"]["manifest_model_revision"], 1);
}

#[test]
fn a_save_on_a_saved_network_runs_on_it_and_plans_its_next_revision() {
    let answer = call(
        "water.save",
        request(
            json!({"operation": "validate", "model": saved("water_village_a"), "message": "checked"}),
        ),
        Some(RELEASE.into()),
    );
    // Validate answers no network, so there is nothing to save.
    assert_eq!(answer["refusal"]["code"], NETWORK_INVALID, "{answer}");
    let answer = call(
        "water.save",
        request(
            json!({"operation": "example", "payload": example(), "model": saved("water_village_a")}),
        ),
        Some(RELEASE.into()),
    );
    assert_eq!(answer["publish"]["target"], "model", "{answer}");
    assert_eq!(answer["publish"]["source"], "model");
    assert_eq!(
        answer["publish"]["pack"]["packed"]["manifest_model_revision"],
        3
    );
}

#[test]
fn a_run_on_a_saved_network_reads_as_the_kernels_arm_read_it() {
    let answer = call(
        "water.run",
        request(json!({"operation": "validate", "model": saved("water_village_a")})),
        None,
    );
    assert_eq!(answer["status"], "ok", "{answer}");
    let data = &answer["data"];
    assert_eq!(data["model"], "water_village_a");
    assert_eq!(data["version"], 2);
    assert_eq!(data["operation"], "validate");
    assert!(
        data["summary"][0]
            .as_str()
            .unwrap()
            .ends_with("validation issues")
    );
    assert!(answer.get("publish").is_none(), "a run plans no save");
    let plain = call("water.run", request(json!({"operation": "identity"})), None);
    assert_eq!(plain["data"]["engine"], "ds-water");
    // Through the door, identity names the release as the unit's own entry does.
    let named = call(
        "water.run",
        request(json!({"operation": "identity"})),
        Some(RELEASE.into()),
    );
    assert_eq!(named["data"]["release"], RELEASE);
    assert!(
        plain["data"].get("release").is_none(),
        "a development build names none"
    );
}

#[test]
fn every_refusal_is_a_declared_code_with_the_arms_words() {
    let refused = |command: &str, value: Value, release: Option<&str>| {
        let answer = call(command, request(value), release.map(str::to_owned));
        assert_eq!(answer["status"], "refused", "{answer}");
        (
            answer["refusal"]["code"].as_str().unwrap().to_owned(),
            answer["refusal"]["message"].as_str().unwrap().to_owned(),
        )
    };
    assert_eq!(
        refused("water.teleport", json!({}), None).0,
        REQUEST_INVALID
    );
    assert_eq!(
        refused("water.run", json!({"operation": "Scheme!"}), None).0,
        REQUEST_INVALID
    );
    assert_eq!(
        refused(
            "water.run",
            json!({"operation": "design", "payload": [1]}),
            None
        )
        .0,
        REQUEST_INVALID
    );
    assert_eq!(
        refused("water.run", json!({"operation": "teleport"}), None).0,
        ENGINE_REFUSED
    );
    assert_eq!(
        refused("water.save", json!({"operation": "example"}), Some(RELEASE)),
        (
            INPUT_INVALID.into(),
            "Name the network, or the saved network to run on.".into()
        )
    );
    let named = json!({"operation": "example", "payload": example(),
        "name": {"name": "B", "model_id": "water_b", "revision": 1, "project": "p1"}});
    assert_eq!(refused("water.save", named, None).0, UNIT_OUTDATED);
    let mut other = saved("water_village_a");
    other["model_id"] = json!("water_elsewhere");
    assert_eq!(
        refused(
            "water.run",
            json!({"operation": "validate", "model": other}),
            None
        ),
        (
            NETWORK_INVALID.into(),
            "The saved package holds another water network.".into()
        )
    );
    // Every code the door answers is one the manifest declares.
    let manifest = crate::plugin::manifest();
    for command in manifest["commands"].as_array().unwrap() {
        let declared: Vec<&str> = command["refusals"]
            .as_array()
            .unwrap()
            .iter()
            .map(|code| code.as_str().unwrap())
            .collect();
        assert!(declared.contains(&REQUEST_INVALID) && declared.contains(&ENGINE_REFUSED));
    }
}

#[test]
fn a_payload_read_from_a_file_is_its_json_text() {
    let text = example().to_string();
    let named = |payload: Value| {
        call(
            "water.save",
            request(json!({"operation": "example", "payload": payload,
                "name": {"name": "B", "model_id": "water_b", "revision": 1, "project": "p1"}})),
            Some(RELEASE.into()),
        )
    };
    let from_text = named(json!(text));
    assert_eq!(from_text["status"], "ok", "{from_text}");
    assert_eq!(
        from_text["data"],
        named(example())["data"],
        "text and object read alike"
    );
    assert_eq!(named(json!("{oops"))["refusal"]["code"], REQUEST_INVALID);
    assert_eq!(named(json!("[1]"))["refusal"]["code"], REQUEST_INVALID);
}
