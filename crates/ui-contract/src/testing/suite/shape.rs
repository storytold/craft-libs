//! The cases every shape shares: a story paints only inside the rect it was shown at.

use super::{ANY, Case, Subject};
use crate::testing::fixture::Fixture;
use egui::{Rect, Vec2};

pub(super) const CASES: &[Case] = &[Case::new("paints-inside", ANY, small_rooms_paint_inside_the_shown_rect)];

/// The slack between a painted bound and the shown rect: the rounding of the layout, as the layout cases allow. A
/// shape's own slack adds half its stroke width.
const ROUNDING: f32 = 0.5;

/// The rects a story may paint in, before a shape's own slack: the area it allocated, grown by the type's declared
/// outset, and the rect of every open layer. A layer that opened a cover or is modal (a scrim, a full-window mount)
/// covers the window.
fn painted_areas(fixture: &Fixture, outset: f32) -> Vec<Rect> {
    use crate::runtime::overlay;
    let ctx = fixture.ctx();
    let mut areas = vec![fixture.shown().expand(outset)];
    for entry in overlay::open_entries(ctx) {
        areas.push(entry.rect);
        if entry.cover.is_some() || entry.rank.is_modal() {
            areas.push(ctx.content_rect());
        }
    }
    areas
}

/// A story shown in a room of 1 by 1 and in a room of half its settled size does not panic, paints only inside the
/// rect it allocated grown by the type's declared outset (or the rect of a layer it opened) and registers finite
/// part shapes. A shadow is no paint bound.
fn small_rooms_paint_inside_the_shown_rect(subject: &Subject) -> Result<(), String> {
    let tokens = crate::testing::style::tokens(craft_theme::Mode::Light, craft_theme::Contrast::Standard);
    let outset = (subject.probe.paint_outset)(&tokens);
    for story in &subject.stories {
        let settled = subject.base(story).fixture.shown().size();
        for room in [Vec2::splat(1.0), settled / 2.0] {
            let mut fixture = Fixture::story(story);
            fixture.room(room);
            fixture.settle();
            let areas = painted_areas(&fixture, outset);
            let shapes = fixture.shape_bounds().iter().zip(fixture.shape_overshoot());
            if let Some((outside, _)) =
                shapes.into_iter().find(|(bounds, overshoot)| !areas.iter().any(|area| area.expand(ROUNDING + **overshoot).contains_rect(**bounds)))
            {
                return Err(format!("{}: in a room of {room:?} a shape paints at {outside:?}, outside {:?}", story.name, fixture.shown()));
            }
            let tree = fixture.tree();
            if tree.keys().into_iter().filter_map(|key| tree.bounds_of(key)).any(|bounds| !(bounds.min.is_finite() && bounds.max.is_finite())) {
                return Err(format!("{}: in a room of {room:?} a part has non-finite bounds", story.name));
            }
        }
    }
    Ok(())
}
