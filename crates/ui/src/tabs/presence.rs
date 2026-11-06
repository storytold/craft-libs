use egui::Id;
use std::collections::{HashMap, HashSet};

/// One present tab or outgoing paint-only ghost. Use `visibility` for opacity/entry offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabPresenceFrame {
    pub id: Id,
    /// The caller's current strip position, or the last known position for a departing tab.
    pub position: usize,
    pub visibility: f32,
    /// False for outgoing ghosts: do not register tab input, commands or keytips for them.
    pub present: bool,
    /// Request a repaint while true.
    pub moving: bool,
}

/// Tracks contextual tab entry/exit independently of any document model or egui global memory.
/// First-frame tabs appear immediately. Keep one instance per strip and supply stable IDs.
/// Removed tabs remain as outgoing ghosts until their fade completes. Reappearing tabs reverse
/// from their current visibility rather than flashing or retaining a second outgoing copy.
#[derive(Clone, Debug, Default)]
pub struct TabPresence {
    initialized: bool,
    last_time: f64,
    entries: HashMap<Id, Entry>,
}

#[derive(Clone, Debug)]
struct Entry {
    position: usize,
    present: bool,
    from: f32,
    start: f64,
    duration: f64,
}

impl Entry {
    fn visibility(&self, now: f64) -> f32 {
        let p = if self.duration > 0.0 { ((now - self.start) / self.duration).clamp(0.0, 1.0) as f32 } else { 1.0 };
        let to = if self.present { 1.0 } else { 0.0 };
        self.from + (to - self.from) * (1.0 - (1.0 - p).powi(3))
    }
}

impl TabPresence {
    /// Advance the strip using `(stable ID, position)` pairs and monotonic seconds. Duration is
    /// seconds, capped at ten. Motion-off, invalid time/duration and clock rollback settle current
    /// tabs and discard ghosts immediately. Duplicate IDs use their first supplied position.
    ///
    /// Results preserve input order, followed by outgoing ghosts in last-position/ID order.
    /// The caller owns labels/rectangles needed to paint ghosts, overflow, selection and input.
    /// A disappearing selected tab still requires the app to choose an applicable replacement.
    pub fn update(&mut self, shown: &[(Id, usize)], now: f64, duration: f32, motion: bool) -> Vec<TabPresenceFrame> {
        let animate = self.initialized && motion && now.is_finite() && now >= self.last_time && duration.is_finite() && duration > 0.0;
        let time = if now.is_finite() { now } else { 0.0 };
        let duration = if animate { f64::from(duration.min(10.0)) } else { 0.0 };
        let mut seen = HashSet::with_capacity(shown.len());
        let mut frames = Vec::with_capacity(shown.len());
        for &(id, position) in shown {
            if !seen.insert(id) {
                continue;
            }
            let entry =
                self.entries.entry(id).or_insert(Entry { position, present: true, from: if animate { 0.0 } else { 1.0 }, start: time, duration });
            if !entry.present || !animate {
                let from = if animate { entry.visibility(time) } else { 1.0 };
                *entry = Entry { position, present: true, from, start: time, duration };
            }
            entry.position = position;
            let visibility = entry.visibility(time);
            frames.push(TabPresenceFrame { id, position, visibility, present: true, moving: visibility < 1.0 });
        }
        let mut outgoing = Vec::new();
        self.entries.retain(|&id, entry| {
            if seen.contains(&id) {
                return true;
            }
            if !animate {
                return false;
            }
            if entry.present {
                let from = entry.visibility(time);
                *entry = Entry { position: entry.position, present: false, from, start: time, duration };
            }
            let visibility = entry.visibility(time);
            if visibility <= 0.0 {
                return false;
            }
            outgoing.push(TabPresenceFrame { id, position: entry.position, visibility, present: false, moving: true });
            true
        });
        outgoing.sort_by_key(|frame| (frame.position, frame.id.value()));
        frames.extend(outgoing);
        self.initialized = true;
        self.last_time = time;
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outgoing_tabs_reverse_without_duplicates_and_keep_their_last_position() {
        let mut presence = TabPresence::default();
        let a = Id::new("table");
        let b = Id::new("chart");
        assert_eq!(presence.update(&[(a, 2)], 0.0, 1.0, true)[0].visibility, 1.0);
        let frames = presence.update(&[(b, 3)], 1.0, 1.0, true);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].visibility, 0.0);
        assert_eq!(frames[1].position, 2);
        assert!(!frames[1].present);
        let frames = presence.update(&[(b, 3)], 1.5, 1.0, true);
        let outgoing = frames[1].visibility;
        assert!(outgoing > 0.0 && outgoing < 1.0);
        let frames = presence.update(&[(a, 4), (b, 3), (a, 9)], 1.5, 1.0, true);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].visibility, outgoing);
        assert!(frames[0].present);
        assert_eq!(frames[0].position, 4);
        assert!(presence.update(&[(a, 4)], 4.0, 1.0, true).iter().all(|f| f.visibility.is_finite()));
        presence.update(&[], 5.0, 1.0, true);
        let frames = presence.update(&[], 6.0, 1.0, true);
        assert!(frames.is_empty());
    }
    #[test]
    fn motion_off_invalid_parameters_and_rollback_drop_ghosts_and_settle() {
        let a = Id::new("a");
        let b = Id::new("b");
        for (now, duration, motion) in [(1.2, 1.0, false), (f64::NAN, 1.0, true), (1.2, f32::NAN, true), (1.2, -1.0, true), (0.5, 1.0, true)] {
            let mut presence = TabPresence::default();
            presence.update(&[(a, 0)], 0.0, 1.0, true);
            presence.update(&[(b, 1)], 1.0, 1.0, true);
            assert_eq!(
                presence.update(&[(b, 1)], now, duration, motion),
                vec![TabPresenceFrame { id: b, position: 1, visibility: 1.0, present: true, moving: false }]
            );
        }
    }
}
