//! Turns `--flag value` arguments into a method's params, guided by its spec.
//!
//! Beyond plain `--name value` for each param, a few shortcuts keep common
//! commands short:
//! - an enum value is a flag of its own: `--right` for `--direction right`;
//! - a tagged union's tag and fields are flags too: `--kind browser --url …`,
//!   or just `--browser --url …`, or just `--url …` when only one variant has
//!   a `url`;
//! - a struct's fields are flags: `--pane-id … --terminal-id …`;
//! - IDs and the one remaining required param can be positional:
//!   `azj tile get tile_…`, `azj terminal send-text 'ls'`.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::spec::{MethodSpec, Param, Ty};

/// A union variant: its tag value and its fields.
type Variant = (String, Vec<Param>);

/// A union param being assembled from flags.
#[derive(Default)]
struct PendingUnion {
    variant: Option<String>,
    /// Raw flag values by field name; converted once the variant is known.
    fields: BTreeMap<String, Vec<String>>,
}

pub fn parse_params(spec: &MethodSpec, args: &[String]) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    let mut unions: BTreeMap<String, PendingUnion> = BTreeMap::new();
    let mut objects: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
    let mut positionals = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        i += 1;
        if arg == "--" {
            positionals.extend(args[i..].iter().cloned());
            break;
        }
        let Some(flag) = arg.strip_prefix("--") else {
            positionals.push(arg.clone());
            continue;
        };
        let (name, inline) = match flag.split_once('=') {
            Some((n, v)) => (n, Some(v.to_owned())),
            None => (flag, None),
        };
        let key = name.replace('-', "_");
        // `--no-kill`
        if let Some(neg) = key.strip_prefix("no_")
            && let Some(p) = param(spec, neg).filter(|p| matches!(p.ty, Ty::Bool))
        {
            out.insert(p.name.clone(), Value::Bool(false));
            continue;
        }

        // 1. A param by name.
        if let Some(p) = param(spec, &key) {
            match &p.ty {
                Ty::Bool => {
                    let value = match &inline {
                        Some(v) => parse_bool(v)
                            .ok_or_else(|| format!("--{name}: `{v}` isn't true or false"))?,
                        None => match args.get(i).and_then(|v| parse_bool(v)) {
                            Some(b) => {
                                i += 1;
                                b
                            }
                            None => true,
                        },
                    };
                    out.insert(p.name.clone(), Value::Bool(value));
                }
                Ty::StringList => {
                    let v = take_value(args, &mut i, &inline, name, "text")?;
                    push_list(&mut out, &p.name, v);
                }
                Ty::Tagged { variants, .. } => {
                    let v = take_value(args, &mut i, &inline, name, &p.ty.describe())?;
                    if v.trim_start().starts_with('{') {
                        out.insert(p.name.clone(), parse_json(name, &v)?);
                    } else if variants.iter().any(|(n, _)| *n == v) {
                        unions.entry(p.name.clone()).or_default().variant = Some(v);
                    } else {
                        return Err(format!("--{name}: `{v}` isn't one of {}", p.ty.describe()));
                    }
                }
                ty => {
                    let v = take_value(args, &mut i, &inline, name, &ty.describe())?;
                    out.insert(p.name.clone(), convert(name, ty, &v)?);
                }
            }
            continue;
        }

        // 2. A union's tag, e.g. `--kind browser`.
        let tagged = |pred: &dyn Fn(&str, &[Variant]) -> bool| -> Vec<&Param> {
            spec.params
                .iter()
                .filter(|p| match &p.ty {
                    Ty::Tagged { tag, variants } => pred(tag, variants),
                    _ => false,
                })
                .collect()
        };
        let by_tag = tagged(&|tag, _| tag == key);
        if let [p] = by_tag.as_slice() {
            let Ty::Tagged { variants, .. } = &p.ty else {
                unreachable!()
            };
            let v = take_value(args, &mut i, &inline, name, &p.ty.describe())?;
            if !variants.iter().any(|(n, _)| *n == v) {
                return Err(format!("--{name}: `{v}` isn't one of {}", p.ty.describe()));
            }
            unions.entry(p.name.clone()).or_default().variant = Some(v);
            continue;
        }

        // 3. A union variant's field, e.g. `--url …`.
        let by_field = tagged(&|_, variants| {
            variants
                .iter()
                .any(|(_, fields)| fields.iter().any(|f| f.name == key))
        });
        if let [p] = by_field.as_slice() {
            let Ty::Tagged { variants, .. } = &p.ty else {
                unreachable!()
            };
            let field = variants
                .iter()
                .flat_map(|(_, fields)| fields)
                .find(|f| f.name == key)
                .expect("matched above");
            let raw = match field.ty {
                Ty::Bool if inline.is_none() => "true".to_owned(),
                _ => take_value(args, &mut i, &inline, name, &field.ty.describe())?,
            };
            unions
                .entry(p.name.clone())
                .or_default()
                .fields
                .entry(key.clone())
                .or_default()
                .push(raw);
            continue;
        }

        // 4. A struct field, e.g. `--pane-id …`.
        let by_object: Vec<(&Param, &Param)> = spec
            .params
            .iter()
            .filter_map(|p| match &p.ty {
                Ty::Object(fields) => fields.iter().find(|f| f.name == key).map(|f| (p, f)),
                _ => None,
            })
            .collect();
        if let [(p, field)] = by_object.as_slice() {
            let v = take_value(args, &mut i, &inline, name, &field.ty.describe())?;
            objects
                .entry(p.name.clone())
                .or_default()
                .insert(field.name.clone(), convert(name, &field.ty, &v)?);
            continue;
        }

        // 5. An enum value or union variant as a flag, e.g. `--right`, `--browser`.
        if inline.is_none() {
            let enums: Vec<&Param> = spec
                .params
                .iter()
                .filter(|p| matches!(&p.ty, Ty::Enum(values) if values.contains(&key)))
                .collect();
            if let [p] = enums.as_slice() {
                out.insert(p.name.clone(), Value::String(key));
                continue;
            }
            let variants = tagged(&|_, variants| variants.iter().any(|(n, _)| *n == key));
            if enums.is_empty()
                && let [p] = variants.as_slice()
            {
                unions.entry(p.name.clone()).or_default().variant = Some(key);
                continue;
            }
        }

        return Err(format!(
            "unknown option --{name} for `{} {}`",
            spec.group, spec.action
        ));
    }

    for (name, pending) in unions {
        if out.contains_key(&name) {
            return Err(format!(
                "--{} given both as JSON and as flags",
                dashed(&name)
            ));
        }
        let p = param(spec, &name).expect("union params come from the spec");
        out.insert(name, finish_union(p, pending)?);
    }
    for (name, fields) in objects {
        if out.contains_key(&name) {
            return Err(format!(
                "--{} given both as JSON and as flags",
                dashed(&name)
            ));
        }
        let p = param(spec, &name).expect("object params come from the spec");
        if let Ty::Object(subs) = &p.ty
            && let Some(missing) = subs
                .iter()
                .find(|f| f.required && !fields.contains_key(&f.name))
        {
            return Err(format!(
                "--{} also needs --{}",
                dashed(&name),
                dashed(&missing.name)
            ));
        }
        out.insert(name, Value::Object(fields));
    }

    assign_positionals(spec, &mut out, positionals)?;
    // `azj layout apply` alone opens the working directory's layout file
    // (made absolute by the caller like any path).
    if spec.method == "layout.apply" && !out.contains_key("path") && !out.contains_key("toml") {
        out.insert("path".into(), Value::String(".".into()));
    }

    if let Some(missing) = spec
        .params
        .iter()
        .find(|p| p.required && !out.contains_key(&p.name))
    {
        return Err(format!(
            "`{} {}` needs --{} ({})",
            spec.group,
            spec.action,
            dashed(&missing.name),
            missing.ty.describe()
        ));
    }
    Ok(out)
}

