//! `azj`: the command-line client for the Azulejo socket API.
//!
//! Every API method is a subcommand, `azj <group> <action>`, generated from the
//! protocol's JSON Schema (see [`spec`]). Run inside an Azulejo terminal tile,
//! commands act on that tile, its tab and its workspace unless told otherwise;
//! `--no-caller` makes them act on whatever is focused instead.

pub mod args;
pub mod output;
pub mod spec;

use std::io::Write;
use std::path::PathBuf;

use azulejo_api::{Client, ClientError};
use azulejo_protocol::ErrorCode;
use serde_json::{Map, Value};

use crate::spec::{MethodSpec, Ty};

/// Exit codes.
pub mod exit {
    pub const OK: i32 = 0;
    /// The core refused the command.
    pub const API_ERROR: i32 = 1;
    /// Bad arguments.
    pub const USAGE: i32 = 2;
    /// Azulejo isn't running or the socket is unreachable.
    pub const UNREACHABLE: i32 = 3;
}

/// What `azj` reads from its environment.
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub socket: PathBuf,
    /// `AZULEJO_TILE_ID`: the terminal tile `azj` runs in.
    pub tile: Option<String>,
    /// `AZULEJO_WORKSPACE_ID`.
    pub workspace: Option<String>,
    /// `HERDR_PANE_ID`, the fallback once herdr has restarted and dropped
    /// Azulejo's variables.
    pub herdr_pane: Option<String>,
    /// `HERDR_WORKSPACE_ID`, to tell apart equal pane IDs in different herdr
    /// workspaces.
    pub herdr_workspace: Option<String>,
    /// `HERDR_SOCKET_PATH`: which herdr session the pane belongs to. Compared
    /// with the core's herdr session (`core.version`), so a pane ID from
    /// another session is never mistaken for one of ours.
    pub herdr_socket: Option<String>,
}

impl Env {
    pub fn from_process() -> Env {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        Env {
            socket: azulejo_api::socket_path(),
            tile: var("AZULEJO_TILE_ID"),
            workspace: var("AZULEJO_WORKSPACE_ID"),
            herdr_pane: var("HERDR_PANE_ID"),
            herdr_workspace: var("HERDR_WORKSPACE_ID"),
            herdr_socket: var("HERDR_SOCKET_PATH"),
        }
    }
}

/// Global options, accepted anywhere before `--`.
#[derive(Debug, Default)]
struct Globals {
    json: bool,
    help: bool,
    no_caller: bool,
    socket: Option<PathBuf>,
}

fn split_globals(args: Vec<String>) -> Result<(Globals, Vec<String>), String> {
    let mut g = Globals::default();
    let mut rest = Vec::new();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--" => {
                rest.push(arg);
                rest.extend(iter.by_ref());
            }
            "--json" => g.json = true,
            "-h" | "--help" => g.help = true,
            "--no-caller" => g.no_caller = true,
            "--socket" => {
                g.socket = Some(iter.next().ok_or("--socket needs a path")?.into());
            }
            _ => match arg.strip_prefix("--socket=") {
                Some(path) => g.socket = Some(path.into()),
                None => rest.push(arg),
            },
        }
    }
    Ok((g, rest))
}

/// Subcommands that are another method under a friendlier name.
const ALIASES: &[(&str, &str, &str)] = &[
    ("tile", "split", "layout.split"),
    ("tile", "swap", "layout.swap"),
    ("tile", "move", "layout.move"),
    ("tile", "resize", "layout.resize"),
    ("tile", "zoom", "layout.zoom"),
];

/// Top-level shortcuts: `azj ping`, `azj version`.
const SHORTCUTS: &[(&str, &str)] = &[("ping", "core.ping"), ("version", "core.version")];

