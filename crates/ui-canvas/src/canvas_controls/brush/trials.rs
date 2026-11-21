//! Trials of the stroke trail's texture keys.
#![allow(clippy::panic, clippy::expect_used, reason = "trials assert by panicking")]

use super::{CoverageMask, StrokeTrail};
use crate::canvas::testing::canvas_story::{on_canvas_view, story_view};
use crate::contract::Show as _;
use crate::primitives::Serials;
use crate::testing::{Frames, trial};
use craft_behavior::kurbo::{Point, Rect as DocRect};
use egui::{Shape, Ui};

fn stamped(serials: &mut Serials) -> CoverageMask {
    let mut mask = CoverageMask::new(serials, DocRect::new(40.0, 60.0, 160.0, 140.0), 256);
    mask.stamp_capsule(Point::new(60.0, 100.0), Point::new(140.0, 100.0), 5.0);
    mask
}

/// Two owners each count from one: their masks have the same serial and revision, and the trials they show at once
/// still upload a texture each.
#[trial]
fn two_owners_masks_with_one_serial_show_two_textures() {
    let (mut first, mut second) = (stamped(&mut Serials::default()), stamped(&mut Serials::default()));
    let mut frames = Frames::new();
    let mut ui = |ui: &mut Ui| {
        for (name, mask) in [("Trail A", &mut first), ("Trail B", &mut second)] {
            crate::testing::memory::scoped_by(ui, name, |ui| {
                on_canvas_view(
                    ui,
                    |room, scale| story_view(room, scale, Point::new(100.0, 100.0), 1.0),
                    None,
                    |ui, rect, _| StrokeTrail::new(name, mask).show_at(ui, rect),
                );
            });
        }
    };
    frames.idle(3, &mut ui);
    let shapes = frames.frame_shapes(&mut ui);
    let mut textures: Vec<_> = shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Mesh(mesh) => Some(mesh.texture_id),
            _ => None,
        })
        .collect();
    textures.sort_by_key(|id| format!("{id:?}"));
    textures.dedup();
    assert_eq!(textures.len(), 2, "two trials with one serial each show their own texture: {textures:?}");
}
