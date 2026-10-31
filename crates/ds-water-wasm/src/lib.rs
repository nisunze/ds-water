//! The Water WASM unit: the browser's door to the same command surface the
//! CLI and Core use. The page never sequences engine steps; it hands the
//! Kernel's request to [`water_invoke`] and returns the answer.

use wasm_bindgen::prelude::wasm_bindgen;

/// Run one Water operation (`identity`, `validate`, `simulate`, `layout`,
/// `design`, `profile`, `draw`, `scheme`) on a JSON request.
#[wasm_bindgen]
pub fn water_invoke(operation: &str, request_json: &str) -> Result<String, String> {
    let request: serde_json::Value = if request_json.trim().is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(request_json).map_err(|e| format!("water_request_invalid: {e}"))?
    };
    let answer = ds_water_ops::invoke(operation, request).map_err(|e| e.to_string())?;
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
        assert_eq!(identity["sourceSha"], "development");
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