/// Runs `azj` with `args` (without the program name). Returns the exit code.
pub fn run(args: Vec<String>, env: &Env, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let (globals, mut args) = match split_globals(args) {
        Ok(v) => v,
        Err(e) => return usage_error(err, &e),
    };
    let specs = spec::methods();
    let socket = globals.socket.clone().unwrap_or_else(|| env.socket.clone());

    let Some(first) = args.first().cloned() else {
        let _ = write!(out, "{}", help_overview(&specs));
        return if globals.help { exit::OK } else { exit::USAGE };
    };
    match first.as_str() {
        "help" => {
            args.remove(0);
            return help_for(&specs, &args, out, err);
        }
        "methods" => {
            for s in &specs {
                let _ = writeln!(out, "{}", s.method);
            }
            return exit::OK;
        }
        "schema" => {
            let _ = writeln!(out, "{}", azulejo_protocol::api_schema_text().trim_end());
            return exit::OK;
        }
        "call" => return raw_call(&args[1..], &socket, &globals, out, err),
        _ => {}
    }

    let (method, rest) = if let Some((_, m)) = SHORTCUTS.iter().find(|(s, _)| *s == first) {
        (m.to_string(), args[1..].to_vec())
    } else {
        if args.len() < 2 || args[1].starts_with('-') {
            if globals.help || args.len() == 1 {
                return help_for(&specs, &args[..1], out, err);
            }
            return usage_error(
                err,
                &format!("`azj {first}` needs an action; try `azj help {first}`"),
            );
        }
        let action = args[1].replace('_', "-");
        let method = ALIASES
            .iter()
            .find(|(g, a, _)| *g == first && *a == action)
            .map(|(_, _, m)| m.to_string())
            .unwrap_or_else(|| format!("{first}.{}", action.replace('-', "_")));
        (method, args[2..].to_vec())
    };
    let Some(spec) = specs.iter().find(|s| s.method == method) else {
        return usage_error(
            err,
            &format!(
                "unknown command `{}`; run `azj help` for the list",
                args[..2.min(args.len())].join(" ")
            ),
        );
    };
    if globals.help {
        let _ = write!(out, "{}", help_method(spec));
        return exit::OK;
    }

    let mut params = match args::parse_params(spec, &rest) {
        Ok(p) => p,
        Err(e) => return usage_error(err, &e),
    };
    // The core runs elsewhere (its own working directory), so relative paths
    // only mean something here.
    if let Ok(cwd) = std::env::current_dir() {
        absolutise_paths(
            &mut params,
            &cwd,
            std::env::var_os("HOME").map(PathBuf::from),
        );
    }

    let mut client = match Client::connect(&socket) {
        Ok(c) => c,
        Err(e) => return unreachable(err, &e),
    };

    let mut filled = Vec::new();
    if !globals.no_caller {
        match fill_caller(spec, &mut params, env, &mut client) {
            Ok(f) => filled = f,
            Err(e) => {
                let _ = writeln!(err, "azj: {e}");
                return exit::API_ERROR;
            }
        }
    }

    let params = (!params.is_empty()).then_some(Value::Object(params));
    match client.call(&spec.method, params) {
        Ok(result) => {
            print_result(&result, &globals, out);
            exit::OK
        }
        Err(ClientError::Rpc(e)) => {
            let _ = writeln!(err, "azj: {e}");
            if e.kind == Some(ErrorCode::NotFound) && !filled.is_empty() {
                let _ = writeln!(
                    err,
                    "azj: {} came from this terminal's environment; pass --no-caller to act on the focused one",
                    filled.join(", ")
                );
            }
            exit::API_ERROR
        }
        Err(e) => unreachable(err, &e),
    }
}

/// Params that name files or folders: top-level `path`, `cwd` and `root`, and
/// the same inside a tile's `content`.
const PATH_PARAMS: [&str; 3] = ["path", "cwd", "root"];

/// Makes relative path params absolute against `cwd`, and expands a leading
/// `~/` (or a bare `~`) against `home`.
pub fn absolutise_paths(
    params: &mut Map<String, Value>,
    cwd: &std::path::Path,
    home: Option<PathBuf>,
) {
    let fix = |value: &mut Value| {
        let Value::String(text) = value else { return };
        if text.is_empty() {
            return;
        }
        let path = if text == "~" || text.starts_with("~/") {
            let Some(home) = &home else { return };
            let rest = text.trim_start_matches('~').trim_start_matches('/');
            if rest.is_empty() {
                home.clone()
            } else {
                home.join(rest)
            }
        } else if std::path::Path::new(text.as_str()).is_absolute() {
            return;
        } else {
            cwd.join(text.as_str())
        };
        *text = path.to_string_lossy().into_owned();
    };
    for key in PATH_PARAMS {
        if let Some(value) = params.get_mut(key) {
            fix(value);
        }
    }
    if let Some(Value::Object(content)) = params.get_mut("content") {
        for key in PATH_PARAMS {
            if let Some(value) = content.get_mut(key) {
                fix(value);
            }
        }
    }
}

