//! Project layout files (F6), the command catalogue for the palette (F5) and
//! config reloads (F25b).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use azulejo_model::{TabId, WorkspaceId};

/// Where `layout.apply` puts a layout file's tabs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApplyMode {
    /// A new workspace named after the file, in the focused window.
    #[default]
    NewWorkspace,
    /// Extra tabs in `workspace` (the focused one by default).
    NewTab,
    /// The file's tabs take the place of the workspace's active tab, which
    /// closes. Its terminals detach; their herdr panes keep running.
    ReplaceTab,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LayoutApply {
    /// A layout file (`.azulejo.toml`). Give exactly one of `path` and `toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The layout file's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toml: Option<String>,
    /// What relative `cwd` and `path` values in the file resolve against.
    /// Defaults to the file's directory, or the workspace's cwd for `toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_dir: Option<String>,
    /// For `new_tab` and `replace_tab`; the focused workspace by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceId>,
    #[serde(default)]
    pub mode: ApplyMode,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LayoutExport {
    /// Export one tab. Without it, the whole workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<TabId>,
    /// The workspace to export; the focused one by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceId>,
    /// Also write the file here. Paths in it are made relative to its
    /// directory where they're inside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// One entry in the command palette.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CommandEntry {
    /// Stable id: the keymap action name (`split_right`, `switch_tab_3`), or
    /// the method name for commands no key is bound to (`browser.open`).
    pub id: String,
    pub title: String,
    /// `tile`, `layout`, `tab`, `workspace`, `window`, `terminal`, `browser`,
    /// `preview` or `app`.
    pub category: String,
    /// The protocol method it sends, if any; `None` for shell-only actions
    /// (palette, sidebar, find…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Keymap action name, when the entry is a keymap action. With `index`
    /// for indexed actions (`switch_tab` + 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u8>,
    /// Current bindings on this platform, in config syntax (`cmd+d`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chords: Vec<String>,
    /// Acts on the focused tile, so the palette greys it out without one.
    #[serde(default)]
    pub needs_tile: bool,
    /// Needs input before it can run (a URL, a path, a name): the palette
    /// prompts for `prompt` first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Not supported yet; shown greyed out.
    #[serde(default)]
    pub planned: bool,
    /// The only context the action works in (`browser`, `files`, `palette`,
    /// `find`, `resize`…); `None` when it works anywhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

/// One keymap action in one context, from `config.keys`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KeyBinding {
    /// The config name (`split_right`, `switch_tab`).
    pub action: String,
    /// `global`, `prefix` (after the prefix key), or the context section it
    /// sits in: `terminal`, `browser`, `preview`, `files`, `palette`, `find`
    /// or `resize`.
    pub context: String,
    pub title: String,
    /// The bundled defaults' chords, in config syntax (`ctrl+1..8` for
    /// indexed actions). Empty when unbound by default.
    #[serde(default)]
    pub default: Vec<String>,
    /// The chords in effect now. Empty when unbound.
    #[serde(default)]
    pub current: Vec<String>,
    /// `default` (bundled keymap) or `user` (the config file sets it).
    pub source: String,
}

/// A problem in the config file, for the shell's banner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigDiagnostic {
    /// `error` or `warning`.
    pub severity: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// 1-based.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// 1-based.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
}

/// Payload of `config.reloaded`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigReloaded {
    /// False when the new file had errors: the previous config stays.
    pub applied: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<ConfigDiagnostic>,
}
