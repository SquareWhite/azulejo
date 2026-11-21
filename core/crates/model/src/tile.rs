//! Tiles and what they show.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::TileId;

/// The kind of content a tile shows. Terminals are the only kind; the enum
/// (and the `kind` tag on the wire) leaves room for others. Older builds had
/// browser, preview and files tiles: saved state drops them on load
/// ([`REMOVED_KINDS`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum TileKind {
    Terminal,
}

/// Tile kinds earlier builds had and this one doesn't: a saved session
/// naming one loads without those tiles, and a layout file naming one is
/// refused with a clear error.
pub const REMOVED_KINDS: &[&str] = &["browser", "preview", "files"];

/// What happens to a terminal tile when the process in its pane exits
/// (`exit`, ctrl+d, a finished command). Config `[terminal] on_exit`; a tile
/// can override it (layout files' `on_exit`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OnExit {
    /// The tile closes, like any terminal window whose shell exits.
    #[default]
    Close,
    /// The tile stays, ended, offering Respawn or Close.
    Keep,
}

/// The herdr pane a terminal tile views. Both IDs are kept: herdr changes a
/// pane's ID when it moves to another workspace, while the terminal ID is what
/// `herdr terminal attach` takes. The tab ID matters because closing a herdr
/// tab reports only the tab, and herdr keeps pane and tab IDs (but not the
/// process) across a server restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HerdrRef {
    pub pane_id: String,
    pub terminal_id: String,
    pub tab_id: String,
    /// The herdr machine the pane runs on: `None` for Local, else herdr's
    /// saved-machine ID. herdr IDs are only unique per server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

impl HerdrRef {
    /// A pane on Local.
    pub fn local(
        pane_id: impl Into<String>,
        terminal_id: impl Into<String>,
        tab_id: impl Into<String>,
    ) -> Self {
        Self {
            pane_id: pane_id.into(),
            terminal_id: terminal_id.into(),
            tab_id: tab_id.into(),
            machine: None,
        }
    }
}

/// A tile's content and its kind-specific state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "lowercase")]
#[non_exhaustive]
pub enum TileContent {
    Terminal {
        /// Working directory for a new pane.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        /// Command typed into the pane's shell once it starts. One element is
        /// sent as a shell line (`npm run dev`); more are an argv, quoted for
        /// the shell. herdr has no way to replace the shell, so the shell stays
        /// when the command exits.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<Vec<String>>,
        /// Extra environment for a new pane, on top of `AZULEJO_*`.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
        /// Set once the herdr adapter has created or adopted a pane.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        herdr: Option<HerdrRef>,
        /// The herdr pane is gone (its process exited with `on_exit = "keep"`,
        /// or it was lost with its server). The shell offers Respawn or Close;
        /// binding a new pane clears it.
        #[serde(default, skip_serializing_if = "is_false")]
        ended: bool,
        /// Overrides `[terminal] on_exit` for this tile.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on_exit: Option<OnExit>,
    },
}

impl TileContent {
    pub fn kind(&self) -> TileKind {
        match self {
            TileContent::Terminal { .. } => TileKind::Terminal,
        }
    }

    /// An empty terminal in the default directory.
    pub fn terminal() -> Self {
        TileContent::Terminal {
            cwd: None,
            command: None,
            env: BTreeMap::new(),
            herdr: None,
            ended: false,
            on_exit: None,
        }
    }
}

pub(crate) fn is_false(b: &bool) -> bool {
    !*b
}

/// What herdr says the agent in a terminal tile is doing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Working,
    /// Waiting for the user, e.g. a permission prompt.
    Blocked,
    Done,
    #[default]
    #[serde(other)]
    Unknown,
}

impl AgentStatus {
    /// Statuses that ask for a look: the agent waits, or it finished.
    pub fn wants_attention(self) -> bool {
        matches!(self, AgentStatus::Blocked | AgentStatus::Done)
    }
}

/// The agent running in a terminal tile, from herdr's agent status.
/// Absent when herdr knows of no agent there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentInfo {
    /// The agent herdr detected or was told about, e.g. `claude`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub status: AgentStatus,
    /// When `status` last changed, in milliseconds since the Unix epoch.
    pub since_ms: u64,
    /// The agent became blocked or finished and needs a look. A finish
    /// stays lit until the user reads it (`tile.seen`); blocked stays lit
    /// until the agent's state changes. herdr's `done` status can outlive a
    /// read, so this, not `status`, is the "unseen" flag.
    #[serde(default, skip_serializing_if = "is_false")]
    pub attention: bool,
    /// The user marked the tile unread (`tile.mark_unread`) to come back to
    /// it. Keeps `attention` lit whatever the agent does next, until the
    /// user reads it (`tile.seen`). Saved with the session, so it survives a
    /// relaunch.
    #[serde(default, skip_serializing_if = "is_false")]
    pub unread: bool,
}

/// What herdr reports about a terminal tile's pane: enough to say what
/// runs there and where.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PaneMeta {
    /// The title the program in the pane set (OSC 0/2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The pane's working directory (its foreground process's, when known).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// The shell's name, e.g. `zsh`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// The command running in the foreground when it isn't the shell, e.g.
    /// `just dogfood`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

