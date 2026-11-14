//! `azj`: the command-line client for the Azulejo socket API.
//!
//! Every API method is a subcommand, `azj <group> <action>`, generated from the
//! protocol's JSON Schema (see [`spec`]). Run inside an Azulejo terminal tile,
//! commands act on that tile, its tab and its workspace unless told otherwise;
//! `--no-caller` makes them act on whatever is focused instead.
//!
//! `--json` prints results as raw JSON on stdout. Errors go there too, as one
//! object shaped like the API's errors: `{"code": "not_found", "message": …,
//! "data": {…}}`, where `data` is `{}` when there's nothing to add. Without
//! `--json`, errors are `azj: …` lines on stderr. Either way the exit code
//! says what kind of failure it was (see [`exit`]).

pub mod args;
pub mod output;
pub mod spec;

use std::io::Write;
use std::path::{Path, PathBuf};

use azulejo_api::{Client, ClientError, RpcError};
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

/// Splits the global options off `args`. The globals come back even when the
/// rest is an error, so it can still be reported as JSON.
fn split_globals(args: Vec<String>) -> (Globals, Result<Vec<String>, Failure>) {
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
            "--socket" => match iter.next() {
                Some(path) => g.socket = Some(path.into()),
                None => {
                    let e = Failure::usage(ErrorCode::InvalidParams, "--socket needs a path")
                        .about("param", "socket");
                    return (g, Err(e));
                }
            },
            _ => match arg.strip_prefix("--socket=") {
                Some(path) => g.socket = Some(path.into()),
                None => rest.push(arg),
            },
        }
    }
    (g, Ok(rest))
}

/// Why `azj` failed: an error in the API's shape (`code`, `message`, `data`),
/// its exit code, and the lines a person sees.
#[derive(Debug)]
struct Failure {
    code: ErrorCode,
    message: String,
    data: Map<String, Value>,
    exit: i32,
    /// Printed as `azj: <line>` on stderr without `--json`.
    lines: Vec<String>,
}

impl Failure {
    /// Bad arguments, caught by `azj` itself.
    fn usage(code: ErrorCode, message: impl Into<String>) -> Failure {
        let message = message.into();
        Failure {
            code,
            lines: vec![message.clone()],
            message,
            data: Map::new(),
            exit: exit::USAGE,
        }
    }

    /// The core refused the command.
    fn rpc(e: &RpcError) -> Failure {
        Failure {
            code: e.kind.unwrap_or_else(|| code_for_rpc(e.code)),
            message: e.message.clone(),
            data: e.data.clone(),
            exit: exit::API_ERROR,
            lines: vec![e.to_string()],
        }
    }

    /// Any client error: the core's refusal, or no core at `socket`.
    fn client(e: &ClientError, socket: &Path) -> Failure {
        if let ClientError::Rpc(e) = e {
            return Failure::rpc(e);
        }
        let mut lines = vec![e.to_string()];
        if matches!(e, ClientError::NotRunning { .. }) {
            lines.push(
                "start Azulejo, or point AZULEJO_SOCKET (or --socket) at its socket".to_owned(),
            );
        }
        let code = match e {
            ClientError::Protocol(_) => ErrorCode::Internal,
            _ => ErrorCode::Unavailable,
        };
        Failure {
            code,
            message: e.to_string(),
            data: Map::new(),
            exit: exit::UNREACHABLE,
            lines,
        }
        .about("socket", socket.display().to_string())
    }

    /// Says it in other words, for people and programs alike.
    fn retold(mut self, message: String) -> Failure {
        self.lines = vec![message.clone()];
        self.message = message;
        self
    }

    /// Adds `key` to what the error is about.
    fn about(mut self, key: &str, value: impl Into<Value>) -> Failure {
        self.data.insert(key.to_owned(), value.into());
        self
    }