fn print_result(result: &Value, globals: &Globals, out: &mut dyn Write) {
    if globals.json {
        let _ = writeln!(out, "{result}");
    } else {
        let _ = write!(out, "{}", output::human(result));
    }
}

fn usage_error(err: &mut dyn Write, msg: &str) -> i32 {
    let _ = writeln!(err, "azj: {msg}");
    exit::USAGE
}

fn unreachable(err: &mut dyn Write, e: &ClientError) -> i32 {
    let _ = writeln!(err, "azj: {e}");
    if matches!(e, ClientError::NotRunning { .. }) {
        let _ = writeln!(
            err,
            "azj: start Azulejo, or point AZULEJO_SOCKET (or --socket) at its socket"
        );
    }
    exit::UNREACHABLE
}

/// `azj call <method> [json-params]`: the escape hatch.
fn raw_call(
    args: &[String],
    socket: &PathBuf,
    globals: &Globals,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let Some(method) = args.first() else {
        return usage_error(err, "usage: azj call <method> [json-params]");
    };
    let params = match args.get(1) {
        Some(text) => match serde_json::from_str::<Value>(text) {
            Ok(v) => Some(v),
            Err(e) => return usage_error(err, &format!("params aren't valid JSON: {e}")),
        },
        None => None,
    };
    let mut client = match Client::connect(socket) {
        Ok(c) => c,
        Err(e) => return unreachable(err, &e),
    };
    match client.call(method, params) {
        Ok(result) => {
            print_result(&result, globals, out);
            exit::OK
        }
        Err(ClientError::Rpc(e)) => {
            let _ = writeln!(err, "azj: {e}");
            exit::API_ERROR
        }
        Err(e) => unreachable(err, &e),
    }
}

/// Where the calling terminal lives, looked up lazily.
#[derive(Default)]
struct CallerInfo {
    window: Option<String>,
    workspace: Option<String>,
    tab: Option<String>,
    tile: Option<String>,
}

/// Fills unset `tile`, `tab`, `workspace` and `window` params with the calling
/// terminal's. Returns the names of the params it filled.
fn fill_caller(
    spec: &MethodSpec,
    params: &mut Map<String, Value>,
    env: &Env,
    client: &mut Client,
) -> Result<Vec<String>, String> {
    // Which params to fill. `tile.list`'s workspace is a filter, not a target.
    let wants: Vec<&str> = spec
        .params
        .iter()
        .filter(|p| !p.required && !params.contains_key(&p.name))
        .filter(|p| matches!(&p.ty, Ty::Id(prefix) if crate::caller_prefix(&p.name) == Some(prefix.as_str())))
        .filter(|p| !(spec.method == "tile.list" && p.name == "workspace"))
        .map(|p| p.name.as_str())
        .collect();
    if wants.is_empty() || (env.tile.is_none() && env.herdr_pane.is_none()) {
        return Ok(Vec::new());
    }

    let mut info = CallerInfo {
        tile: env.tile.clone(),
        workspace: env.workspace.clone(),
        ..CallerInfo::default()
    };
    let needs_lookup = |info: &CallerInfo| {
        wants.iter().any(|w| match *w {
            "tile" => info.tile.is_none(),
            "tab" => info.tab.is_none(),
            "workspace" => info.workspace.is_none(),
            "window" => info.window.is_none(),
            _ => false,
        })
    };
    if needs_lookup(&info) {
        if let Some(tile) = &info.tile {
            let found = client
                .call("tile.get", Some(serde_json::json!({ "tile": tile })))
                .map_err(|e| format!("AZULEJO_TILE_ID={tile} can't be looked up ({e}); pass --no-caller to act on the focused tile"))?;
            let get = |k: &str| found.get(k).and_then(Value::as_str).map(str::to_owned);
            info.tab = get("tab");
            info.window = get("window");
            info.workspace = info.workspace.or_else(|| get("workspace"));
        } else if let Some(pane) = &env.herdr_pane
            && same_herdr_session(env, client)
        {
            let session = client
                .call("session.get", None)
                .map_err(|e| format!("can't look up herdr pane {pane}: {e}"))?;
            if let Some(found) =
                find_herdr_pane(&session["session"], pane, env.herdr_workspace.as_deref())
            {
                info = found;
            }
        }
    }

    let mut filled = Vec::new();
    for want in wants {
        let value = match want {
            "tile" => info.tile.clone(),
            "tab" => info.tab.clone(),
            "workspace" => info.workspace.clone(),
            "window" => info.window.clone(),
            _ => None,
        };
        if let Some(v) = value {
            params.insert(want.to_owned(), Value::String(v));
            filled.push(format!("--{want}"));
        }
    }
    Ok(filled)
}