fn take_value(
    args: &[String],
    i: &mut usize,
    inline: &Option<String>,
    name: &str,
    what: &str,
) -> Result<String, String> {
    if let Some(v) = inline {
        return Ok(v.clone());
    }
    let v = args
        .get(*i)
        .ok_or_else(|| format!("--{name} needs a value ({what})"))?;
    *i += 1;
    Ok(v.clone())
}

fn param<'a>(spec: &'a MethodSpec, key: &str) -> Option<&'a Param> {
    spec.params.iter().find(|p| p.name == key)
}

pub fn dashed(name: &str) -> String {
    name.replace('_', "-")
}

fn push_list(out: &mut Map<String, Value>, name: &str, v: String) {
    match out
        .entry(name.to_owned())
        .or_insert_with(|| Value::Array(Vec::new()))
    {
        Value::Array(items) => items.push(Value::String(v)),
        other => *other = Value::Array(vec![Value::String(v)]),
    }
}

fn finish_union(p: &Param, pending: PendingUnion) -> Result<Value, String> {
    let Ty::Tagged { tag, variants } = &p.ty else {
        unreachable!("only tagged params are pending unions")
    };
    let variant = match pending.variant {
        Some(v) => v,
        None => {
            let fitting: Vec<&String> = variants
                .iter()
                .filter(|(_, fields)| {
                    pending
                        .fields
                        .keys()
                        .all(|k| fields.iter().any(|f| f.name == *k))
                })
                .map(|(n, _)| n)
                .collect();
            match fitting.as_slice() {
                [only] => (*only).clone(),
                _ => {
                    return Err(format!(
                        "say which --{} this is ({})",
                        dashed(tag),
                        p.ty.describe()
                    ));
                }
            }
        }
    };
    let fields = &variants
        .iter()
        .find(|(n, _)| *n == variant)
        .expect("variant was checked")
        .1;
    let mut obj = Map::new();
    obj.insert(tag.clone(), Value::String(variant.clone()));
    for (key, raws) in pending.fields {
        let field = fields.iter().find(|f| f.name == key).ok_or_else(|| {
            format!(
                "--{} doesn't apply to {} {variant}",
                dashed(&key),
                dashed(tag)
            )
        })?;
        let value = match &field.ty {
            Ty::StringList => Value::Array(raws.into_iter().map(Value::String).collect()),
            ty => {
                let raw = raws.last().expect("at least one value");
                convert(&dashed(&key), ty, raw)?
            }
        };
        obj.insert(key, value);
    }
    Ok(Value::Object(obj))
}

