//! Agent state on terminal tiles (F11, F16), from herdr's agent status.
//!
//! herdr reports a status per pane: through a per-pane subscription
//! (`pane.agent_status_changed`) and in every survey. A tile's
//! [`AgentInfo::attention`] lights up when its agent becomes blocked or
//! finishes. A finished agent's stays lit until the user reads the result:
//! `tile.seen`, which the shell sends once the tile has been focused in the
//! key window for a moment. A blocked agent's stays lit until its state
//! changes: looking doesn't answer it.
//!
//! herdr has a "seen" notion of its own (`done` is a finish nobody has
//! focused in herdr yet, `idle` one somebody has), but only herdr's focus
//! commands set it, and Azulejo leaves herdr's focus alone (ADR 0012). So
//! `done` outlives a read here, and a finish in herdr's focused pane arrives
//! as `idle`. [`update`] treats both as finishes, and `attention` is the only
//! "unseen" flag views should trust (herdr-notes.md, question 11).

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
/// Attention: becoming blocked or finishing turns it on. Finishing means
/// done, or idle straight from working or blocked (herdr reports a finish in
/// its own focused pane as idle). Going back to work turns it off (someone
/// answered, maybe from a phone). Done → idle keeps it: the result is still
/// unseen here.
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
                        && prev.is_some_and(|p| match p.status {
                            AgentStatus::Working | AgentStatus::Blocked => true,
                            AgentStatus::Done => p.attention,
                            AgentStatus::Idle | AgentStatus::Unknown => false,
                        })),
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
///
/// `fresher(pane)`: herdr's stream reported that pane's status after the
/// survey was requested, so the survey's snapshot may predate it; such panes
/// keep what the stream said.
pub fn refresh(
    session: &mut Session,
    machine: &crate::link::Machine,
    survey: &herdr::Survey,
    at_ms: u64,
    fresher: impl Fn(&str) -> bool,
) -> bool {
    let updates: Vec<(TileId, Option<String>, AgentStatus)> = session
        .tiles()
        .filter_map(|(_, tile)| match &tile.content {
            TileContent::Terminal {
                herdr: Some(h),
                ended: false,
                ..
            } if h.machine == *machine && !fresher(&h.pane_id) => survey
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

/// The user read the tile's agent's result: clears a finished agent's
/// attention. A blocked agent keeps it until its state changes. Returns
/// whether anything cleared.
pub fn mark_seen(session: &mut Session, tile: TileId) -> bool {
    match session.tile_mut(tile).ok().and_then(|t| t.agent.as_mut()) {
        Some(a) if a.attention && a.status != AgentStatus::Blocked => {
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

    fn session() -> (Session, TileId) {
        let mut s = Session::new();
        let (_, ws) = s.create_window("w", None);
        s.create_tab(ws, terminal(), None).unwrap();
        let id = s.focused_tile().unwrap();
        (s, id)
    }

    #[test]
    fn seen_stays_seen_until_the_next_transition() {
        let (mut s, id) = session();
        set(s.tile_mut(id).unwrap(), AgentStatus::Working, 1);
        set(s.tile_mut(id).unwrap(), AgentStatus::Done, 2);
        assert!(mark_seen(&mut s, id));
        assert!(!mark_seen(&mut s, id));
        // Still done (herdr's own "seen" never flips): the same report, from
        // the stream or a survey, doesn't relight it.
        assert!(!set(s.tile_mut(id).unwrap(), AgentStatus::Done, 3));
        assert!(!s.tile(id).unwrap().needs_attention());
        // Idle after seen-done stays dark.
        set(s.tile_mut(id).unwrap(), AgentStatus::Idle, 4);
        assert!(!s.tile(id).unwrap().needs_attention());
        // A new run that finishes lights it again.
        set(s.tile_mut(id).unwrap(), AgentStatus::Working, 5);
        set(s.tile_mut(id).unwrap(), AgentStatus::Done, 6);
        assert!(s.tile(id).unwrap().needs_attention());
    }

    #[test]
    fn looking_does_not_answer_a_blocked_agent() {
        let (mut s, id) = session();
        set(s.tile_mut(id).unwrap(), AgentStatus::Blocked, 1);
        assert!(!mark_seen(&mut s, id));
        assert!(s.tile(id).unwrap().needs_attention());
        // Answered: back to work, then finished, and that can be read.
        set(s.tile_mut(id).unwrap(), AgentStatus::Working, 2);
        assert!(!s.tile(id).unwrap().needs_attention());
        set(s.tile_mut(id).unwrap(), AgentStatus::Done, 3);
        assert!(mark_seen(&mut s, id));
        assert!(!s.tile(id).unwrap().needs_attention());
    }

    #[test]
    fn a_finish_in_herdrs_focused_pane_still_asks_for_a_look() {
        // herdr marks its focused pane's finish seen and reports idle, not
        // done; nobody here has read it.
        let mut t = terminal();
        set(&mut t, AgentStatus::Working, 1);
        set(&mut t, AgentStatus::Idle, 2);
        assert!(t.needs_attention());
        let mut t = terminal();
        set(&mut t, AgentStatus::Blocked, 1);
        set(&mut t, AgentStatus::Idle, 2);
        assert!(t.needs_attention());
        // An agent that turns up idle (a fresh session) hasn't finished
        // anything.
        let mut t = terminal();
        set(&mut t, AgentStatus::Idle, 1);
        assert!(!t.needs_attention());
        set(&mut t, AgentStatus::Unknown, 2);
        set(&mut t, AgentStatus::Idle, 3);
        assert!(!t.needs_attention());
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
