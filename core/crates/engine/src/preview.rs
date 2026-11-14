//! Preview tiles in a running core (F9): watch each tile's file, render it off
//! the engine's thread, and publish `preview.updated`.
//!
//! Rendered content isn't part of the session and isn't saved; the core keeps
//! the latest per tile for `preview.get`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use azulejo_model::{Session, TileContent, TileId};
use azulejo_preview::{Kind, Rendered, Stamp, Watcher};
use azulejo_protocol::{Event, PreviewContent, PreviewKind};

use crate::driver::LogFn;

/// How long a file must be quiet after a change before it's re-rendered.
pub(crate) const DEBOUNCE: Duration = Duration::from_millis(50);

/// Receives a finished render, on whichever thread produced it.
pub(crate) type Deliver = Arc<dyn Fn(TileId, PathBuf, Rendered) + Send + Sync>;

pub(crate) struct Previews {
    watcher: Option<Watcher<TileId>>,
    deliver: Deliver,
    log: LogFn,
    /// The file each preview tile shows, as the tile stores it.
    shown: HashMap<TileId, PathBuf>,
    latest: HashMap<TileId, PreviewContent>,
    /// The file stamp behind each tile's latest render.
    stamps: HashMap<TileId, Option<Stamp>>,
    revision: u64,
}

impl Previews {
    pub(crate) fn new(deliver: Deliver, log: LogFn) -> Self {
        let watcher = {
            let deliver = deliver.clone();
            Watcher::new(DEBOUNCE, move |tile, path: PathBuf| {
                let rendered = azulejo_preview::render(&path);
                deliver(tile, path, rendered);
            })
        };
        let watcher = match watcher {
            Ok(w) => Some(w),
            Err(e) => {
                log(format!(
                    "file watching unavailable ({e}); previews won't live-reload"
                ));
                None
            }
        };
        Previews {
            watcher,
            deliver,
            log,
            shown: HashMap::new(),
            latest: HashMap::new(),
            stamps: HashMap::new(),
            revision: 0,
        }
    }

    /// Matches watches to the session's preview tiles: starts watching new
    /// files (and renders them), stops for closed tiles and changed paths.
    pub(crate) fn sync(&mut self, session: &Session) {
        let wanted: HashMap<TileId, PathBuf> = session
            .tiles()
            .filter_map(|(_, tile)| match &tile.content {
                TileContent::Preview { path: Some(path) } => Some((tile.id, PathBuf::from(path))),
                _ => None,
            })
            .collect();
        let stale: Vec<TileId> = self
            .shown
            .iter()
            .filter(|(id, path)| wanted.get(id) != Some(*path))
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            self.shown.remove(&id);
            self.latest.remove(&id);
            self.stamps.remove(&id);
            if let Some(w) = &mut self.watcher {
                w.unwatch(&id);
            }
        }
        for (id, path) in wanted {
            if self.shown.contains_key(&id) {
                continue;
            }
            if let Some(w) = &mut self.watcher
                && let Err(e) = w.watch(id, &path)
            {
                (self.log)(format!(
                    "can't watch {} for {id} ({e}); it won't live-reload",
                    path.display()
                ));
            }
            self.shown.insert(id, path.clone());
            render_in_background(id, path, self.deliver.clone());
        }
    }

    /// A finished render: the event to publish, or `None` when the tile has
    /// closed or moved on to another file since, or when the file is exactly
    /// as it was at the last render (a repeated or replayed notification).
    pub(crate) fn accept(
        &mut self,
        tile: TileId,
        path: &Path,
        rendered: Rendered,
    ) -> Option<Event> {
        if self.shown.get(&tile).map(PathBuf::as_path) != Some(path) {
            return None;
        }
        if self.latest(tile, path).is_some() && self.stamps.get(&tile) == Some(&rendered.stamp) {
            return None;
        }
        self.stamps.insert(tile, rendered.stamp);
        self.revision += 1;
        let content = content(tile, path, rendered, self.revision);
        self.latest.insert(tile, content.clone());
        Some(Event::PreviewUpdated(content))
    }

    /// The latest render for `tile` if it shows `path`.
    pub(crate) fn latest(&self, tile: TileId, path: &Path) -> Option<&PreviewContent> {
        self.latest
            .get(&tile)
            .filter(|c| Path::new(&c.path) == path)
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }
}

fn render_in_background(tile: TileId, path: PathBuf, deliver: Deliver) {
    let spawned = std::thread::Builder::new()
        .name("azulejo-preview-render".into())
        .spawn({
            let path = path.clone();
            let deliver = deliver.clone();
            move || {
                let rendered = azulejo_preview::render(&path);
                deliver(tile, path, rendered);
            }
        });
    if spawned.is_err() {
        let rendered = azulejo_preview::render(&path);
        deliver(tile, path, rendered);
    }
}

/// The protocol's view of a render.
pub(crate) fn content(
    tile: TileId,
    path: &Path,
    rendered: Rendered,
    revision: u64,
) -> PreviewContent {
    PreviewContent {
        tile,
        path: path.display().to_string(),
        kind: kind(rendered.kind),
        html: rendered.html,
        language: rendered.language,
        truncated: rendered.truncated,
        revision,
    }
}

fn kind(kind: Kind) -> PreviewKind {
    match kind {
        Kind::Markdown => PreviewKind::Markdown,
        Kind::Code => PreviewKind::Code,
        Kind::Text => PreviewKind::Text,
        Kind::Image => PreviewKind::Image,
        Kind::Video => PreviewKind::Video,
        Kind::Audio => PreviewKind::Audio,
        Kind::Pdf => PreviewKind::Pdf,
        Kind::Binary => PreviewKind::Binary,
        Kind::Directory => PreviewKind::Directory,
        Kind::Missing => PreviewKind::Missing,
        Kind::Unreadable => PreviewKind::Unreadable,
    }
}

#[cfg(test)]
#[path = "preview_tests.rs"]
mod tests;
