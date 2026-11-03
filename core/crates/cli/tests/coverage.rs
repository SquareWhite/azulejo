//! Every API method must be reachable from `azj`, with every one of its params.
//! Adding a method or param to the protocol without the CLI understanding it
//! fails here.

use azulejo_cli::args::{dashed, parse_params};
use azulejo_cli::spec::{MethodSpec, Param, Ty, methods};
use azulejo_protocol::{Command, METHODS, TabId, TileId, WindowId, WorkspaceId};
use serde_json::{Value, json};

fn sample_id(prefix: &str) -> String {
    match prefix {
        "tile" => TileId::new().to_string(),
        "tab" => TabId::new().to_string(),
        "ws" => WorkspaceId::new().to_string(),
        "win" => WindowId::new().to_string(),
        other => panic!("no sample for {other}_ IDs; add one"),
    }
}

/// Flags that set `p` to some valid value.
fn flags_for(p: &Param) -> Vec<String> {
    let flag = format!("--{}", dashed(&p.name));
    match &p.ty {
        Ty::String => vec![flag, "text".into()],
        Ty::Id(prefix) => vec![flag, sample_id(prefix)],
        Ty::Integer => vec![flag, "1".into()],
        Ty::Number => vec![flag, "0.1".into()],
        Ty::Bool => vec![flag],
        Ty::Enum(values) => vec![flag, values[0].clone()],
        Ty::StringList => vec![flag.clone(), "a".into(), flag, "b".into()],
        Ty::Object(fields) => fields.iter().flat_map(flags_for).collect(),
        Ty::Tagged { tag, variants } => {
            // The last variant, with all its fields, to exercise field flags.
            let (name, fields) = variants.last().expect("unions have variants");
            let mut v = vec![format!("--{}", dashed(tag)), name.clone()];
            v.extend(fields.iter().flat_map(flags_for));
            v
        }
        Ty::Json => vec![flag, "{}".into()],
    }
}

fn check(spec: &MethodSpec, params: &[&Param]) {
    let argv: Vec<String> = params.iter().flat_map(|p| flags_for(p)).collect();
    let parsed = parse_params(spec, &argv).unwrap_or_else(|e| {
        panic!(
            "`azj {} {} {}` didn't parse: {e}",
            spec.group,
            spec.action,
            argv.join(" ")
        )
    });
    for p in params {
        assert!(
            parsed.contains_key(&p.name),
            "{}: --{} was dropped",
            spec.method,
            p.name
        );
    }
    if azulejo_api::SERVER_METHODS.contains(&spec.method.as_str()) {
        return;
    }
    let mut request = json!({"method": spec.method});
    if !parsed.is_empty() {
        request["params"] = Value::Object(parsed);
    }
    Command::from_json(&request.to_string()).unwrap_or_else(|e| {
        panic!(
            "`azj {} {}` sent params the core rejects: {e}\n{request}",
            spec.group, spec.action
        )
    });
}

#[test]
fn every_method_parses_with_required_params() {
    let specs = methods();
    for method in METHODS.iter().chain(azulejo_api::SERVER_METHODS) {
        let spec = specs
            .iter()
            .find(|s| s.method == *method)
            .unwrap_or_else(|| panic!("no CLI subcommand for `{method}`"));
        let required: Vec<&Param> = spec.params.iter().filter(|p| p.required).collect();
        check(spec, &required);
    }
}

#[test]
fn every_param_of_every_method_has_a_flag() {
    for spec in methods() {
        // Each param on its own, plus the required ones.
        for p in &spec.params {
            let mut params: Vec<&Param> = spec.params.iter().filter(|q| q.required).collect();
            if !p.required {
                params.push(p);
            }
            check(&spec, &params);
        }
    }
}
