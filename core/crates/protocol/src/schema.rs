//! The JSON Schema for the whole API, written to `schema/api.v1.json`.

use schemars::generate::SchemaSettings;

use crate::{API_VERSION, Command, Error, Event, METHODS, Notification, Reply};

/// The API schema: `Command`, `Event`, `Reply`, `Error` and `Notification`
/// (event-stream messages) plus everything they
/// reference, under `$defs`.
pub fn api_schema() -> serde_json::Value {
    let mut generator = SchemaSettings::draft2020_12().into_generator();
    generator.subschema_for::<Command>();
    generator.subschema_for::<Event>();
    generator.subschema_for::<Reply>();
    generator.subschema_for::<Error>();
    generator.subschema_for::<Notification>();
    let defs: serde_json::Map<String, serde_json::Value> = generator
        .take_definitions(true)
        .into_iter()
        .collect::<std::collections::BTreeMap<_, _>>()
        .into_iter()
        .collect();
    serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": format!("https://azulejo.dev/schema/api.v{API_VERSION}.json"),
        "title": "Azulejo API",
        "description": "Generated from core/crates/protocol by `just schema`. Do not edit.",
        "version": API_VERSION,
        "methods": METHODS,
        "$defs": defs,
    })
}

/// The schema as committed: pretty JSON with a trailing newline.
pub fn api_schema_text() -> String {
    let mut text = serde_json::to_string_pretty(&api_schema()).expect("schema serialises");
    text.push('\n');
    text
}