/// Whether `azj` runs in a pane of the core's herdr session. Unknown on either
/// side counts as the same, so older cores and plain shells keep working.
fn same_herdr_session(env: &Env, client: &mut Client) -> bool {
    let Some(ours) = &env.herdr_socket else {
        return true;
    };
    let Ok(version) = client.call("core.version", None) else {
        return true;
    };
    let Some(theirs) = version.get("herdr_socket").and_then(Value::as_str) else {
        return true;
    };
    let canonical = |p: &str| std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p));
    canonical(ours) == canonical(theirs)
}

/// The param names `fill_caller` handles, with the ID kind each takes.
fn caller_prefix(name: &str) -> Option<&'static str> {
    match name {
        "tile" => Some("tile"),
        "tab" => Some("tab"),
        "workspace" => Some("ws"),
        "window" => Some("win"),
        _ => None,
    }
}

/// Finds the terminal tile viewing herdr pane `pane`. When the workspace has a
/// herdr binding and `herdr_workspace` is known, both must match, so a pane ID
/// from an unrelated herdr session is less likely to be mistaken for ours.
fn find_herdr_pane(
    session: &Value,
    pane: &str,
    herdr_workspace: Option<&str>,
) -> Option<CallerInfo> {
    for window in session["windows"].as_array()? {
        for ws in window["workspaces"].as_array().into_iter().flatten() {
            if let (Some(want), Some(bound)) = (herdr_workspace, ws["herdr_workspace"].as_str())
                && want != bound
            {
                continue;
            }
            for tab in ws["tabs"].as_array().into_iter().flatten() {
                for tile in tab["tiles"].as_array().into_iter().flatten() {
                    if tile["herdr"]["pane_id"].as_str() == Some(pane) {
                        let s = |v: &Value| v.as_str().map(str::to_owned);
                        return Some(CallerInfo {
                            window: s(&window["id"]),
                            workspace: s(&ws["id"]),
                            tab: s(&tab["id"]),
                            tile: s(&tile["id"]),
                        });
                    }
                }
            }
        }
    }
    None
}

fn help_overview(specs: &[MethodSpec]) -> String {
    let mut s = String::from(
        "azj: control Azulejo from the command line\n\n\
         usage: azj <group> <action> [options]\n\
         \x20      azj help <group> [<action>]\n\
         \x20      azj call <method> [json-params]\n\
         \x20      azj methods | schema | ping | version\n\n\
         global options:\n\
         \x20 --json          print the raw JSON result\n\
         \x20 --no-caller     act on the focused tile, not the terminal azj runs in\n\
         \x20 --socket PATH   socket to use (default: $AZULEJO_SOCKET, else the app's)\n\n\
         commands:\n",
    );
    for spec in specs {
        let mut line = format!("  {} {}", spec.group, spec.action);
        if let Some(d) = &spec.description {
            while line.len() < 28 {
                line.push(' ');
            }
            line.push_str(first_sentence(d));
        }
        s.push_str(&line);
        s.push('\n');
    }
    s.push_str("\naliases: azj tile split|swap|move|resize|zoom = azj layout …\n");
    s
}

