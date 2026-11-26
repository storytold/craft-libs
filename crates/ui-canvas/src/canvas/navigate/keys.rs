//! Page keys: Photoshop's keyboard scrolling of a canvas, read from the intents a focused canvas takes.

use super::pan;
use craft_behavior::kurbo::{Point, Size, Vec2 as DocVec};
use craft_behavior::{CanvasView, MoveMods, NavMove};
use egui::Vec2;

/// What a page key does to the view.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum PageKey {
    /// Scroll by this many screen points (positive: show what is below or to the right).
    Scroll(Vec2),
    /// Show the document's top-left corner (Home).
    Start,
    /// Show the document's bottom-right corner (End).
    End,
}

impl PageKey {
    /// Document pixels a Shift + page key scrolls, at any zoom ([`small_step`](Self::small_step)).
    pub const SMALL_STEP: f32 = 10.0;

    /// The points a Shift + page key scrolls a view of `zoom` (screen points per document pixel) on a screen of
    /// `pixels_per_point`: [`SMALL_STEP`](Self::SMALL_STEP) document pixels, rounded to whole device pixels and at
    /// least one, so 10 at 50 %, 100 %, 200 % and 400 % and 12 at 25 % (2.5 device pixels make 3), whatever the
    /// display scaling. A zoom or scale that is not finite and positive steps one device pixel.
    pub fn small_step(zoom: f64, pixels_per_point: f32) -> f32 {
        let ppp = if pixels_per_point.is_finite() && pixels_per_point > 0.0 { pixels_per_point } else { 1.0 };
        let device = f64::from(Self::SMALL_STEP) * zoom * f64::from(ppp);
        let device = if device.is_finite() { device.round().max(1.0) } else { 1.0 };
        // At most a few thousand device pixels at the largest zoom, so the cast keeps the value.
        #[allow(clippy::cast_possible_truncation)]
        let device = device.min(f64::from(f32::MAX)) as f32;
        device / ppp
    }

    /// Photoshop's page keys for the move a focused canvas took: Page Up and Page Down scroll a canvas height up or
    /// down, Cmd/Ctrl turns them sideways (Page Up left, Page Down right), Shift makes the step `small` points
    /// ([`small_step`](Self::small_step)); Home and End show the corners. `None` for other moves, with Alt, or with
    /// Shift or Cmd on Home and End. `canvas` is the canvas size in points.
    pub fn of(step: NavMove, mods: MoveMods, canvas: Vec2, small: f32) -> Option<PageKey> {
        if mods.fine {
            return None;
        }
        let sign = match step {
            NavMove::PageUp => -1.0,
            NavMove::PageDown => 1.0,
            NavMove::Start if !mods.extend && !mods.lead => return Some(PageKey::Start),
            NavMove::End if !mods.extend && !mods.lead => return Some(PageKey::End),
            _ => return None,
        };
        let sideways = mods.lead;
        let length = match (mods.extend, sideways) {
            (true, _) => small,
            (false, true) => canvas.x,
            (false, false) => canvas.y,
        };
        Some(PageKey::Scroll(if sideways { Vec2::new(sign * length, 0.0) } else { Vec2::new(0.0, sign * length) }))
    }

    /// Applies the key to `view` for a document of `size`: a scroll, or the corner at the canvas corner the document
    /// runs on from. The zoom stays. With Overscroll off the caller clamps the result with the canvas's scroll
    /// limits. Returns whether the view changed.
    pub fn apply(self, view: &mut CanvasView, size: Size) -> bool {
        let before = *view;
        match self {
            PageKey::Scroll(delta) => pan(view, -delta),
            PageKey::Start | PageKey::End => {
                let end = self == PageKey::End;
                // The document corner goes to the canvas corner the document runs on from: the top-left one in an
                // upright view, another when it is turned or mirrored.
                let toward = if end { -1.0 } else { 1.0 };
                let corner = if end { Point::new(size.width, size.height) } else { Point::ORIGIN };
                let dir = view.map_vec(DocVec::new(toward, toward));
                let canvas = view.rect.size();
                let at = Vec2::new(-dir.x.signum() * canvas.x / 2.0, -dir.y.signum() * canvas.y / 2.0);
                let center = corner - view.unmap_vec(at);
                if center.x.is_finite() && center.y.is_finite() {
                    view.center = center;
                }
            }
        }
        *view != before
    }
}
