use egui::Id;

/// The new page's entry progress. Applications translate/clip their child UI and set its opacity
/// using this value. It describes only the new content; no outgoing content is retained.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContentEntryFrame {
    /// Eased 0..1, with 1 meaning fully settled.
    pub progress: f32,
    /// +1 enters from the right, -1 from the left, 0 means no movement. Order is caller-owned.
    pub direction: i8,
    /// Request an egui repaint while true.
    pub moving: bool,
}

impl ContentEntryFrame {
    const SETTLED: Self = Self { progress: 1.0, direction: 0, moving: false };
}

/// Directional entry when switching tabs, reusable for ribbons, inspectors and settings pages.
/// Keep one instance per content region. IDs must not depend on translated labels.
/// The first page appears immediately; a new page enters from the side indicated by its order
/// relative to the previous one. Reordering the same tab alone never starts an animation.
#[derive(Clone, Debug, Default)]
pub struct ContentEntry {
    state: Option<State>,
}

#[derive(Clone, Debug)]
struct State {
    selected: Id,
    order: usize,
    start: f64,
    last_time: f64,
    direction: i8,
}

impl ContentEntry {
    /// `now` is monotonic seconds, `duration` seconds (bounded to ten). Motion-off, invalid time,
    /// invalid/non-positive durations and a backwards clock settle immediately, without replay
    /// when motion is re-enabled. A second selection change starts the *new* page's entry again;
    /// the host owns page replacement, focus, enabled state and any input suppression.
    pub fn update(&mut self, selected: Id, order: usize, now: f64, duration: f32, motion: bool) -> ContentEntryFrame {
        let active = motion && now.is_finite() && duration.is_finite() && duration > 0.0;
        let time = if now.is_finite() { now } else { 0.0 };
        let mut direction = 0;
        let mut start = time;
        if let Some(previous) = self.state.as_ref().filter(|p| active && time >= p.last_time) {
            if previous.selected == selected {
                direction = previous.direction;
                start = previous.start;
            } else {
                direction = match order.cmp(&previous.order) {
                    std::cmp::Ordering::Greater => 1,
                    std::cmp::Ordering::Less => -1,
                    std::cmp::Ordering::Equal => 0,
                };
            }
        }
        let progress = if active { ((time - start) / f64::from(duration.min(10.0))).clamp(0.0, 1.0) as f32 } else { 1.0 };
        if progress >= 1.0 {
            direction = 0;
        }
        self.state = Some(State { selected, order, start, last_time: time, direction });
        if direction == 0 {
            ContentEntryFrame::SETTLED
        } else {
            ContentEntryFrame { progress: 1.0 - (1.0 - progress).powi(3), direction, moving: true }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_direction_interruption_and_initial_appearance() {
        let mut entry = ContentEntry::default();
        let a = Id::new("a");
        let b = Id::new("b");
        let c = Id::new("c");
        assert_eq!(entry.update(a, 0, 0.0, 1.0, true), ContentEntryFrame::SETTLED);
        assert_eq!(entry.update(b, 1, 1.0, 1.0, true), ContentEntryFrame { progress: 0.0, direction: 1, moving: true });
        let mid = entry.update(b, 1, 1.5, 1.0, true);
        assert!(mid.progress > 0.0 && mid.progress < 1.0);
        assert_eq!(entry.update(c, 0, 1.5, 1.0, true).direction, -1);
        assert_eq!(entry.update(c, 0, 3.0, 1.0, true), ContentEntryFrame::SETTLED);
        assert_eq!(entry.update(c, 20, 4.0, 1.0, true), ContentEntryFrame::SETTLED);
    }
    #[test]
    fn motion_off_and_hostile_clocks_settle_without_replay() {
        let mut entry = ContentEntry::default();
        let a = Id::new("a");
        let b = Id::new("b");
        entry.update(a, 0, 0.0, 1.0, true);
        entry.update(b, 1, 1.0, 1.0, true);
        assert_eq!(entry.update(b, 1, 1.1, 1.0, false), ContentEntryFrame::SETTLED);
        assert_eq!(entry.update(b, 1, 1.2, 1.0, true), ContentEntryFrame::SETTLED);
        for duration in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
            assert_eq!(entry.update(a, 0, 2.0, duration, true), ContentEntryFrame::SETTLED);
        }
        assert_eq!(entry.update(b, 1, f64::NAN, 1.0, true), ContentEntryFrame::SETTLED);
        entry.update(a, 0, 2.0, 1.0, true);
        assert_eq!(entry.update(b, 1, 1.0, 1.0, true), ContentEntryFrame::SETTLED);
    }
}