    /// Prints the failure and returns its exit code. With `--json` it's an
    /// error object on stdout, where results go; otherwise text on stderr.
    fn report(self, json: bool, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
        if json {
            let object = serde_json::json!({
                "code": self.code,
                "message": self.message,
                "data": self.data,
            });
            let _ = writeln!(out, "{object}");
        } else {
            for line in &self.lines {
                let _ = writeln!(err, "azj: {line}");
            }
        }
        self.exit
    }
}

/// The protocol error code for JSON-RPC code `code`, for servers that don't
/// send one.
fn code_for_rpc(code: i64) -> ErrorCode {
    use ErrorCode::*;
    [
        ParseError,
        InvalidRequest,
        MethodNotFound,
        InvalidParams,
        NotFound,
        InvalidState,
        Unsupported,
        Unavailable,
    ]
    .into_iter()
    .find(|c| i64::from(c.rpc_code()) == code)
    .unwrap_or(Internal)
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
    let (globals, args) = split_globals(args);
    match args.and_then(|args| dispatch(args, &globals, env, out)) {
        Ok(code) => code,
        Err(e) => e.report(globals.json, out, err),
    }
}

fn dispatch(
    mut args: Vec<String>,
    globals: &Globals,
    env: &Env,
    out: &mut dyn Write,
) -> Result<i32, Failure> {
    let specs = spec::methods();
    let socket = globals.socket.clone().unwrap_or_else(|| env.socket.clone());

    let Some(first) = args.first().cloned() else {
        if globals.json && !globals.help {
            return Err(Failure::usage(
                ErrorCode::InvalidRequest,
                "no command; run `azj help` for the list",
            ));
        }
        let _ = write!(out, "{}", help_overview(&specs));
        return Ok(if globals.help { exit::OK } else { exit::USAGE });
    };
    match first.as_str() {
        "help" => {
            args.remove(0);
            return help_for(&specs, &args, out);
        }
        "methods" => {
            for s in &specs {
                let _ = writeln!(out, "{}", s.method);
            }
            return Ok(exit::OK);
        }
        "schema" => {
            let _ = writeln!(out, "{}", azulejo_protocol::api_schema_text().trim_end());
            return Ok(exit::OK);
        }
        "call" => return raw_call(&args[1..], &socket, globals, out),
        // `azj events [--follow]` and `azj events subscribe` stream events.
        "events"
            if !globals.help
                && args
                    .get(1)
                    .is_none_or(|a| a.starts_with('-') || a == "subscribe" || a == "follow") =>
        {
            let skip = if args.get(1).is_some_and(|a| !a.starts_with('-')) {
                2
            } else {
                1
            };
            return follow_events(&args[skip..], &specs, &socket, globals, out);
        }
        _ => {}
    }

    let (method, rest) = if let Some((_, m)) = SHORTCUTS.iter().find(|(s, _)| *s == first) {
        (m.to_string(), args[1..].to_vec())
    } else {
        if args.len() < 2 || args[1].starts_with('-') {
            if globals.help || args.len() == 1 {
                return help_for(&specs, &args[..1], out);
            }
            return Err(Failure::usage(
                ErrorCode::InvalidRequest,
                format!("`azj {first}` needs an action; try `azj help {first}`"),
            ));
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
        return Err(Failure::usage(
            ErrorCode::MethodNotFound,
            format!(
                "unknown command `{}`; run `azj help` for the list",
                args[..2.min(args.len())].join(" ")
            ),
        )
        .about("method", method));
    };
    if globals.help {
        let _ = write!(out, "{}", help_method(spec));
        return Ok(exit::OK);
    }

    let mut params =
        args::parse_params(spec, &rest).map_err(|e| Failure::usage(ErrorCode::InvalidParams, e))?;
    // The core runs elsewhere (its own working directory), so relative paths
    // only mean something here, except those relative to a tile's root.
    if let Ok(cwd) = std::env::current_dir() {
        absolutise_paths(
            spec,
            &mut params,
            &cwd,
            std::env::var_os("HOME").map(PathBuf::from),
        );
    }

    let mut client = Client::connect(&socket).map_err(|e| Failure::client(&e, &socket))?;

    let mut filled = Vec::new();
    if !globals.no_caller {
        filled = fill_caller(spec, &mut params, env, &mut client, &socket)?;
    }

    let params = (!params.is_empty()).then_some(Value::Object(params));
    match client.call(&spec.method, params) {
        Ok(result) => {
            print_result(&result, globals, out);
            Ok(exit::OK)
        }
        Err(e) => {
            let mut failure = Failure::client(&e, &socket);
            if failure.code == ErrorCode::NotFound && !filled.is_empty() {
                failure.lines.push(format!(
                    "{} came from this terminal's environment; pass --no-caller to act on the focused one",
                    filled.join(", ")
                ));
            }
            Err(failure)
        }
    }
}

/// Params that name files or folders: top-level `path`, `cwd`, `root` and
/// `base_dir`, and the same inside a tile's `content`.
const PATH_PARAMS: [&str; 4] = ["path", "cwd", "root", "base_dir"];

/// What the schema says about paths the core resolves against a tile's root
/// (`files.open`, `files.reveal`). `azj` leaves those as typed.
const ROOT_RELATIVE: &str = "relative to the tile's root";

/// Whether `spec`'s param `name` is a path the core resolves against a tile's
/// root, going by its schema description.
pub fn is_root_relative(spec: &MethodSpec, name: &str) -> bool {
    spec.params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| p.description.as_deref())
        .is_some_and(|d| d.contains(ROOT_RELATIVE))
}