/// Hands leftover bare arguments to the params they most likely mean.
fn assign_positionals(
    spec: &MethodSpec,
    out: &mut Map<String, Value>,
    positionals: Vec<String>,
) -> Result<(), String> {
    let mut rest = Vec::new();
    for arg in positionals {
        // An ID goes to the one unset param of its kind.
        let id_params: Vec<&Param> = spec
            .params
            .iter()
            .filter(|p| !out.contains_key(&p.name))
            .filter(|p| matches!(&p.ty, Ty::Id(prefix) if is_id(&arg, prefix)))
            .collect();
        match id_params.as_slice() {
            [p] => {
                out.insert(p.name.clone(), Value::String(arg));
            }
            _ => rest.push(arg),
        }
    }
    if rest.is_empty() {
        return Ok(());
    }

    let unset = |pred: &dyn Fn(&Param) -> bool| -> Vec<&Param> {
        spec.params
            .iter()
            .filter(|p| !out.contains_key(&p.name) && pred(p))
            .collect()
    };
    let positional_ty = |p: &Param| {
        matches!(
            p.ty,
            Ty::String | Ty::Id(_) | Ty::Integer | Ty::Number | Ty::Enum(_) | Ty::StringList
        )
    };
    let required = unset(&|p| p.required);
    let target = match required.as_slice() {
        [only] if positional_ty(only) => Some(*only),
        [] => match unset(&|p| matches!(p.ty, Ty::String)).as_slice() {
            [only] => Some(*only),
            // Among several optional strings, a lone `path` still takes a
            // bare argument (`azj layout apply ./proj`).
            several => several.iter().find(|p| p.name == "path").copied(),
        },
        _ => None,
    };
    let Some(p) = target else {
        return Err(format!(
            "unexpected argument `{}` for `{} {}`",
            rest[0], spec.group, spec.action
        ));
    };
    let value = match &p.ty {
        Ty::StringList => Value::Array(rest.into_iter().map(Value::String).collect()),
        ty => {
            if rest.len() > 1 {
                return Err(format!(
                    "unexpected argument `{}`; quote text with spaces",
                    rest[1]
                ));
            }
            convert(&dashed(&p.name), ty, &rest[0])?
        }
    };
    out.insert(p.name.clone(), value);
    Ok(())
}

fn is_id(arg: &str, prefix: &str) -> bool {
    arg.strip_prefix(prefix)
        .and_then(|r| r.strip_prefix('_'))
        .is_some_and(|ulid| ulid.len() == 26 && ulid.chars().all(|c| c.is_ascii_alphanumeric()))
}

