//! Retargetable selection underline geometry, with caller-owned painting and time.

use egui::{Id, Rect};

/// Geometry to paint this frame. Request another egui repaint while `moving` is true.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnderlineFrame {
    /// The underline rectangle to paint.
    pub rect: Rect,
    /// Whether another frame is needed to finish the transition.
    pub moving: bool,
}

/// One strip's underline. Keep a separate instance per strip; identity is independent of labels.
///
/// The first target appears immediately. A selection change slides from the currently displayed
/// rectangle, including when interrupted. Layout changes for the same selection snap to the new
/// rectangle so window resizing does not leave the marker behind its tab. This owns no egui
/// memory, theme, selected page or widget interaction.
#[derive(Clone, Debug, Default)]
pub struct SlidingUnderline {
    state: Option<Segment>,
}

#[derive(Clone, Debug)]
struct Segment {
    selected: Id,
    from: Rect,
    to: Rect,
    start: f64,
    last_time: f64,
    duration: f64,
}

impl SlidingUnderline {
    /// Sample and advance the underline. `now` is monotonic seconds (e.g. egui input time).
    /// `duration` is seconds, capped at ten. Motion-off, non-positive/invalid durations, invalid
    /// time and a backwards clock snap to the target and discard any in-flight movement.
    /// Invalid, empty or excessive rectangles clear the state and return `None`.
    pub fn update(&mut self, selected: Id, target: Rect, now: f64, duration: f32, motion: bool) -> Option<UnderlineFrame> {
        if !target.is_finite() || target.width() <= 0.0 || target.height() <= 0.0 || target.size().max_elem() > 1_000_000.0 {
            self.state = None;
            return None;
        }
        let duration = if motion && now.is_finite() && duration.is_finite() { f64::from(duration.clamp(0.0, 10.0)) } else { 0.0 };
        let time = if now.is_finite() { now } else { 0.0 };
        let snap = Segment { selected, from: target, to: target, start: time, last_time: time, duration };
        let next = match self.state.as_ref() {
            Some(previous) if duration > 0.0 && time >= previous.last_time && previous.selected != selected => {
                Segment { from: previous.sample(time).rect, ..snap }
            }
            Some(previous) if duration > 0.0 && time >= previous.last_time && previous.to == target && previous.duration == duration => {
                Segment { last_time: time, ..previous.clone() }
            }
            _ => snap,
        };
        let frame = next.sample(time);
        self.state = Some(next);
        Some(frame)
    }
}

impl Segment {
    fn sample(&self, now: f64) -> UnderlineFrame {
        let progress = if self.duration > 0.0 { ((now - self.start) / self.duration).clamp(0.0, 1.0) as f32 } else { 1.0 };
        let eased = 1.0 - (1.0 - progress).powi(3);
        // f64 intermediates avoid overflow for finite rectangles on opposite ends of f32 space.
        let lerp = |a: f32, b: f32| (f64::from(a) * (1.0 - f64::from(eased)) + f64::from(b) * f64::from(eased)) as f32;
        let rect = Rect::from_min_max(
            egui::pos2(lerp(self.from.min.x, self.to.min.x), lerp(self.from.min.y, self.to.min.y)),
            egui::pos2(lerp(self.from.max.x, self.to.max.x), lerp(self.from.max.y, self.to.max.y)),
        );
        UnderlineFrame { rect, moving: self.from != self.to && progress < 1.0 }
    }
}
