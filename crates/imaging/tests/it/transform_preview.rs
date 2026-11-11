//! The transform preview: its maps, its texture levels and its styled composite.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_behavior::CanvasView;
use craft_behavior::kurbo::Point;
use craft_imaging::ui::transform_preview::{
    PreviewDisplay, PreviewMap, PreviewPixels, PreviewTextures, StylePlan, StyledContent, StyledFrame, StyledPreview, StyledSource, TransformPreview,
};
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::testing::{Fixture, trial};
use egui::{Color32, ColorImage, Shape};

fn pt(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

fn solid(size: [usize; 2], color: Color32) -> ColorImage {
    ColorImage::new(size, vec![color; size[0] * size[1]])
}

/// The meshes `TransformPreview` paints for `map` in the fixture's canvas under `rotation` degrees, and the view.
fn shown(map: fn() -> PreviewMap<'static>, opacity: f32, clip: Option<[u32; 2]>, rotation: f64) -> (Vec<egui::Mesh>, CanvasView) {
    let meshes = Rc::new(RefCell::new(Vec::new()));
    let seen_view = Rc::new(Cell::new(None));
    let (sink, view_sink) = (Rc::clone(&meshes), Rc::clone(&seen_view));
    let textures = PreviewTextures::new("t", PreviewPixels::new(solid([2, 2], Color32::from_gray(u8::MAX)), [1.0, 1.0]));
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let view = CanvasView::new(room, pt(100.0, 100.0), 1.0).rotation(rotation);
        view_sink.set(Some(view));
        Canvas::new("Document").view(view).show(ui, |ui, body| {
            let mut preview = TransformPreview::new("Moving layer", &textures).map(map()).opacity(opacity);
            if let Some(clip) = clip {
                preview = preview.clip(clip);
            }
            preview.show_at(ui, body.rect)
        });
        let layer = ui.layer_id();
        *sink.borrow_mut() = ui.ctx().graphics(|g| {
            g.get(layer)
                .map(|list| {
                    list.all_entries()
                        .filter_map(|clipped| match &clipped.shape {
                            Shape::Mesh(mesh) => Some((**mesh).clone()),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default()
        });
    });
    fixture.settle();
    let view = seen_view.get().expect("the canvas was shown");
    (meshes.take(), view)
}

fn unit_quad() -> [Point; 4] {
    [pt(0.0, 0.0), pt(100.0, 0.0), pt(100.0, 100.0), pt(0.0, 100.0)]
}

// A Distort dragged past the horizon leaves the cells with a corner on the far side out of the mesh; the cells in
// front stay.
#[trial]
fn cells_past_the_horizon_are_left_out_of_a_distorted_quad() {
    let flat = PreviewMap::quad(unit_quad()).build(1.0, [1.0, 1.0]);
    // 24 × 24 cells, two triangles each.
    assert_eq!(flat.indices.len(), 24 * 24 * 6);
    // A dart (the bottom right corner dragged inside the box): the horizon crosses the square.
    let dart = PreviewMap::quad([pt(0.0, 0.0), pt(100.0, 0.0), pt(30.0, 30.0), pt(0.0, 100.0)]).build(1.0, [1.0, 1.0]);
    assert!(!dart.indices.is_empty() && dart.indices.len() < flat.indices.len(), "{}", dart.indices.len());
    assert!(dart.indices.iter().all(|&k| dart.points[k as usize].x.is_finite()));
}

// A quad hanging past the canvas edge draws nothing past it, also under a turned view.
#[trial]
fn pixels_past_the_canvas_edge_are_clipped_in_document_space() {
    fn quad() -> PreviewMap<'static> {
        PreviewMap::quad([pt(-50.0, -50.0), pt(150.0, -50.0), pt(150.0, 150.0), pt(-50.0, 150.0)])
    }
    for rotation in [0.0, 30.0] {
        let (meshes, view) = shown(quad, 1.0, Some([100, 100]), rotation);
        assert_eq!(meshes.len(), 1);
        for vertex in &meshes[0].vertices {
            let at = view.to_doc(vertex.pos);
            assert!(at.x > -1e-3 && at.y > -1e-3 && at.x < 100.001 && at.y < 100.001, "{at:?}");
        }
        // The covered area is the whole canvas (100 × 100 document units).
        let area: f32 = meshes[0]
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let [a, b, c] = [t[0], t[1], t[2]].map(|k| meshes[0].vertices[k as usize].pos);
                ((b - a).x * (c - a).y - (b - a).y * (c - a).x).abs() / 2.0
            })
            .sum();
        assert!((area - 10_000.0).abs() < 5.0, "{area}");
    }
    // Without a clip the whole quad is drawn.
    let (open, view) = shown(quad, 1.0, None, 0.0);
    assert!(open[0].vertices.iter().any(|v| view.to_doc(v.pos).x < 0.0));
}