fn parse_bool(v: &str) -> Option<bool> {
    match v {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

fn parse_json(name: &str, v: &str) -> Result<Value, String> {
    serde_json::from_str(v).map_err(|e| format!("--{name}: invalid JSON: {e}"))
}

fn convert(name: &str, ty: &Ty, v: &str) -> Result<Value, String> {
    Ok(match ty {
        Ty::String => Value::String(v.to_owned()),
        Ty::Id(prefix) => {
            if !v.starts_with(&format!("{prefix}_")) {
                return Err(format!("--{name}: expected a {prefix}_… ID, got `{v}`"));
            }
            Value::String(v.to_owned())
        }
        Ty::Integer => v
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| format!("--{name}: `{v}` isn't a whole number"))?,
        Ty::Number => v
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .ok_or_else(|| format!("--{name}: `{v}` isn't a number"))?,
        Ty::Bool => Value::Bool(
            parse_bool(v).ok_or_else(|| format!("--{name}: `{v}` isn't true or false"))?,
        ),
        Ty::Enum(values) => {
            let v = v.replace('-', "_");
            if !values.contains(&v) {
                return Err(format!("--{name}: `{v}` isn't one of {}", values.join("|")));
            }
            Value::String(v)
        }
        Ty::StringList => Value::Array(vec![Value::String(v.to_owned())]),
        Ty::Object(_) | Ty::Tagged { .. } | Ty::Json => parse_json(name, v)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::methods;
    use serde_json::json;

    fn parse(method: &str, args: &[&str]) -> Result<Value, String> {
        let spec = methods().into_iter().find(|s| s.method == method).unwrap();
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        parse_params(&spec, &args).map(Value::Object)
    }

    const TILE: &str = "tile_01JBX3K6Z8V9W0Q1R2S3T4V5W6";

    #[test]
    fn browser_split_example() {
        let p = parse(
            "layout.split",
            &[
                "--right",
                "--kind",
                "browser",
                "--url",
                "http://localhost:3000",
            ],
        )
        .unwrap();
        assert_eq!(
            p,
            json!({"direction": "right", "content": {"kind": "browser", "url": "http://localhost:3000"}})
        );
    }

    #[test]
    fn union_shortcuts() {
        let want = json!({"direction": "down", "content": {"kind": "browser", "url": "x"}});
        assert_eq!(
            parse("layout.split", &["--down", "--browser", "--url", "x"]).unwrap(),
            want
        );
        assert_eq!(
            parse("layout.split", &["--down", "--url", "x"]).unwrap(),
            want
        );
        assert_eq!(
            parse(
                "layout.split",
                &[
                    "--direction=down",
                    "--content",
                    r#"{"kind":"browser","url":"x"}"#
                ]
            )
            .unwrap(),
            want
        );
        assert_eq!(
            parse(
                "layout.split",
                &[
                    "--right",
                    "--cwd",
                    "/tmp",
                    "--command",
                    "htop",
                    "--command",
                    "-d"
                ]
            )
            .unwrap(),
            json!({"direction": "right", "content": {"kind": "terminal", "cwd": "/tmp", "command": ["htop", "-d"]}})
        );
        let err = parse(
            "layout.split",
            &["--right", "--kind", "terminal", "--url", "x"],
        )
        .unwrap_err();
        assert!(err.contains("doesn't apply"), "{err}");
    }

    #[test]
    fn positionals() {
        assert_eq!(parse("tile.get", &[TILE]).unwrap(), json!({"tile": TILE}));
        assert_eq!(
            parse("terminal.send_text", &["ls -la\n"]).unwrap(),
            json!({"text": "ls -la\n"})
        );
        assert_eq!(
            parse("terminal.send_keys", &[TILE, "ctrl+c", "enter"]).unwrap(),
            json!({"tile": TILE, "keys": ["ctrl+c", "enter"]})
        );
        assert_eq!(parse("tab.move", &["2"]).unwrap(), json!({"index": 2}));
        assert_eq!(
            parse("workspace.move", &["0"]).unwrap(),
            json!({"index": 0})
        );
        assert_eq!(
            parse("tab.rename", &["build"]).unwrap(),
            json!({"name": "build"})
        );
        assert!(parse("core.ping", &["extra"]).is_err());
    }

    #[test]
    fn struct_fields_and_bools() {
        assert_eq!(
            parse(
                "terminal.bind",
                &[
                    "--tile",
                    TILE,
                    "--pane-id",
                    "w1:p1",
                    "--terminal-id",
                    "term_1",
                    "--tab-id",
                    "w1:t1"
                ]
            )
            .unwrap(),
            json!({"tile": TILE, "herdr": {"pane_id": "w1:p1", "terminal_id": "term_1", "tab_id": "w1:t1"}})
        );
        let err = parse("terminal.bind", &["--tile", TILE, "--pane-id", "w1:p1"]).unwrap_err();
        assert!(err.contains("--herdr also needs --"), "{err}");
        assert_eq!(
            parse("tile.close", &["--kill"]).unwrap(),
            json!({"kill": true})
        );
        assert_eq!(
            parse("tile.close", &["--no-kill"]).unwrap(),
            json!({"kill": false})
        );
        assert_eq!(
            parse("layout.zoom", &["--zoomed", "false"]).unwrap(),
            json!({"zoomed": false})
        );
        assert_eq!(
            parse("layout.resize", &["--left", "--amount", "-0.1"]).unwrap(),
            json!({"direction": "left", "amount": -0.1})
        );
        assert_eq!(
            parse("terminal.read", &["--source", "recent-unwrapped"]).unwrap(),
            json!({"source": "recent_unwrapped"})
        );
    }

    #[test]
    fn helpful_errors() {
        let err = parse("layout.split", &[]).unwrap_err();
        assert!(err.contains("--direction"), "{err}");
        let err = parse("layout.split", &["--sideways"]).unwrap_err();
        assert!(err.contains("unknown option"), "{err}");
        let err = parse("tile.get", &["--tile", "tab_01JBX3K6Z8V9W0Q1R2S3T4V5W6"]).unwrap_err();
        assert!(err.contains("tile_"), "{err}");
    }
}
