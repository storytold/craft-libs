//! Ephemeral presence (spec §6.1): cursors, selections, viewports, per-app projections.
//!
//! `Presence` is never stored — the server forwards the latest value per client (rate-shaped at
//! ~30 Hz, last-wins, sanitized) and forgets it the moment the connection drops. The only
//! exception in the whole system is "who is spotlighting", which the server keeps long enough
//! to tell late joiners (spec §2.7). Everything here rides the wire as transient messages and
//! never enters the document, the journal, or any saved state.

use crate::ids::{ClientId, NodeId};
use serde::{Deserialize, Serialize};

/// One presence message (the union of every app's projection; apps fill what they know).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PresenceMsg {
    /// The cursor position in page coordinates, plus the page id.
    pub cursor: Option<(f64, f64)>,
    /// The page (canvas) or slide (deck) the peer is on.
    pub page: Option<u64>,
    /// The selected object ids (bounded to 10,000 — spec §2.12).
    pub selection: Vec<NodeId>,
    /// The viewport rectangle, so others can offer follow mode.
    pub viewport: Option<(f64, f64, f64, f64)>,
    /// The peer is editing this text object (with caret and selection in characters).
    pub editing_text: Option<(NodeId, usize, Option<usize>)>,
    /// Timeline apps: the shared playhead position in ticks.
    pub playhead: Option<u64>,
    /// Timeline apps: the track range being edited.
    pub track_range: Option<(u32, u64, u64)>,
    /// `ArtCraft`: a generation job is in flight (job id, model).
    pub generating: Option<(String, String)>,
}

/// The server-side presence sanitizer and rate budget (spec §6.1, spec §2.12 values).
#[derive(Clone, Debug)]
pub struct PresenceGuard {
    seen_per_second: std::collections::HashMap<ClientId, u32>,
    window_start: u64,
    /// The presence rate budget: 30 messages/second on average.
    pub rate_per_second: u32,
    /// The burst allowance above the average rate.
    pub burst: u32,
    /// Selections are bounded (v2: 10,000 ids).
    pub max_selection: usize,
}

impl Default for PresenceGuard {
    fn default() -> Self {
        Self { seen_per_second: std::collections::HashMap::new(), window_start: 0, rate_per_second: 30, burst: 60, max_selection: 10_000 }
    }
}

impl PresenceGuard {
    /// A new guard.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Admit or drop a presence message (last-wins means a drop is harmless, spec §6.1).
    pub fn admit(&mut self, from: ClientId, msg: &mut PresenceMsg, now_ms: u64) -> bool {
        // Reset the window each second (a pure time bucket; the clock is injected by the caller).
        if now_ms.saturating_sub(self.window_start) >= 1_000 {
            self.seen_per_second.clear();
            self.window_start = now_ms;
        }
        let budget = self.seen_per_second.entry(from).or_insert(0);
        if *budget >= self.rate_per_second + self.burst {
            return false; // Silently dropped: the newest value wins anyway.
        }
        *budget += 1;
        // Sanitize: numbers finite, selections bounded (spec §6.1 `sanitize`).
        if let Some((x, y)) = msg.cursor
            && (!x.is_finite() || !y.is_finite())
        {
            msg.cursor = None;
        }
        if msg.selection.len() > self.max_selection {
            msg.selection.truncate(self.max_selection);
        }
        true
    }
}

/// A peer's presence as the UI keeps it (with the name and color the host assigned).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeerPresence {
    /// The peer's client id.
    pub client: ClientId,
    /// The display name ("Guest N" for link guests — v2 rule).
    pub name: String,
    /// The assigned color (the host's eight-color palette, v2 rule).
    pub color: String,
    /// The latest presence.
    pub presence: PresenceMsg,
}