/// Makes `spec`'s relative path params absolute against `cwd`, and expands a
/// leading `~/` (or a bare `~`) against `home`. Paths the schema calls
/// relative to a tile's root stay as they are: the core resolves them.
pub fn absolutise_paths(
    spec: &MethodSpec,
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
            PathBuf::from(text.as_str())
        } else {
            cwd.join(text.as_str())
        };
        // Tidy doubled slashes (`$TMPDIR/x`) and `.` parts; `..` stays, as
        // folding it could cross a symlink.
        let path: PathBuf = path.components().collect();
        *text = path.to_string_lossy().into_owned();
    };
    for key in PATH_PARAMS {
        if is_root_relative(spec, key) {
            continue;
        }
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

/// `azj events [--follow] [--filter NAME]… [--workspace ID] [--tile ID]
/// [--count N] [--json]`: prints events as they happen, one per line, until
/// interrupted (or until `--count` events). `--follow` is the default and only
/// mode; it's accepted for familiarity.
fn follow_events(
    args: &[String],
    specs: &[MethodSpec],
    socket: &Path,
    globals: &Globals,
    out: &mut dyn Write,
) -> Result<i32, Failure> {
    let bad_count = || {
        Failure::usage(ErrorCode::InvalidParams, "--count needs a number").about("param", "count")
    };
    let mut count: Option<u64> = None;
    let mut rest = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--follow" | "-f" => {}
            "--count" | "-n" => match iter.next().and_then(|n| n.parse().ok()) {
                Some(n) => count = Some(n),
                None => return Err(bad_count()),
            },
            other => match other.strip_prefix("--count=").map(str::parse) {
                Some(Ok(n)) => count = Some(n),
                Some(Err(_)) => return Err(bad_count()),
                None => rest.push(arg.clone()),
            },
        }
    }
    let Some(spec) = specs.iter().find(|s| s.method == "events.subscribe") else {
        return Err(Failure::usage(
            ErrorCode::MethodNotFound,
            "this azj doesn't know `events.subscribe`",
        ));
    };
    let params =
        args::parse_params(spec, &rest).map_err(|e| Failure::usage(ErrorCode::InvalidParams, e))?;
    let failed = |e: ClientError| Failure::client(&e, socket);
    let mut client = Client::connect(socket).map_err(failed)?;
    let params = (!params.is_empty()).then_some(Value::Object(params));
    let subscription = client.subscribe(params).map_err(failed)?;
    let mut seen = 0;
    while count.is_none_or(|n| seen < n) {
        let Some(message) = client.next_notification(None).map_err(failed)? else {
            continue;
        };
        let params = &message["params"];
        if params["subscription"] != subscription {
            continue;
        }
        let line = if globals.json {
            message.to_string()
        } else {
            output::event_line(&message)
        };
        if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
            // The reader went away (e.g. `| head`).
            return Ok(exit::OK);
        }
        if message["method"] == "events.event" {
            seen += 1;
        }
    }
    Ok(exit::OK)
}

