//! The unit's command door (W4, product scope §7): `water.run` and
//! `water.save` as the page's `plugin` door asks them, after the Kernel has
//! opened the saved networks the call names. The unit computes (unpack, run,
//! pack); a save answers the Kernel's `publish` plan, which the Kernel
//! fences, uploads and versions with the person's authority. Each step and
//! word is the Kernel's built-in Water arm's, moved as it is
//! (ds-command-kernel `water_web.rs`, `gateway_exchange/water_models.rs`).

use serde_json::{Map, Value, json};

pub const REQUEST_INVALID: &str = "water_request_invalid";
pub const ENGINE_REFUSED: &str = "water_engine_refused";
pub const UNIT_OUTDATED: &str = "water_unit_outdated";
pub const INPUT_INVALID: &str = "water_models_input_invalid";
pub const NETWORK_INVALID: &str = "water_network_invalid";
/// The unit's request bound: a country-scale network fits well inside it.
pub const MAX_REQUEST_BYTES: usize = 256 * 1024 * 1024;

type Refused = (&'static str, String);

/// One door call: the command, the Kernel-admitted request; the answer the
/// Kernel reads (`{status, data, publish?}` or `{status: refused, …}`).
pub fn call(command: &str, request: Map<String, Value>, release: Option<String>) -> Value {
    let answer = match command {
        "water.run" => run(request, release),
        "water.save" => save(request, release),
        _ => Err((
            REQUEST_INVALID,
            format!("`{command}` is not a command this unit answers."),
        )),
    };
    match answer {
        Ok(answer) => answer,
        Err((code, message)) => {
            json!({"status": "refused", "refusal": {"code": code, "message": message}})
        }
    }
}

fn invoke(operation: &str, request: Value) -> Result<Value, Refused> {
    if !operation_admitted(operation) {
        return Err((
            REQUEST_INVALID,
            format!("`{operation}` is not a water operation name."),
        ));
    }
    let frame = json!({ "operation": operation, "request": request }).to_string();
    if frame.len() > MAX_REQUEST_BYTES {
        return Err((
            REQUEST_INVALID,
            "The request exceeds the Water unit's bound.".into(),
        ));
    }
    ds_water_ops::invoke(operation, request).map_err(|error| (ENGINE_REFUSED, error.to_string()))
}

/// An operation name the unit may be asked for: a short lowercase word.
pub fn operation_admitted(operation: &str) -> bool {
    !operation.is_empty()
        && operation.len() <= 32
        && operation
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b == b'_')
}

fn text<'a>(request: &'a Map<String, Value>, member: &str) -> Option<&'a str> {
    request
        .get(member)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
}

/// The operation and its request object (`null` when there is none). The
/// payload is one JSON object, or its text as a page reads it from a file.
fn operation(request: &Map<String, Value>) -> Result<(String, Value), Refused> {
    let operation = text(request, "operation")
        .ok_or((REQUEST_INVALID, "Name the Water operation.".to_owned()))?
        .to_owned();
    let read = match request.get("payload") {
        Some(Value::String(text)) => Some(
            serde_json::from_str(text)
                .map_err(|e| (REQUEST_INVALID, format!("The request is not JSON: {e}")))?,
        ),
        other => other.cloned(),
    };
    let payload = match read {
        None | Some(Value::Null) => Value::Null,
        Some(payload @ Value::Object(_)) => payload,
        Some(_) => {
            return Err((
                REQUEST_INVALID,
                "The request must be one JSON object.".into(),
            ));
        }
    };
    let payload = operation_request(&operation, payload);
    Ok((operation, payload))
}

/// The network a saved model's package holds, opened by the Kernel as
/// `member` (`{model_id, version, dsgrid_base64, …}`).
fn opened(member: &Value) -> Result<Value, Refused> {
    let answer = invoke("unpack", json!({"dsgrid_base64": member["dsgrid_base64"]}))?;
    if answer["model_id"] != member["model_id"] {
        return Err((
            NETWORK_INVALID,
            "The saved package holds another water network.".into(),
        ));
    }
    answer
        .get("network")
        .filter(|network| network.is_object())
        .cloned()
        .ok_or((
            NETWORK_INVALID,
            "The saved package holds no water network.".into(),
        ))
}

/// `water.run`: one operation, on a saved network when `model` names one.
fn run(request: Map<String, Value>, release: Option<String>) -> Result<Value, Refused> {
    let (operation, payload) = operation(&request)?;
    let data = match request.get("model").filter(|model| model.is_object()) {
        // `identity` names the release, as the unit's own entry answers it.
        None => {
            let mut answer = invoke(&operation, payload)?;
            if let (Some(release), "identity", Some(fields)) =
                (release, operation.as_str(), answer.as_object_mut())
            {
                fields.insert("release".into(), release.into());
            }
            answer
        }
        Some(model) => {
            let network = opened(model)?;
            let answer = invoke(&operation, on_network(payload, network.clone()))?;
            on_saved(
                &operation,
                model["model_id"].as_str().unwrap_or_default(),
                model["version"].as_i64().unwrap_or_default(),
                &network,
                answer,
            )
        }
    };
    Ok(json!({"status": "ok", "data": data}))
}