// An identity mesh at zoom 1 samples each patch at the minimum.

// A warp mesh's rest is the unwarped frame, also when only the corner controls moved: pulling the top-left corner out
// leaves its rest on the frame's corner, so the pull ranks as a displacement.

// Opacity 0.5 tints every vertex white at alpha 128.

// A pin map draws the solver's triangles in its order with texels at the rest positions; a function map draws a
// 40 × 40 grid through the function.

// The quad's scale for the texture level is its longest side over the source frame's.

/// A source with a fixed reach (in work pixels, whatever the scale) that counts its compositions.
struct Fake {
    revision: Cell<u64>,
    reach: i32,
    composed: Cell<u32>,
}

impl Fake {
    fn new(reach: i32) -> Self {
        Self { revision: Cell::new(1), reach, composed: Cell::new(0) }
    }
}

impl StyledSource for Fake {
    fn revision(&self) -> u64 {
        self.revision.get()
    }
    fn pixel_cost_ns(&self) -> f64 {
        20.0
    }
    fn reach(&self, _scale: f64) -> i32 {
        self.reach
    }
    fn compose(&self, plan: &StylePlan, _content: &StyledContent) -> Option<ColorImage> {
        self.composed.set(self.composed.get() + 1);
        let size = [plan.shown.width() as usize, plan.shown.height() as usize];
        Some(solid(size, Color32::from_gray(u8::MAX)))
    }
}

fn square(x: f64, y: f64, size: f64) -> [Point; 4] {
    [pt(x, y), pt(x + size, y), pt(x + size, y + size), pt(x, y + size)]
}

fn frame(textures: &PreviewTextures, quad: [Point; 4], zoom: f32) -> StyledFrame<'_> {
    StyledFrame::new(textures, quad, [100, 100], zoom)
}

fn styled_textures() -> PreviewTextures {
    PreviewTextures::new("styled", PreviewPixels::new(solid([4, 4], Color32::from_rgb(200, 100, 50)), [1.0, 1.0]))
}

fn context() -> (Fixture, egui::Context) {
    let fixture = Fixture::new(|_| {});
    let ctx = fixture.ctx().clone();
    (fixture, ctx)
}

// Two passes with the same box, zoom, revision, display key, level and filter build once; a change of any one builds
// again.
#[trial]
fn the_styled_frame_is_rebuilt_only_when_its_key_changes() {
    let (_fixture, ctx) = context();
    let textures = styled_textures();
    let source = Fake::new(2);
    let preview = StyledPreview::new("styled-frame");
    let box_a = square(10.0, 10.0, 40.0);
    assert!(preview.composite(&ctx, &frame(&textures, box_a, 1.0), &source, None).is_some());
    assert!(preview.composite(&ctx, &frame(&textures, box_a, 1.0), &source, None).is_some());
    assert_eq!(preview.builds(), 1);
    assert_eq!(source.composed.get(), 1);

    let mut builds = 1;
    let mut expect_build = |frame: &StyledFrame<'_>, source: &Fake, display: Option<&PreviewDisplay>, what: &str| {
        preview.composite(&ctx, frame, source, display);
        builds += 1;
        assert_eq!(preview.builds(), builds, "{what}");
    };
    expect_build(&frame(&textures, square(11.0, 10.0, 40.0), 1.0), &source, None, "moved box");
    expect_build(&frame(&textures, square(11.0, 10.0, 40.0), 0.5), &source, None, "zoom");
    source.revision.set(2);
    expect_build(&frame(&textures, square(11.0, 10.0, 40.0), 0.5), &source, None, "revision");
    let display = PreviewDisplay::new(7, Clone::clone);
    expect_build(&frame(&textures, square(11.0, 10.0, 40.0), 0.5), &source, Some(&display), "display key");
    let mut nearest = frame(&textures, square(11.0, 10.0, 40.0), 0.5);
    nearest.nearest = true;
    expect_build(&nearest, &source, Some(&display), "filter");
}

// The budget is 30 ms dragging and 120 ms at rest over the cost per pixel plus the resample's, clamped to 0.25 to 8
// million pixels.

// With nearest interpolation every zoom and scale gives level 0 with nearest filtering.
