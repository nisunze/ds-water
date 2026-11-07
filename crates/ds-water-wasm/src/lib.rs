//! The Water WASM unit: the browser's door to the same command surface the
//! CLI and Core use. The page never sequences engine steps; it hands the
//! Kernel's request to [`water_invoke`] and returns the answer.

use wasm_bindgen::prelude::wasm_bindgen;

pub mod plugin;

/// The `ds water …` commands this unit brings to the page's command door
/// (`ds.plugin-commands/v1`); the page registers them with the Kernel.
#[wasm_bindgen(js_name = pluginManifest)]
pub fn plugin_manifest() -> String {
    plugin::manifest().to_string()
}

/// Water Network, this unit's own Design area (`ds.plugin-ribbon/v1`), a
/// registration separate from its commands.
#[wasm_bindgen(js_name = pluginRibbon)]
pub fn plugin_ribbon() -> String {
    plugin::ribbon().to_string()
}

/// Run one Water operation (`identity`, `validate`, `simulate`, `layout`,
/// `design`, `profile`, `draw`, `scheme`) on a JSON request.
#[wasm_bindgen]
pub fn water_invoke(operation: &str, request_json: &str) -> Result<String, String> {
    let request: serde_json::Value = if request_json.trim().is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(request_json).map_err(|e| format!("water_request_invalid: {e}"))?
    };
    answered(operation, request)
}

/// The page's unit door calls one argument: `{operation, request}`, framed
/// by the Kernel (`water_web::frame`).
#[wasm_bindgen]
pub fn water_invoke_json(frame_json: &str) -> Result<String, String> {
    let frame: serde_json::Value =
        serde_json::from_str(frame_json).map_err(|e| format!("water_request_invalid: {e}"))?;
    let operation = frame["operation"]
        .as_str()
        .ok_or("water_request_invalid: the frame names no operation")?;
    answered(operation, frame["request"].clone())
}

/// The release a stamped build names (`ds-water@<version>+<source>`), which a
/// saved network records; a development build names none.
fn release() -> Option<String> {
    let source = env!("DS_WATER_WASM_SOURCE_SHA");
    (source.len() == 40 && source.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| format!("ds-water@{}+{source}", env!("CARGO_PKG_VERSION")))
}

/// One operation's answer as text; `identity` also names the release.
fn answered(operation: &str, request: serde_json::Value) -> Result<String, String> {
    let mut answer = ds_water_ops::invoke(operation, request).map_err(|e| e.to_string())?;
    if let ("identity", Some(release), Some(fields)) =
        (operation, release(), answer.as_object_mut())
    {
        fields.insert("release".into(), release.into());
    }
    serde_json::to_string(&answer).map_err(|e| e.to_string())
}

/// The unit's identity: what the Kernel admits it by — the engine, its
/// operations and the producer receipt (source and dependency closure) the
/// release stamped into it.
#[wasm_bindgen]
pub fn water_engine_identity() -> String {
    let engine =
        ds_water_ops::invoke("identity", serde_json::Value::Null).expect("identity never fails");
    serde_json::json!({
        "schema": "ds.water-engine/v1",
        "protocolMajor": 1,
        "environment": "wasm",
        "coreVersion": env!("CARGO_PKG_VERSION"),
        "sourceSha": env!("DS_WATER_WASM_SOURCE_SHA"),
        "dependencyClosureSha256": env!("DS_WATER_WASM_DEPENDENCY_CLOSURE_SHA256"),
        "capabilities": engine["operations"],
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_door_passes_requests_and_refusals_through() {
        let identity: serde_json::Value = serde_json::from_str(&water_engine_identity()).unwrap();
        assert_eq!(identity["schema"], "ds.water-engine/v1");
        let answer: serde_json::Value =
            serde_json::from_str(&water_invoke("identity", "").unwrap()).unwrap();
        assert_eq!(answer["engine"], "ds-water");
        match identity["sourceSha"].as_str().unwrap() {
            "development" => assert!(
                answer.get("release").is_none(),
                "a development build names no release"
            ),
            source => assert_eq!(
                answer["release"],
                format!("ds-water@{}+{source}", env!("CARGO_PKG_VERSION"))
            ),
        }
        assert!(
            identity["capabilities"]
                .as_array()
                .unwrap()
                .iter()
                .any(|op| op == "scheme")
        );
        assert!(
            water_invoke("teleport", "{}")
                .unwrap_err()
                .contains("unknown water operation")
        );
        assert!(
            water_invoke("simulate", "{not json")
                .unwrap_err()
                .starts_with("water_request_invalid")
        );
    }
}
