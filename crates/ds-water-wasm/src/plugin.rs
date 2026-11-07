//! What the Water unit hands the Kernel's generic component door (product
//! scope §7): its `ds water …` commands (`ds.plugin-commands/v1`) and its own
//! Design area, Water Network (`ds.plugin-ribbon/v1` with an `area`). These
//! are two separate registrations, so a Kernel that refuses the area ribbon
//! still holds the commands.
//!
//! The area is the Kernel's Water arm moved as it is (ds-cli e09cbcea2:
//! `application_menus_distribution::water_groups` and `model_row`'s Water
//! lead) with the words the page shows today. The Kernel keeps the shared
//! model frame (`water_setup` … `water_profile`) in the `water_network`
//! group, where the page lends the network selector.

use crate::door;
use serde_json::{Value, json};

pub const NAMESPACE: &str = "water";

/// The project model kind this unit computes on.
pub const MODEL_KIND: &str = "water_network";

/// The operations `water.run` and `water.save` take, as Core declares them.
const OPERATIONS: &str = "identity, validate, simulate, analysis, layout, process, design, profile, draw, scheme or example";

/// The `ds water …` commands, as Core declares them (ds-cli-design `water.rs`).
/// The door supplies the project; Core keeps the lane and the payload file.
pub fn manifest() -> Value {
    json!({
        "schema": "ds.plugin-commands/v1",
        "plugin": "water",
        "namespace": NAMESPACE,
        // The project model kind the Kernel reads and publishes for Water, as
        // its built-in record declares it (ds-command-kernel `water_models.rs`).
        "models": [{
            "kind": MODEL_KIND,
            "noun": "water network",
            "title": "Water network",
            "short": "network",
            "engine": "Water engine",
            "id_prefix": "water_",
            "source_prefix": "water-r",
            "input_invalid": "water_models_input_invalid",
            "model_invalid": "water_network_invalid",
            "source_changed": "water_source_changed"
        }],
        // `water.models` lists the project's saved networks: the Kernel's own
        // read by the model kind, so the unit declares no door for it.
        "commands": [
            {
                "words": ["water", "run"],
                "summary": "Lay out, process, design, profile and draw water supply networks.",
                "members": [
                    {"name": "operation", "type": "text", "required": true,
                     "help": format!("The Water operation: {OPERATIONS}.")},
                    {"name": "payload", "type": "json", "help": "The operation's request as one JSON object."},
                    {"name": "model", "type": "model", "kind": MODEL_KIND,
                     "help": "Run on this saved Water network's current governed revision."}
                ],
                "examples": ["ds water run --operation identity"],
                "refusals": [door::REQUEST_INVALID, door::ENGINE_REFUSED, door::NETWORK_INVALID]
            },
            {
                "words": ["water", "save"],
                "summary": "Save the network a Water operation makes as a project's next version.",
                "effect": "project_write",
                "members": [
                    {"name": "name", "type": "model_name", "kind": MODEL_KIND,
                     "help": "The network's name; it makes the model id water_<slug>."},
                    {"name": "model", "type": "model", "kind": MODEL_KIND,
                     "help": "Save the result as this saved network's next version."},
                    {"name": "operation", "type": "text", "required": true,
                     "help": format!("The Water operation: {OPERATIONS}.")},
                    {"name": "payload", "type": "json", "help": "The operation's request as one JSON object."},
                    {"name": "message", "type": "text", "help": "Why this version exists."}
                ],
                "examples": ["ds water save --project <id> --name \"Test village 1\" --operation example"],
                "refusals": [door::REQUEST_INVALID, door::ENGINE_REFUSED, door::NETWORK_INVALID,
                             door::UNIT_OUTDATED, door::INPUT_INVALID]
            }
        ]
    })
}

fn page(id: &str, icon: &str, en: &str, fr: &str) -> Value {
    json!({"page": id, "icon": icon, "label": {"en": en, "fr": fr}})
}

/// Water Network, Design's area for a water network (owner, 2–3 Nov 2026:
/// it mimics Distribution and Transmission): Settings, the network frame,
/// Engineer and Output; the Kernel adds the shared frame and Design's reports.
pub fn ribbon() -> Value {
    json!({
        "schema": "ds.plugin-ribbon/v1",
        "namespace": NAMESPACE,
        "area": {
            "id": "water",
            "kind": "water",
            "icon": "map",
            "frame": "water_network",
            "lead": [page("water_open", "folder", "Open network", "Ouvrir le réseau")]
        },
        "label": {"en": "Water Network", "fr": "Réseau d'eau"},
        "groups": [
            {"id": "settings", "label": {"en": "Settings", "fr": "Paramètres"}, "entries": [
                page("water_library", "library", "Library", "Bibliothèque"),
                page("water_layout", "add", "New", "Nouveau"),
                page("water_import", "import", "Import", "Importer"),
                page("water_refresh", "refresh", "Refresh", "Actualiser")
            ]},
            {"id": "engineer", "label": {"en": "Engineer", "fr": "Ingénierie"}, "entries": [
                page("water_process", "process", "Process", "Traiter"),
                page("water_autoprocess", "autoprocess", "Auto-process", "Traitement auto"),
                page("water_design", "pipe_size", "Size pipes", "Dimensionner les conduites"),
                page("water_simulate", "statistics", "Simulate", "Simuler")
            ]},
            {"id": "output", "label": {"en": "Output", "fr": "Sortie"}, "entries": [
                page("water_draw", "print", "Drawings", "Plans"),
                page("water_export", "export", "Export", "Exporter")
            ]}
        ]
    })
}

#[cfg(test)]
#[path = "plugin_tests.rs"]
mod tests;