fn first_sentence(d: &str) -> &str {
    d.split_once(". ")
        .map(|(a, _)| a)
        .unwrap_or(d)
        .trim_end_matches('.')
}

fn help_for(
    specs: &[MethodSpec],
    args: &[String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    match args {
        [] => {
            let _ = write!(out, "{}", help_overview(specs));
            exit::OK
        }
        [group] => {
            let matching: Vec<&MethodSpec> = specs.iter().filter(|s| s.group == *group).collect();
            if matching.is_empty() {
                return usage_error(err, &format!("no command group `{group}`"));
            }
            for spec in matching {
                let _ = writeln!(out, "{}", help_method(spec));
            }
            exit::OK
        }
        [group, action, ..] => {
            let action = action.replace('_', "-");
            let method = ALIASES
                .iter()
                .find(|(g, a, _)| g == group && *a == action)
                .map(|(_, _, m)| m.to_string())
                .unwrap_or_else(|| format!("{group}.{}", action.replace('-', "_")));
            match specs.iter().find(|s| s.method == method) {
                Some(spec) => {
                    let _ = write!(out, "{}", help_method(spec));
                    exit::OK
                }
                None => usage_error(err, &format!("no command `{group} {action}`")),
            }
        }
    }
}

fn help_method(spec: &MethodSpec) -> String {
    let mut s = format!("azj {} {}    ({})\n", spec.group, spec.action, spec.method);
    if let Some(d) = &spec.description {
        s.push_str(&format!("  {d}\n"));
    }
    for p in &spec.params {
        let req = if p.required { " (required)" } else { "" };
        s.push_str(&format!(
            "  --{} <{}>{req}",
            args::dashed(&p.name),
            p.ty.describe()
        ));
        if let Some(d) = &p.description {
            s.push_str(&format!("  {d}"));
        }
        s.push('\n');
        match &p.ty {
            Ty::Tagged { tag, variants } => {
                for (name, fields) in variants {
                    let flags: Vec<String> = fields
                        .iter()
                        .map(|f| format!("--{} <{}>", args::dashed(&f.name), f.ty.describe()))
                        .collect();
                    s.push_str(&format!(
                        "      --{} {name}  {}\n",
                        args::dashed(tag),
                        flags.join(" ")
                    ));
                }
            }
            Ty::Object(fields) => {
                for f in fields {
                    s.push_str(&format!(
                        "      --{} <{}>\n",
                        args::dashed(&f.name),
                        f.ty.describe()
                    ));
                }
            }
            _ => {}
        }
    }
    s
}

#[cfg(test)]
mod path_tests {
    use super::*;
    use serde_json::json;

    fn fixed(params: Value) -> Value {
        let Value::Object(mut map) = params else {
            panic!()
        };
        absolutise_paths(
            &mut map,
            std::path::Path::new("/work/proj"),
            Some(PathBuf::from("/home/me")),
        );
        Value::Object(map)
    }

    #[test]
    fn relative_paths_become_absolute() {
        assert_eq!(
            fixed(json!({"path": "docs/plan.md", "tile": "tile_x"})),
            json!({"path": "/work/proj/docs/plan.md", "tile": "tile_x"})
        );
        assert_eq!(
            fixed(json!({"content": {"kind": "preview", "path": "a.md"}, "cwd": "."})),
            json!({"content": {"kind": "preview", "path": "/work/proj/a.md"}, "cwd": "/work/proj/."})
        );
        assert_eq!(
            fixed(json!({"content": {"kind": "files", "root": "src"}})),
            json!({"content": {"kind": "files", "root": "/work/proj/src"}})
        );
    }

    #[test]
    fn absolute_home_and_other_params_are_left_alone_or_expanded() {
        assert_eq!(
            fixed(json!({"path": "/etc/hosts"})),
            json!({"path": "/etc/hosts"})
        );
        assert_eq!(
            fixed(json!({"path": "~/notes.md"})),
            json!({"path": "/home/me/notes.md"})
        );
        assert_eq!(fixed(json!({"path": "~"})), json!({"path": "/home/me"}));
        assert_eq!(
            fixed(json!({"url": "docs/x", "path": ""})),
            json!({"url": "docs/x", "path": ""})
        );
    }
}
