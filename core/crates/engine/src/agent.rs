//! Agent state on terminal tiles (F11, F16), from herdr's agent status.
//!
//! herdr reports a status per pane: through a per-pane subscription
//! (`pane.agent_status_changed`) and in every survey. A tile's
//! [`AgentInfo::attention`] lights up when its agent becomes blocked or
//! finishes, and stays lit until someone looks: focusing the tile, or
//! `tile.seen` from a shell that knows the tile is on screen.

use azulejo_herdr as herdr;
use azulejo_model::{AgentInfo, AgentStatus, Session, Tile, TileContent, TileId};

use crate::adopt::same_pane;

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

pub fn from_herdr(status: herdr::AgentStatus) -> AgentStatus {
    match status {
        herdr::AgentStatus::Idle => AgentStatus::Idle,
        herdr::AgentStatus::Working => AgentStatus::Working,
        herdr::AgentStatus::Blocked => AgentStatus::Blocked,
        herdr::AgentStatus::Done => AgentStatus::Done,
        herdr::AgentStatus::Unknown => AgentStatus::Unknown,
    }
}

/// Applies a reported status to a terminal tile. Returns whether anything
/// changed. `name` is kept from before when herdr doesn't repeat it.
///
/// Attention: becoming blocked or done turns it on. Going back to work turns
/// it off (someone answered, maybe from a phone). Done → idle keeps it: the
/// result is still unseen.
pub fn update(tile: &mut Tile, name: Option<String>, status: AgentStatus, at_ms: u64) -> bool {
    if !matches!(tile.content, TileContent::Terminal { .. }) {
        return false;
    }
    let prev = tile.agent.as_ref();
    // An unknown status with no agent means herdr let go of the agent.
    let name = match status {
        AgentStatus::Unknown => name,
        _ => name.or_else(|| prev.and_then(|p| p.name.clone())),
    };
    let next = if status == AgentStatus::Unknown && name.is_none() {
        None
    } else {
        Some(match prev {
            Some(p) if p.status == status => AgentInfo {
                name,
                status,
                since_ms: p.since_ms,
                attention: p.attention,
            },
            _ => AgentInfo {
                name,
                status,
                since_ms: at_ms,
                attention: status.wants_attention()
                    || (status == AgentStatus::Idle
                        && prev.is_some_and(|p| p.attention && p.status == AgentStatus::Done)),
            },
        })
    };
    if tile.agent == next {
        return false;
    }
    tile.agent = next;
    true
}

/// Brings every bound tile's agent in line with a survey: on connect, after a
/// resync, and when panes appear. Returns whether anything changed.
pub fn refresh(session: &mut Session, survey: &herdr::Survey, at_ms: u64) -> bool {
    let updates: Vec<(TileId, Option<String>, AgentStatus)> = session
        .tiles()
        .filter_map(|(_, tile)| match &tile.content {
            TileContent::Terminal {
                herdr: Some(h),
                ended: false,
                ..
            } => survey
                .panes
                .iter()
                .find(|p| same_pane(p, h))
                .map(|p| (tile.id, p.agent.clone(), from_herdr(p.agent_status))),
            _ => None,
        })
        .collect();
    let mut changed = false;
    for (id, name, status) in updates {
        if let Ok(tile) = session.tile_mut(id) {
            changed |= update(tile, name, status, at_ms);
        }
    }
    changed
}

/// Clears a tile's attention. Returns whether it was on.
pub fn clear_attention(session: &mut Session, tile: TileId) -> bool {
    match session.tile_mut(tile).ok().and_then(|t| t.agent.as_mut()) {
        Some(a) if a.attention => {
            a.attention = false;
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminal() -> Tile {
        Tile::new(TileContent::terminal())
    }

    fn set(t: &mut Tile, status: AgentStatus, at: u64) -> bool {
        update(t, Some("claude".into()), status, at)
    }

    #[test]
    fn blocked_and_done_ask_for_attention_working_clears_it() {
        let mut t = terminal();
        assert!(set(&mut t, AgentStatus::Working, 1));
        let a = t.agent.clone().unwrap();
        assert_eq!(
            (a.status, a.since_ms, a.attention),
            (AgentStatus::Working, 1, false)
        );

        assert!(set(&mut t, AgentStatus::Blocked, 2));
        assert!(t.needs_attention());
        assert_eq!(t.agent.as_ref().unwrap().since_ms, 2);

        // The same status again changes nothing, not even `since`.
        assert!(!set(&mut t, AgentStatus::Blocked, 3));
        assert_eq!(t.agent.as_ref().unwrap().since_ms, 2);

        // Answered from elsewhere: back to work, no attention.
        assert!(set(&mut t, AgentStatus::Working, 4));
        assert!(!t.needs_attention());

        // Done stays lit through idle until seen.
        assert!(set(&mut t, AgentStatus::Done, 5));
        assert!(set(&mut t, AgentStatus::Idle, 6));
        assert!(t.needs_attention());
    }

    #[test]
    fn seen_stays_seen_until_the_next_transition() {
        let mut s = Session::new();
        let (_, ws) = s.create_window("w", None);
        s.create_tab(ws, terminal(), None).unwrap();
        let id = s.focused_tile().unwrap();
        set(s.tile_mut(id).unwrap(), AgentStatus::Blocked, 1);
        assert!(clear_attention(&mut s, id));
        assert!(!clear_attention(&mut s, id));
        // Still blocked: the same report doesn't relight it.
        assert!(!set(s.tile_mut(id).unwrap(), AgentStatus::Blocked, 2));
        assert!(!s.tile(id).unwrap().needs_attention());
        // Idle after seen-done stays dark.
        set(s.tile_mut(id).unwrap(), AgentStatus::Done, 3);
        clear_attention(&mut s, id);
        set(s.tile_mut(id).unwrap(), AgentStatus::Idle, 4);
        assert!(!s.tile(id).unwrap().needs_attention());
    }

    #[test]
    fn unknown_without_an_agent_clears_the_state() {
        let mut t = terminal();
        set(&mut t, AgentStatus::Working, 1);
        // herdr keeps the name until the agent is released.
        assert!(!update(&mut t, None, AgentStatus::Working, 2));
        assert_eq!(t.agent.as_ref().unwrap().name.as_deref(), Some("claude"));
        // Released: unknown and no agent.
        assert!(update(&mut t, None, AgentStatus::Unknown, 3));
        assert_eq!(t.agent, None);
        let mut fresh = terminal();
        assert!(!update(&mut fresh, None, AgentStatus::Unknown, 1));
        assert_eq!(fresh.agent, None);
    }

    #[test]
    fn only_terminals_have_agents() {
        let mut t = Tile::new(TileContent::Browser {
            url: None,
            page_title: None,
        });
        assert!(!set(&mut t, AgentStatus::Blocked, 1));
        assert_eq!(t.agent, None);
    }
}
