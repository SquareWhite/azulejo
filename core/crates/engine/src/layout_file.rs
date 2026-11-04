//! Project layout files (F6): `.azulejo.toml` describes tabs, splits and
//! tiles, and `layout.apply` opens them; `layout.export` writes a tab or
//! workspace back out. See `docs/layout-files.md` for the format.
//!
//! Small files can skip `[[tab]]`: top-level `layout`, `tile` and `root` make
//! one tab.
//!
//! ```toml
//! name = "shop"
//! layout = "tile"                      # master left, even stack right
//!
//! [[tile]]
//! command = "nvim ."
//! [[tile]]
//! command = ["cargo", "watch", "-x", "test"]
//! [[tile]]
//! url = "http://localhost:3000"        # kind inferred: browser
//! ```
//!
//! Applying only builds the model; terminal tiles get herdr panes (and their
//! commands typed in) through the usual `tile.created` effects.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use azulejo_model::{
    Axis, LayoutKind, LayoutParams, MAX_RATIO, MIN_RATIO, Node, OnExit, Orientation, Session, Tab,
    TabId, TabLayout, Tile, TileContent, TileId, TileParams, WorkspaceId,
};
use azulejo_protocol::{
    ApplyMode, Created, Error, ErrorCode, Focus, LayoutApply, LayoutExport, Reply,
};
use serde::{Deserialize, Serialize};

/// Most tiles a single tab may have in a layout file.
const MAX_TILES: usize = 64;

// ----- file format -----

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FileSpec {
    /// Format version; 1 when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    version: Option<u32>,
    /// Workspace name for `new_workspace`; the directory's name by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    /// Working directory for terminals, relative to the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
    // One tab without `[[tab]]`: the same keys as a tab, minus its name.
    // (serde's `flatten` can't be combined with `deny_unknown_fields`.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layout: Option<Arrangement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    masters: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    orientation: Option<Orientation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sizes: Option<Vec<f64>>,
    #[serde(default, rename = "tile", skip_serializing_if = "Vec::is_empty")]
    tiles: Vec<NodeSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root: Option<NodeSpec>,
    #[serde(default, rename = "tab", skip_serializing_if = "Vec::is_empty")]
    tabs: Vec<TabSpec>,
}

impl FileSpec {
    /// Moves the top-level tab keys out as a tab (empty if there were none).
    fn take_single(&mut self) -> TabSpec {
        TabSpec {
            name: None,
            layout: self.layout.take(),
            masters: self.masters.take(),
            ratio: self.ratio.take(),
            orientation: self.orientation.take(),
            sizes: self.sizes.take(),
            tiles: std::mem::take(&mut self.tiles),
            root: self.root.take(),
        }
    }

