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

/// Engine name, version and operations, for the unit's install record.
#[wasm_bindgen]
pub fn water_engine_identity() -> String {
    water_invoke("identity", "").expect("identity never fails")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_door_passes_requests_and_refusals_through() {
        assert!(water_engine_identity().contains("\"engine\":\"ds-water\""));
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
