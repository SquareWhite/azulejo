//! The CLI's view of the API, derived from the protocol's JSON Schema so every
//! method gets a subcommand without anyone writing one by hand.

use serde_json::{Map, Value};

/// A subcommand: `azj <group> <action>` for method `<group>.<action>`.
#[derive(Debug, Clone)]
pub struct MethodSpec {
    pub method: String,
    pub group: String,
    /// The action as typed: underscores become dashes (`send-text`).
    pub action: String,
    pub description: Option<String>,
    pub params: Vec<Param>,
}

/// One parameter of a method.
#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub required: bool,
    pub description: Option<String>,
    pub ty: Ty,
}

#[derive(Debug, Clone)]
pub enum Ty {
    String,
    /// A prefixed ID such as `tile_…`; holds the prefix without `_`.
    Id(String),
    Integer,
    Number,
    Bool,
    Enum(Vec<String>),
    StringList,
    /// A struct; its fields can be given as flags of their own.
    Object(Vec<Param>),
    /// A tagged union such as tile content: `tag` picks the variant.
    Tagged {
        tag: String,
        variants: Vec<(String, Vec<Param>)>,
    },
    /// Anything else; given as JSON.
    Json,
}

impl Ty {
    pub fn describe(&self) -> String {
        match self {
            Ty::String => "text".into(),
            Ty::Id(prefix) => format!("{prefix}_…"),
            Ty::Integer => "integer".into(),
            Ty::Number => "number".into(),
            Ty::Bool => "flag".into(),
            Ty::Enum(values) => values.join("|"),
            Ty::StringList => "text…".into(),
            Ty::Object(_) => "object".into(),
            Ty::Tagged { variants, .. } => variants
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join("|"),
            Ty::Json => "json".into(),
        }
    }
}

/// Methods the server answers itself; they have no params and aren't in the
/// protocol's `Command` enum.
const SERVER_METHOD_DOCS: &[(&str, &str)] = &[
    ("core.version", "The core's version and API version."),
    (
        "events.subscribe",
        "Streams events. Not available yet (PLAN step 2.8).",
    ),
];

/// Every method, in schema order, then the server's own methods.
pub fn methods() -> Vec<MethodSpec> {
    let schema = azulejo_protocol::api_schema();
    let defs = schema["$defs"].as_object().expect("schema has $defs");
    let variants = defs["Command"]["oneOf"]
        .as_array()
        .expect("Command is a oneOf");
    let mut specs: Vec<MethodSpec> = variants
        .iter()
        .map(|variant| {
            let method = variant["properties"]["method"]["const"]
                .as_str()
                .expect("each command variant names its method")
                .to_owned();
            let params = match variant["properties"].get("params") {
                Some(params) => object_params(&resolve(params, defs), defs),
                None => Vec::new(),
            };
            spec(method, description(variant), params)
        })
        .collect();
    for (method, doc) in SERVER_METHOD_DOCS {
        debug_assert!(azulejo_api::SERVER_METHODS.contains(method));
        specs.push(spec(
            (*method).to_owned(),
            Some((*doc).to_owned()),
            Vec::new(),
        ));
    }
    specs
}

fn spec(method: String, description: Option<String>, params: Vec<Param>) -> MethodSpec {
    let (group, action) = method.split_once('.').expect("methods are group.action");
    MethodSpec {
        group: group.to_owned(),
        action: action.replace('_', "-"),
        method,
        description,
        params,
    }
}