/// `azj call <method> [json-params]`: the escape hatch.
fn raw_call(
    args: &[String],
    socket: &Path,
    globals: &Globals,
    out: &mut dyn Write,
) -> Result<i32, Failure> {
    let Some(method) = args.first() else {
        return Err(Failure::usage(
            ErrorCode::InvalidRequest,
            "usage: azj call <method> [json-params]",
        ));
    };
    let params = match args.get(1) {
        Some(text) => Some(serde_json::from_str::<Value>(text).map_err(|e| {
            Failure::usage(
                ErrorCode::InvalidParams,
                format!("params aren't valid JSON: {e}"),
            )
        })?),
        None => None,
    };
    let failed = |e: ClientError| Failure::client(&e, socket);
    let mut client = Client::connect(socket).map_err(failed)?;
    let result = client.call(method, params).map_err(failed)?;
    print_result(&result, globals, out);
    Ok(exit::OK)
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
    socket: &Path,
) -> Result<Vec<String>, Failure> {
    // Which params to fill. `tile.list`'s workspace is a filter, not a target.
    let wants: Vec<&str> = spec
        .params
        .iter()
        .filter(|p| !p.required && !params.contains_key(&p.name))
        .filter(|p| matches!(&p.ty, Ty::Id(prefix) if crate::caller_prefix(&p.name) == Some(prefix.as_str())))
        .filter(|p| !(spec.method == "tile.list" && p.name == "workspace"))
        // A stream's workspace and tile are filters too.
        .filter(|_| spec.method != "events.subscribe")
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
                .map_err(|e| Failure::client(&e, socket).retold(format!("AZULEJO_TILE_ID={tile} can't be looked up ({e}); pass --no-caller to act on the focused tile")))?;
            let get = |k: &str| found.get(k).and_then(Value::as_str).map(str::to_owned);
            info.tab = get("tab");
            info.window = get("window");
            info.workspace = info.workspace.or_else(|| get("workspace"));
        } else if let Some(pane) = &env.herdr_pane
            && same_herdr_session(env, client)
        {
            let session = client.call("session.get", None).map_err(|e| {
                Failure::client(&e, socket).retold(format!("can't look up herdr pane {pane}: {e}"))
            })?;
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
         errors go to stderr as `azj: …` lines. With --json, an error is one\n\
         {\"code\", \"message\", \"data\"} object on stdout, like the API's errors.\n\
         exit codes: 0 ok, 1 the core refused, 2 bad arguments, 3 can't reach Azulejo\n\n\
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

fn help_for(specs: &[MethodSpec], args: &[String], out: &mut dyn Write) -> Result<i32, Failure> {
    let no_command =
        |what: String| Failure::usage(ErrorCode::MethodNotFound, format!("no command {what}"));
    match args {
        [] => {
            let _ = write!(out, "{}", help_overview(specs));
            Ok(exit::OK)
        }
        [group] => {
            let matching: Vec<&MethodSpec> = specs.iter().filter(|s| s.group == *group).collect();
            if matching.is_empty() {
                return Err(no_command(format!("group `{group}`")));
            }
            for spec in matching {
                let _ = writeln!(out, "{}", help_method(spec));
            }
            Ok(exit::OK)
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
                    Ok(exit::OK)
                }
                None => Err(no_command(format!("`{group} {action}`"))),
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

    fn fixed(method: &str, params: Value) -> Value {
        let Value::Object(mut map) = params else {
            panic!()
        };
        let spec = spec::methods()
            .into_iter()
            .find(|s| s.method == method)
            .unwrap();
        absolutise_paths(
            &spec,
            &mut map,
            std::path::Path::new("/work/proj"),
            Some(PathBuf::from("/home/me")),
        );
        Value::Object(map)
    }

    #[test]
    fn local_file_paths_become_absolute() {
        assert_eq!(
            fixed(
                "preview.open",
                json!({"path": "docs/plan.md", "tile": "tile_x"})
            ),
            json!({"path": "/work/proj/docs/plan.md", "tile": "tile_x"})
        );
        assert_eq!(
            fixed(
                "layout.apply",
                json!({"path": "layouts/dev.toml", "base_dir": ".."})
            ),
            json!({"path": "/work/proj/layouts/dev.toml", "base_dir": "/work/proj/.."})
        );
        assert_eq!(
            fixed("layout.export", json!({"path": "./.azulejo.toml"})),
            json!({"path": "/work/proj/.azulejo.toml"})
        );
        assert_eq!(
            fixed(
                "layout.split",
                json!({"content": {"kind": "preview", "path": "a.md"}})
            ),
            json!({"content": {"kind": "preview", "path": "/work/proj/a.md"}})
        );
        assert_eq!(
            fixed("workspace.create", json!({"cwd": "."})),
            json!({"cwd": "/work/proj"})
        );
        assert_eq!(
            fixed(
                "tab.create",
                json!({"content": {"kind": "files", "root": "src"}})
            ),
            json!({"content": {"kind": "files", "root": "/work/proj/src"}})
        );
        assert_eq!(
            fixed("files.set_root", json!({"root": "src"})),
            json!({"root": "/work/proj/src"})
        );
    }

    #[test]
    fn paths_relative_to_a_tile_root_stay_relative() {
        for method in ["files.open", "files.reveal"] {
            assert_eq!(
                fixed(method, json!({"path": "README.md", "tile": "tile_x"})),
                json!({"path": "README.md", "tile": "tile_x"}),
                "{method}"
            );
            // The core expands `~` for these too.
            assert_eq!(
                fixed(method, json!({"path": "~/notes.md"})),
                json!({"path": "~/notes.md"}),
                "{method}"
            );
        }
        // `dir` isn't a path param anywhere.
        assert_eq!(
            fixed("files.expand", json!({"dir": "src"})),
            json!({"dir": "src"})
        );
    }

    #[test]
    fn the_schema_marks_exactly_the_root_relative_paths() {
        let marked: Vec<String> = spec::methods()
            .iter()
            .flat_map(|s| {
                PATH_PARAMS
                    .iter()
                    .filter(|p| is_root_relative(s, p))
                    .map(|p| format!("{}.{p}", s.method))
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(marked, ["files.reveal.path", "files.open.path"]);
    }

    #[test]
    fn absolute_home_and_other_params_are_tidied_or_left_alone() {
        assert_eq!(
            fixed("preview.open", json!({"path": "/etc/hosts"})),
            json!({"path": "/etc/hosts"})
        );
        assert_eq!(
            fixed("workspace.create", json!({"cwd": "/var/T//azj-e2e/./x"})),
            json!({"cwd": "/var/T/azj-e2e/x"})
        );
        assert_eq!(
            fixed("preview.open", json!({"path": "~/notes.md"})),
            json!({"path": "/home/me/notes.md"})
        );
        assert_eq!(
            fixed("preview.open", json!({"path": "~"})),
            json!({"path": "/home/me"})
        );
        assert_eq!(
            fixed("browser.open", json!({"url": "docs/x"})),
            json!({"url": "docs/x"})
        );
        assert_eq!(
            fixed("preview.open", json!({"path": ""})),
            json!({"path": ""})
        );
    }
}
