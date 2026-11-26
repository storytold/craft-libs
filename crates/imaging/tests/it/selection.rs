//! The selection overlay and the mask views: when the outline is traced again, and how the view commands change the
//! view state.

use craft_behavior::CanvasView;
use craft_behavior::kurbo::Point;
use craft_imaging::selection::PreviewMode;
use craft_imaging::trace::{MaskRect, MaskSource};
use craft_imaging::ui::selection::SelectionOverlay;
use craft_ui::canvas::Canvas;
use craft_ui::contract::ShowContainer as _;
use craft_ui::testing::{Fixture, Story, StoryData, StoryOutput, trial};
use egui::{Rect, Ui, pos2};

/// A rectangle mask whose revision the test sets.
struct Block {
    revision: u64,
}

impl MaskSource for Block {
    fn size(&self) -> [u32; 2] {
        [200, 150]
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn content_bounds(&self) -> MaskRect {
        MaskRect::new(40, 30, 120, 90)
    }

    fn read_row(&self, y: i32, x0: i32, step: u32, out: &mut [f32]) {
        for (i, value) in out.iter_mut().enumerate() {
            let x = x0 + i as i32 * step as i32;
            *value = f32::from(self.content_bounds().contains(x, y));
        }
    }
}

fn show_overlay(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (revision, reduction) = (*data.value("revision", 1u64), *data.value("reduction", 1u32));
    let at = ui.available_rect_before_wrap();
    let view = CanvasView::new(Rect::from_min_max(pos2(0.0, 0.0), at.max), Point::new(100.0, 75.0), 1.0);
    let mask = Block { revision };
    let shown = Canvas::new("Document")
        .view(view)
        .show(ui, |ui, body| SelectionOverlay::new("Selection", &mask).reduction(reduction).preview(PreviewMode::None).show_at(ui, body.rect));
    if let Some(output) = shown.inner {
        *data.value("retraced", false) = output.retraced;
        *data.value("bounds", None::<MaskRect>) = output.bounds;
    }
    StoryOutput::default()
}

fn retraced(fixture: &Fixture) -> bool {
    *fixture.data().value("retraced", false)
}

#[trial]
fn the_overlay_retraces_only_when_the_revision_or_the_reduction_changes() {
    let mut fixture = Fixture::story(&Story::new("selection-overlay", show_overlay));
    fixture.settle();
    assert!(!retraced(&fixture), "a settled frame draws the cached trace");
    assert_eq!(*fixture.data().value("bounds", None::<MaskRect>), Some(MaskRect::new(40, 30, 120, 90)));
    *fixture.data().value("revision", 1u64) = 2;
    fixture.idle(1);
    assert!(retraced(&fixture), "the first frame after a new revision traces");
    fixture.idle(2);
    assert!(!retraced(&fixture), "and the frames after it draw the cache");
    *fixture.data().value("reduction", 1u32) = 2;
    fixture.idle(2);
    assert_eq!(
        *fixture.data().value("bounds", None::<MaskRect>),
        Some(MaskRect::new(80, 60, 240, 180)),
        "the bounds are in full-size document pixels"
    );
}

/// A fully selected mask of the given size in pixels.
struct Filled([u32; 2]);

impl MaskSource for Filled {
    fn size(&self) -> [u32; 2] {
        self.0
    }

    fn revision(&self) -> u64 {
        1
    }

    fn content_bounds(&self) -> MaskRect {
        MaskRect::of_size(self.size())
    }

    fn read_row(&self, _: i32, _: i32, _: u32, out: &mut [f32]) {
        out.fill(1.0);
    }
}

/// The canvas the ants trial shows: 3301 px wide and 60 px high.
const CANVAS: [u32; 2] = [3301, 60];

/// The right and bottom ends of the ants `mask`, reduced `reduction` times, draws on the whole canvas at one pixel per
/// point in a room that holds it with pasteboard around it. The ants are device-pixel bands, painted as meshes.
fn ants_end(mask: Filled, reduction: u32) -> (f32, f32) {
    let [width, height] = CANVAS.map(f64::from);
    let mut fixture = Fixture::new(move |ui| {
        let view = CanvasView::new(ui.available_rect_before_wrap(), Point::new(width / 2.0, height / 2.0), 1.0);
        Canvas::new("Document")
            .view(view)
            .document(craft_behavior::kurbo::Size::new(width, height))
            .show(ui, |ui, body| SelectionOverlay::new("Selection", &mask).reduction(reduction).preview(PreviewMode::None).show_at(ui, body.rect));
    });
    fixture.room(egui::vec2(3500.0, 200.0));
    fixture.settle();
    let points: Vec<egui::Pos2> = fixture
        .frame_shapes()
        .iter()
        .flat_map(|shape| match shape {
            egui::Shape::Mesh(mesh) => mesh.vertices.iter().map(|vertex| vertex.pos).collect(),
            _ => Vec::new(),
        })
        .collect();
    assert!(!points.is_empty(), "the ants draw");
    (points.iter().map(|p| p.x).fold(f32::MIN, f32::max), points.iter().map(|p| p.y).fold(f32::MIN, f32::max))
}

#[trial]
fn the_border_ants_stay_on_the_canvas_of_a_reduced_copy() {
    // Reduced 3 times the canvas is 1101 × 20 pixels, which scaled up are 3303 × 60: the ants still end where the
    // full-size selection's do, on the canvas's border and not 2 px past it.
    let (right, bottom) = ants_end(Filled([1101, 20]), 3);
    let (full_right, full_bottom) = ants_end(Filled(CANVAS), 1);
    assert!((right - full_right).abs() < 0.01, "the ants end on the right border: {right} against {full_right}");
    assert!((bottom - full_bottom).abs() < 0.01, "and on the bottom border: {bottom} against {full_bottom}");
}