    fn put_single(&mut self, tab: TabSpec) {
        self.layout = tab.layout;
        self.masters = tab.masters;
        self.ratio = tab.ratio;
        self.orientation = tab.orientation;
        self.sizes = tab.sizes;
        self.tiles = tab.tiles;
        self.root = tab.root;
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TabSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    /// How `tile` entries are arranged; `tile` by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layout: Option<Arrangement>,
    /// `tile` / `three_column`: how many tiles share the master area.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    masters: Option<u8>,
    /// `tile`: the master area's share, 0.2 to 0.8; `three_column`: the
    /// centre column's, 0.2 to 0.75.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ratio: Option<f64>,
    /// `tile`: which side the masters sit on (`left`, `top`, `right`,
    /// `bottom`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    orientation: Option<Orientation>,
    /// `columns`/`rows`: relative sizes, one per tile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sizes: Option<Vec<f64>>,
    #[serde(default, rename = "tile", skip_serializing_if = "Vec::is_empty")]
    tiles: Vec<NodeSpec>,
    /// An explicit split tree, instead of `layout` + `tile`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root: Option<NodeSpec>,
}

impl TabSpec {
    fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.layout.is_none()
            && self.masters.is_none()
            && self.ratio.is_none()
            && self.orientation.is_none()
            && self.sizes.is_none()
            && self.tiles.is_empty()
            && self.root.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Arrangement {
    /// Dynamic master-stack (ADR 0010): masters on one side, the stack on the
    /// other.
    Tile,
    /// The same as `tile` with one master; kept for older files.
    Tall,
    /// Dynamic: every tile full size, the focused one shown.
    Monocle,
    /// Dynamic: masters centred, stacks either side.
    ThreeColumn,
    /// Manual: side by side.
    Columns,
    /// Manual: stacked.
    Rows,
}

impl Arrangement {
    /// The dynamic kind this makes, or `None` for a manual arrangement.
    fn kind(self) -> Option<LayoutKind> {
        match self {
            Arrangement::Tile | Arrangement::Tall => Some(LayoutKind::Tile),
            Arrangement::Monocle => Some(LayoutKind::Monocle),
            Arrangement::ThreeColumn => Some(LayoutKind::ThreeColumn),
            Arrangement::Columns | Arrangement::Rows => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum SplitAxis {
    /// Children side by side.
    Horizontal,
    /// Children stacked.
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Terminal,
    Browser,
    Preview,
    Files,
}

/// A split (`split` + `children`) or a tile (everything else).
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct NodeSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    split: Option<SplitAxis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sizes: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    children: Vec<NodeSpec>,

    /// Inferred when absent: `url` → browser, `path` → preview, `root` →
    /// files, else terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<Kind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    focus: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    command: Option<CommandSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    env: BTreeMap<String, String>,
    /// `"close"` or `"keep"`: overrides `[terminal] on_exit` for this tile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    on_exit: Option<OnExit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(default, rename = "root", skip_serializing_if = "Option::is_none")]
    files_root: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// `"npm run dev"` (a shell line) or `["cargo", "watch"]` (an argv).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
enum CommandSpec {
    Line(String),
    Argv(Vec<String>),
}

// ----- parsing -----

/// A parsed layout file, ready to insert: fresh tile IDs, absolute paths.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub name: Option<String>,
    pub cwd: Option<String>,
    pub tabs: Vec<Tab>,
}

/// Parses and checks a layout file. Relative paths resolve against
/// `base_dir` (absolute) when given.
pub fn parse(text: &str, base_dir: Option<&Path>) -> Result<Plan, Error> {
    let mut file: FileSpec = toml::from_str(text).map_err(|e| {
        invalid(format!(
            "not a valid layout file: {}",
            e.to_string().trim_end()
        ))
    })?;
    if let Some(v) = file.version
        && v != 1
    {
        return Err(invalid(format!("unsupported layout file version {v}")));
    }
    let cwd = resolve(base_dir, file.cwd.as_deref()).or_else(|| base_dir.map(path_string));
    let single = file.take_single();
    let specs: Vec<TabSpec> = match (single.is_empty(), file.tabs.is_empty()) {
        (true, true) => return Err(invalid("the layout file has no tiles")),
        (false, false) => {
            return Err(invalid(
                "use either top-level `layout`/`tile`/`root` or `[[tab]]` sections, not both",
            ));
        }
        (false, true) => vec![single],
        (true, false) => file.tabs,
    };
    let ctx = Ctx {
        base_dir,
        cwd: cwd.clone(),
    };
    let tabs = specs
        .into_iter()
        .enumerate()
        .map(|(i, spec)| {
            let label = match &spec.name {
                Some(name) => format!("tab {} (`{name}`)", i + 1),
                None => format!("tab {}", i + 1),
            };
            build_tab(spec, &ctx).map_err(|e| invalid(format!("{label}: {e}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Plan {
        name: file.name.filter(|n| !n.trim().is_empty()),
        cwd,
        tabs,
    })
}

struct Ctx<'a> {
    base_dir: Option<&'a Path>,
    /// The file's terminal cwd, absolute when the file had a base.
    cwd: Option<String>,
}

fn build_tab(spec: TabSpec, ctx: &Ctx<'_>) -> Result<Tab, String> {
    let mut tiles = Vec::new();
    let mut focus = Vec::new();
    let mut layout: Option<TabLayout> = None;
    let root = match (spec.root, spec.tiles.is_empty()) {
        (Some(_), false) => return Err("give either `root` or `tile` entries, not both".into()),
        (None, true) => return Err("no tiles".into()),
        (Some(root), true) => {
            if spec.layout.is_some()
                || spec.masters.is_some()
                || spec.ratio.is_some()
                || spec.orientation.is_some()
                || spec.sizes.is_some()
            {
                return Err(
                    "`layout`, `masters`, `ratio`, `orientation` and `sizes` go with `tile` \
                     entries, not `root`"
                        .into(),
                );
            }
            build_node(root, ctx, &mut tiles, &mut focus, "root")?
        }
        (None, false) => {
            let mut leaves = Vec::new();
            for (i, t) in spec.tiles.into_iter().enumerate() {
                let at = format!("tile {}", i + 1);
                if t.split.is_some() || !t.children.is_empty() {
                    return Err(format!("{at}: splits go in `root`, not in `tile` entries"));
                }
                leaves.push(build_node(t, ctx, &mut tiles, &mut focus, &at)?);
            }
            let ids: Vec<TileId> = leaves.iter().map(Node::first_tile).collect();
            let how = spec.layout.unwrap_or(Arrangement::Tile);
            if let Some(kind) = how.kind() {
                if spec.sizes.is_some() {
                    return Err(format!(
                        "`sizes` is for `columns` and `rows`, not `{}`",
                        kind.name()
                    ));
                }
                layout = Some(dynamic_layout(
                    kind,
                    how == Arrangement::Tall,
                    ids.clone(),
                    spec.masters,
                    spec.ratio,
                    spec.orientation,
                )?);
                Node::leaf(ids[0])
            } else {
                if spec.masters.is_some() || spec.orientation.is_some() {
                    return Err(
                        "`masters` and `orientation` are for dynamic layouts (`tile`, \
                         `three_column`)"
                            .into(),
                    );
                }
                arrange(how, &ids, spec.ratio, spec.sizes)?
            }
        }
    };
    if tiles.len() > MAX_TILES {
        return Err(format!(
            "{} tiles; a tab holds at most {MAX_TILES}",
            tiles.len()
        ));
    }
    let focused = match focus.as_slice() {
        [] => root.first_tile(),
        [one] => *one,
        _ => return Err("more than one tile has `focus = true`".into()),
    };
    let mut tab = Tab::manual(
        TabId::new(),
        spec.name.filter(|n| !n.trim().is_empty()),
        root,
        tiles,
        focused,
    );
    if let Some(layout) = layout {
        tab.layout = layout;
        tab.sync();
    }
    Ok(tab)
}

/// A dynamic tab's layout from a file's keys. `tall` means `tile` with one
/// master.
fn dynamic_layout(
    kind: LayoutKind,
    tall: bool,
    order: Vec<TileId>,
    masters: Option<u8>,
    ratio: Option<f64>,
    orientation: Option<Orientation>,
) -> Result<TabLayout, String> {
    let mut params = LayoutParams::default();
    match kind {
        LayoutKind::Tile => {
            if tall && masters.is_some_and(|m| m != 1) {
                return Err("`tall` has one master; use `layout = \"tile\"` for more".into());
            }
            let p: &mut TileParams = &mut params.tile;
            if let Some(m) = masters {
                if m > 10 {
                    return Err(format!("`masters` must be 0 to 10, not {m}"));
                }
                p.masters = m;
            }
            if let Some(r) = ratio {
                p.ratio = in_range(r, 0.2, 0.8, "`ratio`")?;
            }
            if let Some(o) = orientation {
                p.orientation = o;
            }
        }
        LayoutKind::ThreeColumn => {
            if orientation.is_some() {
                return Err("`orientation` is for `tile`".into());
            }
            let p = &mut params.three_column;
            if let Some(m) = masters {
                if !(1..=10).contains(&m) {
                    return Err(format!("`masters` must be 1 to 10, not {m}"));
                }
                p.masters = m;
            }
            if let Some(r) = ratio {
                p.ratio = in_range(r, 0.2, 0.75, "`ratio`")?;
            }
        }
        _ => {
            if masters.is_some() || ratio.is_some() || orientation.is_some() {
                return Err(format!(
                    "`{}` takes no `masters`, `ratio` or `orientation`",
                    kind.name()
                ));
            }
        }
    }
    Ok(TabLayout {
        kind,
        order,
        params,
        ..TabLayout::default()
    })
}

fn in_range(x: f64, min: f64, max: f64, what: &str) -> Result<f64, String> {
    if x.is_finite() && (min..=max).contains(&x) {
        Ok(x)
    } else {
        Err(format!("{what} must be between {min} and {max}, not {x}"))
    }
}

fn arrange(
    how: Arrangement,
    ids: &[TileId],
    ratio: Option<f64>,
    sizes: Option<Vec<f64>>,
) -> Result<Node, String> {
    match how {
        Arrangement::Tile | Arrangement::Tall | Arrangement::Monocle | Arrangement::ThreeColumn => {
            unreachable!("dynamic arrangements build a TabLayout")
        }
        Arrangement::Columns | Arrangement::Rows => {
            if ratio.is_some() {
                return Err("`ratio` is for `tile` and `three_column`; use `sizes`".into());
            }
            let axis = if how == Arrangement::Columns {
                Axis::Horizontal
            } else {
                Axis::Vertical
            };
            let sizes = sizes.unwrap_or_else(|| vec![1.0; ids.len()]);
            chain(ids.iter().map(|id| Node::leaf(*id)).collect(), &sizes, axis)
        }
    }
}

fn build_node(
    spec: NodeSpec,
    ctx: &Ctx<'_>,
    tiles: &mut Vec<Tile>,
    focus: &mut Vec<TileId>,
    at: &str,
) -> Result<Node, String> {
    if let Some(axis) = spec.split {
        let tile_fields = spec.kind.is_some()
            || spec.title.is_some()
            || spec.focus
            || spec.command.is_some()
            || spec.cwd.is_some()
            || !spec.env.is_empty()
            || spec.on_exit.is_some()
            || spec.url.is_some()
            || spec.path.is_some()
            || spec.files_root.is_some();
        if tile_fields {
            return Err(format!(
                "{at}: a split has only `split`, `sizes` and `children`"
            ));
        }
        if spec.children.len() < 2 {
            return Err(format!("{at}: a split needs at least two children"));
        }
        let n = spec.children.len();
        let sizes = spec.sizes.unwrap_or_else(|| vec![1.0; n]);
        let children = spec
            .children
            .into_iter()
            .enumerate()
            .map(|(i, c)| build_node(c, ctx, tiles, focus, &format!("{at}.children[{}]", i + 1)))
            .collect::<Result<Vec<_>, _>>()?;
        let axis = match axis {
            SplitAxis::Horizontal => Axis::Horizontal,
            SplitAxis::Vertical => Axis::Vertical,
        };
        return chain(children, &sizes, axis).map_err(|e| format!("{at}: {e}"));
    }
    if !spec.children.is_empty() || spec.sizes.is_some() {
        return Err(format!("{at}: `children` and `sizes` need `split`"));
    }
    let kind = spec.kind.unwrap_or(if spec.url.is_some() {
        Kind::Browser
    } else if spec.path.is_some() {
        Kind::Preview
    } else if spec.files_root.is_some() {
        Kind::Files
    } else {
        Kind::Terminal
    });
    let only = |what: &str, set: bool, kind_name: &str| {
        if set {
            Err(format!("{at}: `{what}` only applies to {kind_name} tiles"))
        } else {
            Ok(())
        }
    };
    if kind != Kind::Terminal {
        only("command", spec.command.is_some(), "terminal")?;
        only("cwd", spec.cwd.is_some(), "terminal")?;
        only("env", !spec.env.is_empty(), "terminal")?;
        only("on_exit", spec.on_exit.is_some(), "terminal")?;
    }
    if kind != Kind::Browser {
        only("url", spec.url.is_some(), "browser")?;
    }
    if kind != Kind::Preview {
        only("path", spec.path.is_some(), "preview")?;
    }
    if kind != Kind::Files {
        only("root", spec.files_root.is_some(), "files")?;
    }
    let content = match kind {
        Kind::Terminal => {
            for key in spec.env.keys() {
                if key.is_empty() || key.contains('=') || key.starts_with("AZULEJO_") {
                    return Err(format!(
                        "{at}: `env.{key}` isn't allowed (empty, has `=`, or reserved AZULEJO_*)"
                    ));
                }
            }
            let command = match spec.command {
                None => None,
                Some(CommandSpec::Line(line)) if line.trim().is_empty() => {
                    return Err(format!("{at}: `command` is empty"));
                }
                Some(CommandSpec::Line(line)) => Some(vec![line]),
                Some(CommandSpec::Argv(argv)) if argv.is_empty() => {
                    return Err(format!("{at}: `command` is empty"));
                }
                // One word as an argv still needs quoting: a lone element
                // is sent as a shell line.
                Some(CommandSpec::Argv(argv)) if argv.len() == 1 => {
                    Some(vec![shell_quote(&argv[0])])
                }
                Some(CommandSpec::Argv(argv)) => Some(argv),
            };
            TileContent::Terminal {
                cwd: resolve(ctx.base_dir, spec.cwd.as_deref()).or_else(|| ctx.cwd.clone()),
                command,
                env: spec.env,
                herdr: None,
                ended: false,
                on_exit: spec.on_exit,
            }
        }
        Kind::Browser => TileContent::Browser {
            url: spec.url.filter(|u| !u.trim().is_empty()),
            page_title: None,
        },
        Kind::Preview => TileContent::Preview {
            path: resolve(ctx.base_dir, spec.path.as_deref()),
        },
        Kind::Files => TileContent::Files {
            root: resolve(ctx.base_dir, spec.files_root.as_deref()).or_else(|| ctx.cwd.clone()),
        },
    };
    let mut tile = Tile::new(content);
    tile.title = spec.title.filter(|t| !t.trim().is_empty());
    let id = tile.id;
    if spec.focus {
        focus.push(id);
    }
    tiles.push(tile);
    Ok(Node::leaf(id))
}

/// Nests `nodes` along `axis` (`a` first, right-nested), giving each the
/// share `sizes` asks for.
fn chain(mut nodes: Vec<Node>, sizes: &[f64], axis: Axis) -> Result<Node, String> {
    if sizes.len() != nodes.len() {
        return Err(format!(
            "{} sizes for {} children",
            sizes.len(),
            nodes.len()
        ));
    }
    if let Some(bad) = sizes.iter().find(|s| !(s.is_finite() && **s > 0.0)) {
        return Err(format!("sizes must be positive numbers, not {bad}"));
    }
    let Some(last) = nodes.pop() else {
        return Err("no tiles".into());
    };
    let mut node = last;
    let mut rest: f64 = sizes[sizes.len() - 1];
    for (child, size) in nodes.into_iter().zip(sizes).rev() {
        rest += size;
        let ratio = size / rest;
        if !(MIN_RATIO - 1e-9..=MAX_RATIO + 1e-9).contains(&ratio) {
            return Err(format!(
                "sizes {sizes:?} are too uneven: every tile needs at least {:.0}% of what's left",
                MIN_RATIO * 100.0
            ));
        }
        node = Node::Split {
            axis,
            ratio: ratio.clamp(MIN_RATIO, MAX_RATIO),
            a: Box::new(child),
            b: Box::new(node),
        };
    }
    Ok(node)
}

/// `value` made absolute against `base` (and `~` expanded); `None` when absent
/// or empty.
fn resolve(base: Option<&Path>, value: Option<&str>) -> Option<String> {
    let value = value.map(str::trim).filter(|v| !v.is_empty())?;
    let path = if value == "~" || value.starts_with("~/") {
        match std::env::var_os("HOME") {
            Some(home) => {
                PathBuf::from(home).join(value.trim_start_matches('~').trim_start_matches('/'))
            }
            None => PathBuf::from(value),
        }
    } else {
        PathBuf::from(value)
    };
    let path = match base {
        Some(base) if path.is_relative() => base.join(path),
        _ => path,
    };
    Some(path_string(&normalise(&path)))
}

fn path_string(p: &Path) -> String {
    p.display().to_string()
}

/// Drops `.` and folds `..` lexically (the path may not exist yet).
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// Quotes one word for POSIX shells and fish: bare when it's plainly safe,
/// single-quoted otherwise.
pub fn shell_quote(word: &str) -> String {
    let safe = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:,+@%".contains(c));
    if safe {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// The line typed into a terminal for `command` (see `TileContent::Terminal`).
pub fn command_line(command: &[String]) -> Option<String> {
    match command {
        [] => None,
        [line] => Some(line.clone()),
        argv => Some(
            argv.iter()
                .map(|w| shell_quote(w))
                .collect::<Vec<_>>()
                .join(" "),
        ),
    }
}

// ----- apply -----

/// `layout.apply` with `toml` (the core turns `path` into `toml` first).
pub fn apply(s: &mut Session, focus: Focus, p: LayoutApply) -> Result<Reply, Error> {
    if p.path.is_some() {
        return Err(Error::new(
            ErrorCode::Unavailable,
            "reading a layout file needs a running core; pass `toml` instead",
        ));
    }
    let text = p.toml.ok_or_else(|| invalid("give `path` or `toml`"))?;
    let base = p.base_dir.as_deref().map(Path::new);
    if let Some(b) = base
        && b.is_relative()
    {
        return Err(invalid("`base_dir` must be absolute"));
    }
    let plan = parse(&text, base)?;
    let workspace = match p.mode {
        ApplyMode::NewWorkspace => {
            let name = plan
                .name
                .clone()
                .or_else(|| {
                    base.and_then(Path::file_name)
                        .map(|n| n.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "project".into());
            match focus.window {
                Some(window) => s.create_workspace(window, name, plan.cwd.clone())?,
                None => s.create_window(name, plan.cwd.clone()).1,
            }
        }
        ApplyMode::NewTab | ApplyMode::ReplaceTab => p
            .workspace
            .or(focus.workspace)
            .ok_or_else(|| Error::from(azulejo_model::ModelError::NothingFocused))?,
    };
    let replaced = match p.mode {
        ApplyMode::ReplaceTab => s.workspace(workspace)?.active_tab,
        _ => None,
    };
    let mut first = None;
    for tab in plan.tabs {
        let id = s.insert_tab(workspace, tab)?;
        first.get_or_insert(id);
    }
    let first = first.expect("a plan has at least one tab");
    if let Some(old) = replaced {
        s.close_tab(old)?;
    }
    s.focus_workspace(workspace)?;
    s.focus_tab(first)?;
    let tile = s.tab(first)?.focused;
    Ok(Reply::Created(Created {
        window: None,
        workspace: Some(workspace),
        tab: Some(first),
        tile: Some(tile),
    }))
}

// ----- export -----

/// `layout.export`: TOML for a tab, or a workspace's tabs. Paths inside
/// `base_dir` come out relative to it.
pub fn export(s: &Session, focus: Focus, p: &LayoutExport) -> Result<String, Error> {
    let base = p
        .path
        .as_deref()
        .map(Path::new)
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    let base = base.as_deref();
    let file = match p.tab {
        Some(tab) => {
            let tab = s.tab(tab)?;
            let mut file = FileSpec::default();
            let spec = export_tab(tab, base, None);
            // Top-level keys can't carry a tab name; `[[tab]]` can.
            if spec.name.is_some() {
                file.tabs.push(spec);
            } else {
                file.put_single(spec);
            }
            file
        }
        None => {
            let id: WorkspaceId = p
                .workspace
                .or(focus.workspace)
                .ok_or_else(|| Error::from(azulejo_model::ModelError::NothingFocused))?;
            let ws = s.workspace(id)?;
            if ws.tabs.is_empty() {
                return Err(Error::new(
                    ErrorCode::InvalidState,
                    "the workspace has no tabs to export",
                )
                .with_data(serde_json::json!({ "workspace": id })));
            }
            let cwd = ws.cwd.as_deref();
            let mut file = FileSpec {
                name: Some(ws.name.clone()),
                cwd: cwd.map(|c| relative(base, c)),
                ..FileSpec::default()
            };
            let tabs: Vec<TabSpec> = ws.tabs.iter().map(|t| export_tab(t, base, cwd)).collect();
            if tabs.len() == 1 && tabs[0].name.is_none() {
                file.put_single(tabs.into_iter().next().expect("one tab"));
            } else {
                file.tabs = tabs;
            }
            file
        }
    };
    let body = toml::to_string(&file)
        .map_err(|e| Error::new(ErrorCode::Internal, format!("writing TOML: {e}")))?;
    Ok(format!(
        "# Azulejo layout file. Open it with `azj layout apply <path>`.\n\n{body}"
    ))
}

fn export_tab(tab: &Tab, base: Option<&Path>, ws_cwd: Option<&str>) -> TabSpec {
    let many = tab.tiles.len() > 1;
    let tile = |id: TileId| -> NodeSpec {
        let t = tab.tile(id).expect("tree and tiles agree");
        let mut spec = NodeSpec {
            title: t.title.clone(),
            focus: many && tab.focused == id,
            ..NodeSpec::default()
        };
        match &t.content {
            TileContent::Terminal {
                cwd,
                command,
                env,
                on_exit,
                ..
            } => {
                spec.kind = Some(Kind::Terminal);
                // The workspace cwd is the default; only say so when different.
                spec.cwd = cwd
                    .as_deref()
                    .filter(|c| Some(*c) != ws_cwd)
                    .map(|c| relative(base, c));
                spec.command = command.as_ref().map(|c| match c.as_slice() {
                    [line] => CommandSpec::Line(line.clone()),
                    argv => CommandSpec::Argv(argv.to_vec()),
                });
                spec.env = env.clone();
                spec.on_exit = *on_exit;
                if spec.command.is_some()
                    || spec.cwd.is_some()
                    || !spec.env.is_empty()
                    || spec.on_exit.is_some()
                {
                    // Kind inference covers terminals; keep files short.
                    spec.kind = None;
                }
            }
            TileContent::Browser { url, .. } => {
                spec.kind = Some(Kind::Browser);
                spec.url = url.clone();
                if spec.url.is_some() {
                    spec.kind = None;
                }
            }
            TileContent::Preview { path } => {
                spec.kind = Some(Kind::Preview);
                spec.path = path.as_deref().map(|p| relative(base, p));
                if spec.path.is_some() {
                    spec.kind = None;
                }
            }
            TileContent::Files { root } => {
                spec.kind = Some(Kind::Files);
                spec.files_root = root.as_deref().map(|r| relative(base, r));
                if spec.files_root.is_some() {
                    spec.kind = None;
                }
            }
        }
        spec
    };
    let mut out = TabSpec {
        name: tab.name.clone(),
        ..TabSpec::default()
    };
    // A dynamic tab: its kind, parameters that differ from the defaults, and
    // the tiles in order. Stack weights aren't part of the format.
    if tab.is_dynamic() {
        let d = LayoutParams::default();
        let p = tab.layout.params;
        out.tiles = tab.layout.order.iter().copied().map(tile).collect();
        match tab.layout.kind {
            LayoutKind::Tile => {
                out.layout = Some(Arrangement::Tile);
                out.masters = (p.tile.masters != d.tile.masters).then_some(p.tile.masters);
                out.ratio =
                    ((p.tile.ratio - d.tile.ratio).abs() > 1e-9).then_some(round(p.tile.ratio));
                out.orientation =
                    (p.tile.orientation != d.tile.orientation).then_some(p.tile.orientation);
            }
            LayoutKind::ThreeColumn => {
                out.layout = Some(Arrangement::ThreeColumn);
                let c = p.three_column;
                out.masters = (c.masters != d.three_column.masters).then_some(c.masters);
                out.ratio =
                    ((c.ratio - d.three_column.ratio).abs() > 1e-9).then_some(round(c.ratio));
            }
            _ => out.layout = Some(Arrangement::Monocle),
        }
        return out;
    }
    if let Some((ratio, master, stack)) = as_tall(&tab.root)
        && (0.2..=0.8).contains(&ratio)
    {
        // A manual tall tree reads back as `tile`, which rebuilds it exactly.
        out.layout = Some(Arrangement::Tile);
        out.ratio = ((ratio - 0.5).abs() > 1e-9).then_some(round(ratio));
        out.tiles = std::iter::once(master).chain(stack).map(tile).collect();
        return out;
    }
    out.root = Some(export_node(&tab.root, &tile));
    out
}

/// `(master share, master, stack)` when `root` is the canonical tall shape
/// with an even stack, which `layout = "tall"` rebuilds exactly.
fn as_tall(root: &Node) -> Option<(f64, TileId, Vec<TileId>)> {
    let Node::Split {
        axis: Axis::Horizontal,
        ratio,
        a,
        b,
    } = root
    else {
        return None;
    };
    let Node::Leaf { id: master } = a.as_ref() else {
        return None;
    };
    let mut stack = Vec::new();
    let mut node = b.as_ref();
    loop {
        match node {
            Node::Leaf { id } => {
                stack.push(*id);
                break;
            }
            Node::Split {
                axis: Axis::Vertical,
                a,
                b,
                ..
            } if matches!(a.as_ref(), Node::Leaf { .. }) => {
                stack.push(a.first_tile());
                node = b;
            }
            _ => return None,
        }
    }
    // Even: the i'th of n splits gives 1/(n-i).
    let n = stack.len();
    let mut node = b.as_ref();
    let mut i = 0;
    while let Node::Split { ratio, b, .. } = node {
        if (ratio - 1.0 / (n - i) as f64).abs() > 1e-6 {
            return None;
        }
        node = b;
        i += 1;
    }
    Some((*ratio, *master, stack))
}

/// A split tree, with runs along one axis flattened into one `children` list.
fn export_node(node: &Node, tile: &dyn Fn(TileId) -> NodeSpec) -> NodeSpec {
    let Node::Split { axis, .. } = node else {
        return tile(node.first_tile());
    };
    let axis = *axis;
    let mut children = Vec::new();
    let mut sizes = Vec::new();
    let mut left = 1.0;
    let mut node = node;
    loop {
        match node {
            Node::Split {
                axis: a_axis,
                ratio,
                a,
                b,
            } if *a_axis == axis => {
                children.push(export_node(a, tile));
                sizes.push(left * ratio);
                left *= 1.0 - ratio;
                node = b;
            }
            other => {
                children.push(export_node(other, tile));
                sizes.push(left);
                break;
            }
        }
    }
    let even = sizes.iter().all(|s| (s - sizes[0]).abs() < 1e-6);
    NodeSpec {
        split: Some(match axis {
            Axis::Horizontal => SplitAxis::Horizontal,
            Axis::Vertical => SplitAxis::Vertical,
        }),
        sizes: (!even).then(|| sizes.iter().map(|s| round(s * 100.0)).collect()),
        children,
        ..NodeSpec::default()
    }
}

fn round(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

/// `path` relative to `base` when it's inside it; otherwise as it was, but
/// tidied: doubled slashes (`$TMPDIR/x` gives `…/T//x`) and `.` parts go.
fn relative(base: Option<&Path>, path: &str) -> String {
    let path: PathBuf = Path::new(path).components().collect();
    match base.and_then(|b| path.strip_prefix(b).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => ".".into(),
        Some(rest) => path_string(rest),
        None => path_string(&path),
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidParams, message)
}

#[cfg(test)]
#[path = "layout_file_tests.rs"]
mod tests;