/// What a tile's header strip shows, worked out by the core so every
/// shell says the same: e.g. `zsh` — `just dogfood`, `claude` —
/// `~/src/azulejo`, `README.md` — `~/code/shop`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TileHeader {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
}

/// One tile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Tile {
    pub id: TileId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(flatten)]
    pub content: TileContent,
    /// Terminal tiles only: the agent herdr reports in the pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentInfo>,
    /// Terminal tiles only: title, cwd and command herdr reports for the pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<PaneMeta>,
    /// What the header strip shows; the core keeps it current.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<TileHeader>,
}

impl Tile {
    pub fn new(content: TileContent) -> Self {
        Self {
            id: TileId::new(),
            title: None,
            content,
            agent: None,
            pane: None,
            header: None,
        }
    }

    /// The header this tile should show. `home` abbreviates paths to
    /// `~`.
    ///
    /// - Terminal: the agent herdr detected, else the shell; then the command
    ///   running in the foreground, else the working directory. A name the
    ///   user gave the tile replaces the title.
    pub fn compute_header(&self, home: Option<&str>) -> TileHeader {
        let tilde = |p: &str| tilde(p, home);
        let named = self.title.clone().filter(|t| !t.is_empty());
        match &self.content {
            TileContent::Terminal { cwd, ended, .. } => {
                let meta = self.pane.clone().unwrap_or_default();
                let agent = self
                    .agent
                    .as_ref()
                    .and_then(|a| a.name.clone())
                    .filter(|n| !n.is_empty());
                let cwd = meta.cwd.as_deref().or(cwd.as_deref()).map(tilde);
                let (title, subtitle) = match (&agent, &meta.command) {
                    (Some(agent), _) => (agent.clone(), cwd),
                    (None, Some(command)) => (
                        meta.shell.clone().unwrap_or_else(|| "terminal".into()),
                        Some(command.clone()),
                    ),
                    (None, None) => (meta.shell.clone().unwrap_or_else(|| "terminal".into()), cwd),
                };
                let subtitle = if *ended {
                    Some("session ended".into())
                } else {
                    subtitle
                };
                TileHeader {
                    title: named.unwrap_or(title),
                    subtitle,
                }
            }
        }
    }

    /// Whether the tile's agent needs a look.
    pub fn needs_attention(&self) -> bool {
        self.agent.as_ref().is_some_and(|a| a.attention)
    }

    pub fn kind(&self) -> TileKind {
        self.content.kind()
    }
}

/// `path` with the home folder shown as `~`.
pub fn tilde(path: &str, home: Option<&str>) -> String {
    let Some(home) = home
        .map(|h| h.trim_end_matches('/'))
        .filter(|h| !h.is_empty())
    else {
        return path.to_owned();
    };
    if path == home {
        return "~".into();
    }
    match path.strip_prefix(home) {
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_owned(),
    }
}

#[cfg(test)]
mod header_tests {
    use super::*;

    const HOME: Option<&str> = Some("/Users/example");

    fn terminal(meta: PaneMeta) -> Tile {
        let mut t = Tile::new(TileContent::terminal());
        t.pane = Some(meta);
        t
    }

    #[test]
    fn terminal_shows_shell_and_command_or_cwd() {
        let idle = terminal(PaneMeta {
            shell: Some("zsh".into()),
            cwd: Some("/Users/example/src/azulejo".into()),
            ..PaneMeta::default()
        });
        let h = idle.compute_header(HOME);
        assert_eq!(h.title, "zsh");
        assert_eq!(h.subtitle.as_deref(), Some("~/src/azulejo"));

        let busy = terminal(PaneMeta {
            shell: Some("zsh".into()),
            cwd: Some("/Users/example/src/azulejo".into()),
            command: Some("just dogfood".into()),
            ..PaneMeta::default()
        });
        let h = busy.compute_header(HOME);
        assert_eq!(
            (h.title.as_str(), h.subtitle.as_deref()),
            ("zsh", Some("just dogfood"))
        );
    }

    #[test]
    fn terminal_prefers_agent_then_user_name() {
        let mut t = terminal(PaneMeta {
            shell: Some("zsh".into()),
            cwd: Some("/Users/example/x".into()),
            command: Some("claude".into()),
            ..PaneMeta::default()
        });
        t.agent = Some(AgentInfo {
            name: Some("claude".into()),
            status: AgentStatus::Working,
            since_ms: 0,
            attention: false,
            unread: false,
        });
        let h = t.compute_header(HOME);
        assert_eq!(
            (h.title.as_str(), h.subtitle.as_deref()),
            ("claude", Some("~/x"))
        );
        t.title = Some("reviewer".into());
        assert_eq!(t.compute_header(HOME).title, "reviewer");
    }

    #[test]
    fn terminal_without_meta_falls_back() {
        let t = Tile::new(TileContent::terminal());
        let h = t.compute_header(HOME);
        assert_eq!(h.title, "terminal");
        assert_eq!(h.subtitle, None);
    }

    #[test]
    fn tilde_only_at_folder_boundaries() {
        assert_eq!(tilde("/Users/example", HOME), "~");
        assert_eq!(
            tilde("/Users/example-other/x", HOME),
            "/Users/example-other/x"
        );
        assert_eq!(tilde("/tmp", None), "/tmp");
    }
}