/// `water.save`: run one operation and plan the network it answers as the
/// next version of the network `name` names, or of the saved `model` it ran
/// on.
fn save(request: Map<String, Value>, release: Option<String>) -> Result<Value, Refused> {
    let (operation, payload) = operation(&request)?;
    let model = request.get("model").filter(|model| model.is_object());
    let name = request.get("name").filter(|name| name.is_object());
    let payload = match model {
        Some(model) => on_network(payload, opened(model)?),
        None => payload,
    };
    let (target, member) = match (name, model) {
        (Some(name), _) => ("name", name),
        (None, Some(model)) => ("model", model),
        (None, None) => {
            return Err((
                INPUT_INVALID,
                "Name the network, or the saved network to run on.".into(),
            ));
        }
    };
    let release = release.ok_or((
        UNIT_OUTDATED,
        "This Water unit names no release, so a network it made cannot be saved; update the Water unit."
            .into(),
    ))?;
    let answer = invoke(&operation, payload)?;
    let run_summary = summary(&operation, &answer);
    let shown = map(&answer["network"]);
    let network = &answer["network"];
    if !network.is_object() || !network.get("pipes").is_some_and(Value::is_array) {
        return Err((
            NETWORK_INVALID,
            "The answer holds no water network to save.".into(),
        ));
    }
    let pack = invoke(
        "pack",
        json!({"network": network, "model_id": member["model_id"], "revision": member["revision"], "project": member["project"]}),
    )?;
    let message = text(&request, "message")
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Saved from {operation}"));
    Ok(json!({
        "status": "ok",
        "data": {
            "operation": operation, "sheets": [],
            "network": counts(network), "summary": run_summary, "map": shown,
        },
        "publish": {
            "target": target,
            "source": model.map(|_| "model"),
            "message": message,
            "engine_release": release,
            "pack": pack,
        },
    }))
}

/// The request an operation takes: a bare GeoJSON FeatureCollection (a
/// file of water features, as Lay out reads one) is the request's `features`.
pub fn features_request(request: Value) -> Value {
    if request.get("type").and_then(Value::as_str) == Some("FeatureCollection") {
        json!({ "features": request })
    } else {
        request
    }
}

/// Bind a selected customer file to Process; other feature inputs retain their meaning.
pub fn operation_request(operation: &str, request: Value) -> Value {
    if operation == "process"
        && request.get("type").and_then(Value::as_str) == Some("FeatureCollection")
    {
        json!({"customers": request})
    } else {
        features_request(request)
    }
}

