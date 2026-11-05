//! Tiles and what they show.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::TileId;

/// The kind of content a tile shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TileKind {
    Terminal,
    Browser,
    Preview,
    Files,
}

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
    /// saved-machine ID (ADR 0011). herdr IDs are only unique per server.
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
    Browser {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        /// The loaded page's `<title>`, as the shell last reported it
        /// (`browser.state`). Separate from the tile's own `title`, which is
        /// the user's name for the tile.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        page_title: Option<String>,
    },
    Preview {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    Files {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        root: Option<String>,
    },
}

impl TileContent {
    pub fn kind(&self) -> TileKind {
        match self {
            TileContent::Terminal { .. } => TileKind::Terminal,
            TileContent::Browser { .. } => TileKind::Browser,
            TileContent::Preview { .. } => TileKind::Preview,
            TileContent::Files { .. } => TileKind::Files,
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

/// What herdr says the agent in a terminal tile is doing (F16).
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

/// The agent running in a terminal tile, from herdr's agent status (F11,
/// F16). Absent when herdr knows of no agent there.
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
}

/// What herdr reports about a terminal tile's pane (F28): enough to say what
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

/// What a tile's header strip shows (F28), worked out by the core so every
/// shell says the same: e.g. `zsh` — `just dogfood`, `claude` —
/// `~/src/azulejo`, `README.md` — `~/code/shop`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TileHeader {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
}

/// How a preview tile shows its file (F29).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PreviewMode {
    /// Rendered: markdown as a document, images as images.
    #[default]
    Rendered,
    /// The file's text, highlighted as code (markdown included).
    Source,
}

impl PreviewMode {
    pub fn is_rendered(&self) -> bool {
        *self == PreviewMode::Rendered
    }
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
    /// Preview tiles only: rendered or source (F29). Saved per tile.
    #[serde(default, skip_serializing_if = "PreviewMode::is_rendered")]
    pub preview_mode: PreviewMode,
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
            preview_mode: PreviewMode::Rendered,
            header: None,
        }
    }

    /// The header this tile should show (F28). `home` abbreviates paths to
    /// `~`.
    ///
    /// - Terminal: the agent herdr detected, else the shell; then the command
    ///   running in the foreground, else the working directory. A name the
    ///   user gave the tile replaces the title.
    /// - Browser: the page's title, else its host; then the host.
    /// - Preview: the file's name; then its folder.
    /// - Files: the root folder's name; then its path.
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
            TileContent::Browser { url, page_title } => {
                let host = url.as_deref().and_then(host_of);
                let page = page_title.clone().filter(|t| !t.trim().is_empty());
                TileHeader {
                    title: named
                        .or(page)
                        .or_else(|| host.clone())
                        .unwrap_or_else(|| "browser".into()),
                    subtitle: host,
                }
            }
            TileContent::Preview { path } => {
                let (name, folder) = split_path(path.as_deref());
                TileHeader {
                    title: named.or(name).unwrap_or_else(|| "preview".into()),
                    subtitle: folder.as_deref().map(tilde),
                }
            }
            TileContent::Files { root } => {
                let root = root.clone().filter(|r| !r.is_empty());
                let shown = root.as_deref().map(tilde).unwrap_or_else(|| "~".into());
                let name = match root.as_deref() {
                    Some(r) => split_path(Some(r)).0.unwrap_or_else(|| r.to_owned()),
                    None => "home".into(),
                };
                TileHeader {
                    title: named.unwrap_or(name),
                    subtitle: Some(shown),
                }
            }
        }
    }

    /// Whether the tile's agent needs a look (F11).
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

/// A URL's host (and port), e.g. `localhost:5173`; `None` for URLs without
/// one (`about:blank`, `file:` URLs).
fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    (!host.is_empty()).then(|| host.to_owned())
}

/// A path's last component and the folder holding it.
fn split_path(path: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(path) = path
        .map(|p| p.trim_end_matches('/'))
        .filter(|p| !p.is_empty())
    else {
        return (None, None);
    };
    match path.rsplit_once('/') {
        Some((folder, name)) => (
            Some(name.to_owned()),
            Some(if folder.is_empty() {
                "/".into()
            } else {
                folder.to_owned()
            }),
        ),
        None => (Some(path.to_owned()), None),
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
    fn browser_preview_and_files() {
        let b = Tile::new(TileContent::Browser {
            url: Some("http://localhost:5173/patterns/14".into()),
            page_title: Some("Pattern 14".into()),
        });
        let h = b.compute_header(HOME);
        assert_eq!(
            (h.title.as_str(), h.subtitle.as_deref()),
            ("Pattern 14", Some("localhost:5173"))
        );
        let blank = Tile::new(TileContent::Browser {
            url: None,
            page_title: None,
        });
        assert_eq!(blank.compute_header(HOME).title, "browser");

        let p = Tile::new(TileContent::Preview {
            path: Some("/Users/example/code/docs/adr/0009.md".into()),
        });
        let h = p.compute_header(HOME);
        assert_eq!(
            (h.title.as_str(), h.subtitle.as_deref()),
            ("0009.md", Some("~/code/docs/adr"))
        );

        let f = Tile::new(TileContent::Files {
            root: Some("/Users/example/code".into()),
        });
        let h = f.compute_header(HOME);
        assert_eq!(
            (h.title.as_str(), h.subtitle.as_deref()),
            ("code", Some("~/code"))
        );
    }

    #[test]
    fn tilde_only_at_folder_boundaries() {
        assert_eq!(tilde("/Users/example", HOME), "~");
        assert_eq!(tilde("/Users/example-other/x", HOME), "/Users/example-other/x");
        assert_eq!(tilde("/tmp", None), "/tmp");
    }
}