fn description(schema: &Value) -> Option<String> {
    schema
        .get("description")
        .and_then(Value::as_str)
        .map(|d| d.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Follows `$ref`s and strips the `null` from optional fields.
fn resolve(schema: &Value, defs: &Map<String, Value>) -> Value {
    if let Some(name) = ref_name(schema) {
        let mut target = resolve(&defs[name], defs);
        // A description on the referring field beats the type's own.
        if let (Some(d), Value::Object(t)) = (schema.get("description"), &mut target) {
            t.insert("description".into(), d.clone());
        }
        return target;
    }
    if let Some(any) = schema.get("anyOf").and_then(Value::as_array) {
        let non_null: Vec<&Value> = any
            .iter()
            .filter(|s| s.get("type") != Some(&Value::from("null")))
            .collect();
        if let [only] = non_null.as_slice() {
            let mut target = resolve(only, defs);
            if let (Some(d), Value::Object(t)) = (schema.get("description"), &mut target) {
                t.insert("description".into(), d.clone());
            }
            return target;
        }
    }
    if let Some(Value::Array(types)) = schema.get("type") {
        let non_null: Vec<&Value> = types.iter().filter(|t| *t != "null").collect();
        if let [only] = non_null.as_slice() {
            let mut s = schema.clone();
            s["type"] = (*only).clone();
            return s;
        }
    }
    schema.clone()
}

fn ref_name(schema: &Value) -> Option<&str> {
    schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| r.strip_prefix("#/$defs/"))
}

fn object_params(schema: &Value, defs: &Map<String, Value>) -> Vec<Param> {
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let Some(props) = schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    props
        .iter()
        .map(|(name, prop)| {
            let resolved = resolve(prop, defs);
            Param {
                name: name.clone(),
                required: required.contains(&name.as_str()),
                description: description(&resolved),
                ty: classify(&resolved, defs),
            }
        })
        .collect()
}

fn classify(schema: &Value, defs: &Map<String, Value>) -> Ty {
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return Ty::Enum(
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        );
    }
    if let Some(one_of) = schema.get("oneOf").and_then(Value::as_array) {
        // An enum whose variants carry docs: [{"const": "visible"}, …].
        let consts: Option<Vec<String>> = one_of
            .iter()
            .map(|v| v.get("const").and_then(Value::as_str).map(str::to_owned))
            .collect();
        if let Some(values) = consts {
            return Ty::Enum(values);
        }
        // A tagged union: every variant has one property with a `const`.
        let tag = one_of.first().and_then(|v| {
            v.get("properties")?
                .as_object()?
                .iter()
                .find(|(_, p)| p.get("const").is_some())
                .map(|(name, _)| name.clone())
        });
        if let Some(tag) = tag {
            let variants: Option<Vec<(String, Vec<Param>)>> = one_of
                .iter()
                .map(|v| {
                    let name = v["properties"]
                        .get(&tag)?
                        .get("const")?
                        .as_str()?
                        .to_owned();
                    let fields = object_params(v, defs)
                        .into_iter()
                        .filter(|p| p.name != tag)
                        .collect();
                    Some((name, fields))
                })
                .collect();
            if let Some(variants) = variants {
                return Ty::Tagged { tag, variants };
            }
        }
        return Ty::Json;
    }
    match schema.get("type").and_then(Value::as_str) {
        Some("string") => match schema.get("pattern").and_then(Value::as_str) {
            Some(pattern) => match pattern
                .strip_prefix('^')
                .and_then(|p| p.split_once('_'))
                .map(|(prefix, _)| prefix)
                .filter(|p| p.chars().all(|c| c.is_ascii_lowercase()))
            {
                Some(prefix) => Ty::Id(prefix.to_owned()),
                None => Ty::String,
            },
            None => Ty::String,
        },
        Some("integer") => Ty::Integer,
        Some("number") => Ty::Number,
        Some("boolean") => Ty::Bool,
        Some("array")
            if schema.get("items").and_then(|i| i.get("type")) == Some(&Value::from("string")) =>
        {
            Ty::StringList
        }
        Some("object") if schema.get("properties").is_some() => {
            Ty::Object(object_params(schema, defs))
        }
        _ => Ty::Json,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_method_has_a_subcommand() {
        let specs = methods();
        let mut all: Vec<&str> = azulejo_protocol::METHODS.to_vec();
        all.extend(azulejo_api::SERVER_METHODS);
        for method in all {
            assert!(
                specs.iter().any(|s| s.method == method),
                "no CLI subcommand for `{method}`"
            );
        }
        assert_eq!(
            specs.len(),
            azulejo_protocol::METHODS.len() + azulejo_api::SERVER_METHODS.len()
        );
    }

    #[test]
    fn split_params_are_understood() {
        let split = methods()
            .into_iter()
            .find(|s| s.method == "layout.split")
            .unwrap();
        let ty = |name: &str| &split.params.iter().find(|p| p.name == name).unwrap().ty;
        assert!(matches!(ty("tile"), Ty::Id(p) if p == "tile"));
        assert!(matches!(ty("direction"), Ty::Enum(v) if v.contains(&"right".to_owned())));
        assert!(matches!(ty("content"), Ty::Tagged { tag, .. } if tag == "kind"));
    }
}