/// Hand a saved network to an operation's request.
pub fn on_network(request: Value, network: Value) -> Value {
    let mut request = match request {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    request.insert("network".into(), network);
    Value::Object(request)
}

fn number(value: &Value, digits: usize) -> String {
    value
        .as_f64()
        .map_or_else(|| "–".into(), |n| format!("{n:.digits$}"))
}

fn counted(value: &Value) -> usize {
    value.as_array().map_or(0, Vec::len)
}

/// A few lines that say what an operation found.
pub fn summary(operation: &str, answer: &Value) -> Vec<String> {
    match operation {
        "layout" => {
            let mut lines = vec![format!(
                "{} paths, {} points not reached",
                number(&answer["paths"], 0),
                counted(&answer["unserved"])
            )];
            lines.extend(
                answer["notes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned),
            );
            lines
        }
        "validate" => vec![format!("{} validation issues", counted(&answer["issues"]))],
        "process" => vec![
            format!(
                "{} connected, {} unconnected",
                number(&answer["connected"], 0),
                number(&answer["unconnected"], 0)
            ),
            format!("{} fittings", counted(&answer["fittings"])),
        ],
        "design" => vec![
            format!("Cost {}", number(&answer["total_cost"], 0)),
            format!(
                "Lowest pressure {} m at {}",
                number(&answer["min_pressure"]["pressure_m"], 1),
                answer["min_pressure"]["id"].as_str().unwrap_or("–")
            ),
            format!(
                "{} nodes short of pressure, {} over pressure, {} break-pressure tanks",
                counted(&answer["infeasible"]),
                counted(&answer["over_pressure"]),
                counted(&answer["break_pressure_tanks"])
            ),
            format!("{} warnings", counted(&answer["warnings"])),
        ],
        "simulate" => {
            let steps = answer["steps"].as_array().map_or(&[][..], Vec::as_slice);
            let lowest = steps
                .iter()
                .flat_map(|step| step["pressure_m"].as_array().into_iter().flatten())
                .filter_map(Value::as_f64)
                .fold(f64::INFINITY, f64::min);
            vec![
                format!(
                    "{} after {} steps",
                    if answer["ok"] == true {
                        "Converged"
                    } else {
                        "Did not converge"
                    },
                    steps.len()
                ),
                format!(
                    "Lowest pressure {} m",
                    if lowest.is_finite() {
                        format!("{lowest:.1}")
                    } else {
                        "–".into()
                    }
                ),
            ]
        }
        "profile" => vec![
            format!("Length {} m", number(&answer["length_m"], 0)),
            format!(
                "{} fittings, {} findings",
                counted(&answer["fittings"]),
                counted(&answer["findings"])
            ),
        ],
        "draw" => vec![format!("{} sheets", counted(&answer["sheets"]))],
        "scheme" => vec![
            format!("Cost {}", number(&answer["design"]["total_cost"], 0)),
            format!(
                "Lowest pressure {} m at {}",
                number(&answer["design"]["min_pressure"]["pressure_m"], 1),
                answer["design"]["min_pressure"]["id"]
                    .as_str()
                    .unwrap_or("–")
            ),
            format!(
                "{} paths, {} points not reached, {} sheets",
                number(&answer["layout"]["paths"], 0),
                counted(&answer["layout"]["unserved"]),
                counted(&answer["sheets"])
            ),
        ],
        _ => Vec::new(),
    }
}

/// A node's `[x, y]`, when it has both.
fn at(node: &Value) -> Option<[f64; 2]> {
    Some([node.get("x")?.as_f64()?, node.get("y")?.as_f64()?])
}

/// A network in lon/lat as map features: each pipe a line through its
/// vertices with its diameter, each node a point with its kind. A network in
/// any other reference has no map features.
pub fn map(network: &Value) -> Value {
    let mut features = Vec::new();
    if network.get("crs").and_then(Value::as_str) == Some("EPSG:4326") {
        let mut nodes = std::collections::HashMap::new();
        for (key, kind) in [
            ("junctions", "junction"),
            ("reservoirs", "source"),
            ("tanks", "tank"),
        ] {
            for node in network[key].as_array().into_iter().flatten() {
                let (Some(id), Some(xy)) = (node["id"].as_str(), at(node)) else {
                    continue;
                };
                nodes.insert(id.to_owned(), xy);
                features.push(json!({
                    "type": "Feature",
                    "properties": {"kind": kind, "id": id, "demand_lps": node.get("demand_lps"), "head_m": node.get("head_m")},
                    "geometry": {"type": "Point", "coordinates": xy},
                }));
            }
        }
        for pipe in network["pipes"].as_array().into_iter().flatten() {
            let ends = (
                pipe["from"].as_str().and_then(|id| nodes.get(id)),
                pipe["to"].as_str().and_then(|id| nodes.get(id)),
            );
            let (Some(from), Some(to)) = ends else {
                continue;
            };
            let mut line = vec![*from];
            line.extend(
                pipe["vertices"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| Some([v.get(0)?.as_f64()?, v.get(1)?.as_f64()?])),
            );
            line.push(*to);
            features.push(json!({
                "type": "Feature",
                "properties": {"kind": "pipe", "id": pipe["id"], "diameter_mm": pipe["diameter_mm"], "length_m": pipe["length_m"]},
                "geometry": {"type": "LineString", "coordinates": line},
            }));
        }
    }
    json!({"type": "FeatureCollection", "features": features})
}

/// A run on a saved network as the page and the command line read it; its
/// map shows the network the run answered (a design's), else the one opened.
pub fn on_saved(
    operation: &str,
    model: &str,
    version: i64,
    opened: &Value,
    answer: Value,
) -> Value {
    let shown = if answer["network"].is_object() {
        &answer["network"]
    } else {
        opened
    };
    json!({
        "operation": operation,
        "model": model,
        "version": version,
        "summary": summary(operation, &answer),
        "sheets": answer.get("sheets").cloned().unwrap_or_else(|| json!([])),
        "map": map(shown),
        "answer": answer,
    })
}

/// A saved network's size: junctions, links and pipe length.
pub fn counts(network: &Value) -> Value {
    let count = |key: &str| {
        network
            .get(key)
            .and_then(Value::as_array)
            .map_or(0, |items| items.len())
    };
    let length_m: f64 = network["pipes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|pipe| pipe.get("length_m").and_then(Value::as_f64))
        .sum();
    json!({"junctions": count("junctions"),
        "links": count("pipes") + count("pumps") + count("valves"), "length_m": length_m})
}

#[cfg(test)]
#[path = "door_tests.rs"]
mod tests;
