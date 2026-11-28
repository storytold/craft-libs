//! The story canvas: the story document's ground is document pixels under the story's picture, so its squares scale
//! with the view.

use craft_behavior::PixelSize;
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::primitives::Revision;
use craft_ui::testing::canvas_story::{StoryDocument, on_canvas_view, on_ground};
use craft_ui::testing::{Fixture, StoryOutput, trial};
use egui::{Color32, ColorImage, Rect, Vec2};

/// The picture's colour where it is not transparent.
const INK: Color32 = Color32::from_rgb(200, 30, 30);

/// A picture 240 pixels wide: transparent on its left 160, [`INK`] on the rest.
struct Half;

impl RasterSource for Half {
    fn size(&self) -> PixelSize {
        PixelSize::new(240, 80)
    }

    fn revision(&self) -> Revision {
        Revision::new(1)
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        Some(PixelRect::default())
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        let width = out.size[0];
        for (index, pixel) in out.pixels.iter_mut().enumerate() {
            let x = region.x0 + (index % width) as i32 * factor as i32;
            *pixel = if x >= 160 { INK } else { Color32::TRANSPARENT };
        }
    }
}

#[trial]
fn the_ground_shows_in_document_pixels_where_the_picture_is_clear() {
    let shown = on_ground(&Half);
    for factor in [1, 2] {
        let mut out = ColorImage::filled([240 / factor as usize, 80 / factor as usize], Color32::TRANSPARENT);
        shown.read(PixelRect::new(0, 0, 240, 80), factor, &mut out);
        let at = |x: usize| out.pixels[x / factor as usize];
        assert_ne!(at(38), at(42), "a ground square ends at 40 document pixels at every texel size (factor {factor})");
        assert_eq!(at(2), at(82), "squares two apart match (factor {factor})");
        assert!(at(2).a() == u8::MAX && at(42).a() == u8::MAX, "the ground is opaque (factor {factor})");
        assert_eq!(at(200), INK, "the picture covers the ground where it has pixels (factor {factor})");
    }
}

/// The shapes a story canvas paints for the story document at `percent`, and the pixels per point.
fn shown_at(percent: f64) -> (Vec<Rect>, f32) {
    let mut fixture = Fixture::new(move |ui| {
        let _ = on_canvas_view(
            ui,
            |room, ppp| StoryDocument::STANDARD.view(room, ppp, percent),
            Some(&StoryDocument::STANDARD),
            |_, _, _| StoryOutput::default(),
        );
    });
    fixture.room(Vec2::new(1600.0, 1000.0));
    fixture.settle();
    (fixture.shape_bounds().to_vec(), fixture.ctx().pixels_per_point())
}

#[trial]
fn the_story_document_scales_with_the_view_as_one_picture() {
    let painted = |percent: f64| {
        let (shapes, ppp) = shown_at(percent);
        let width = 640.0 * percent as f32 / 100.0 / ppp;
        let documents =
            shapes.iter().filter(|rect| (rect.width() - width).abs() < 1.0 && (rect.height() - width * 400.0 / 640.0).abs() < 1.0).count();
        (documents, shapes.len())
    };
    let (near, near_shapes) = painted(25.0);
    let (far, far_shapes) = painted(50.0);
    assert!(near == 1 && far == 1, "the document and its ground are one picture at the view's size: {near} and {far}");
    assert_eq!(near_shapes, far_shapes, "the ground is part of the document's picture, not shapes per square");
}
