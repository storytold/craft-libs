//! The transform preview: its maps, its texture levels and its styled composite.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_host::{AppInfo, Host, ManualJobs};
use craft_imaging::fold_order::cell_order;
use craft_imaging::ui::transform_preview::{
    PreviewDisplay, PreviewMap, PreviewPixels, PreviewTextures, StylePlan, StyledBudget, StyledContent, StyledFrame, StyledPreview, StyledSource,
    TextureChoice, TransformPreview,
};
use craft_imaging::ui::warp::WarpMesh;
use craft_ui::canvas::{Canvas, PixelRect};
use craft_ui::contract::{MemoProps, MemoView, PlainData, Show as _, ShowContainer as _, Tracked};
use craft_ui::primitives::Serials;
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
    let textures = PreviewTextures::new(&mut Serials::default(), "t", PreviewPixels::new(solid([2, 2], Color32::from_gray(u8::MAX)), [1.0, 1.0]));
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

#[trial]
fn source_clip_moves_with_the_preview_and_preserves_texture_coordinates() {
    let clip = [pt(0.0, 0.0), pt(1.0, 0.0), pt(1.0, 0.5), pt(0.0, 0.5)];
    let moved = [pt(20.0, 40.0), pt(120.0, 40.0), pt(120.0, 140.0), pt(20.0, 140.0)];
    let mesh = PreviewMap::quad(moved).build(1.0, [0.8, 0.6]).clipped_to_source(clip, [0.8, 0.6]);
    assert!(!mesh.indices.is_empty());
    for &index in &mesh.indices {
        let p = mesh.points[index as usize];
        let uv = mesh.uv[index as usize];
        assert!(p.y <= 90.001, "{p:?}");
        assert!((f64::from(uv[1]) / 0.6 - (p.y - 40.0) / 100.0).abs() < 1e-5);
    }
}

/// A clipped preview retains texture coordinates where its frame cuts the image.
#[trial]
fn a_preview_clipped_to_a_frame_keeps_its_texture_coordinates() {
    let quad = [pt(25.0, 20.0), pt(75.0, 20.0), pt(75.0, 80.0), pt(25.0, 80.0)];
    let mesh = PreviewMap::quad(unit_quad()).build(1.0, [1.0, 1.0]).clipped_to_quad(quad);
    assert!(!mesh.indices.is_empty());
    for (p, uv) in mesh.points.iter().zip(&mesh.uv) {
        assert!(p.x >= 24.999 && p.x <= 75.001 && p.y >= 19.999 && p.y <= 80.001, "{p:?}");
        assert!((f64::from(uv[0]) - p.x / 100.0).abs() < 1e-5 && (f64::from(uv[1]) - p.y / 100.0).abs() < 1e-5);
    }
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

/// Twice the signed area of the cell of `row`-wide `points` whose top left vertex is `a`: its sign is the cell's facing.
fn facing(points: &[Point], a: usize, row: usize) -> f64 {
    let q = [a, a + 1, a + row + 1, a + row].map(|k| points[k]);
    (0..4).map(|i| q[i].x * q[(i + 1) % 4].y - q[(i + 1) % 4].x * q[i].y).sum()
}

// The renderer and the preview share one fold order: a cell the warp turns over comes last, so a fold shows inside out
// only where the picture folds; the others go by increasing displacement. A vertex sent to infinity goes under.
#[trial]
fn a_turned_over_cell_draws_last_and_an_unbounded_cell_goes_under() {
    // The top right anchor of a one-patch warp dragged far past the left edge folds the top right of the picture back.
    let source = DocRect::new(0.0, 0.0, 180.0, 120.0);
    let mut mesh = WarpMesh::identity(source, 1, 1);
    mesh.points[3].x -= 260.0;
    let built = PreviewMap::mesh(&mesh, source).build(1.0, [1.0, 1.0]);
    let row = (built.indices[2] - built.indices[0]) as usize - 1;
    let cells: Vec<usize> = built.indices.chunks(6).map(|cell| cell[0] as usize).collect();
    assert_eq!(cells.len(), (row - 1) * (built.points.len() / row - 1), "every cell is drawn once");
    let turned = |a: usize| facing(&built.points, a, row) * facing(&built.rest, a, row) < 0.0;
    let moved = |a: usize| [a, a + 1, a + row, a + row + 1].iter().map(|&k| (built.points[k] - built.rest[k]).hypot()).sum::<f64>();
    let turned_count = cells.iter().filter(|&&a| turned(a)).count();
    assert!(turned_count > 0 && turned_count < cells.len(), "the fold turns some cells over and not all: {turned_count} of {}", cells.len());
    for pair in cells.windows(2) {
        let (before, after) = (pair[0], pair[1]);
        assert!(turned(before) <= turned(after), "turned cells come after the others: {before} then {after}");
        if turned(before) == turned(after) {
            assert!(moved(before) <= moved(after) + 1e-9, "by increasing displacement: {before} then {after}");
        }
    }

    // A cell with a vertex sent to infinity or to NaN goes under the cells that moved a finite way.
    let rest = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [0.0, 1.0], [1.0, 1.0], [2.0, 1.0]];
    let mut pos = rest.map(|[x, y]| [x + 1.0, y]);
    pos[2] = [f64::INFINITY, 0.0];
    pos[5] = [f64::NAN, 1.0];
    assert_eq!(cell_order(&pos, &rest, 3), vec![1, 0]);

    // Mirrored cells go on top even when they moved less than the cells around them; a mirrored placement as a whole
    // turns nothing over.
    let rest = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0], [0.0, 1.0], [1.0, 1.0], [2.0, 1.0], [3.0, 1.0]];
    let pos = [[-9.0, 0.0], [1.0, 0.0], [2.2, 0.0], [1.6, 0.0], [-9.0, 1.0], [1.0, 1.0], [2.2, 1.0], [1.6, 1.0]];
    assert_eq!(cell_order(&pos, &rest, 4), vec![1, 0, 2]);
    let mirrored = rest.map(|[x, y]| [-x, y]);
    assert_eq!(cell_order(&mirrored, &mirrored, 4), vec![0, 1, 2]);

    // Mismatched lengths and a row under 2 give no cells.
    assert!(cell_order(&pos, &rest[..5], 4).is_empty());
    assert!(cell_order(&pos, &rest, 1).is_empty());
}

// The work stays within its budget for any box, and a box that cannot be composited is refused.
#[trial]
fn a_style_plan_stays_within_its_budget_at_screen_size_and_refuses_hostile_boxes() {
    let reach = |scale: f64| (4.0 * scale).round() as i32;
    let area = |plan: &StylePlan| f64::from(plan.content.width()) * f64::from(plan.content.height());
    let budget = StyledBudget::default();

    // A box as big as a huge canvas at 100 %: the scale drops until the content fits, and no further.
    let huge = [pt(0.0, 0.0), pt(12_000.0, 0.0), pt(12_000.0, 8_000.0), pt(0.0, 8_000.0)];
    for pixels in [budget.pixels(100.0, true), budget.pixels(100.0, false)] {
        let plan = StylePlan::new(huge, [12_000, 8_000], 1.0, pixels, reach).expect("planned");
        assert!(area(&plan) <= pixels && area(&plan) > pixels * 0.5, "{} of {pixels}", area(&plan));
        assert!(plan.scale < 1.0);
    }

    // Hostile boxes: not finite, collapsed, astronomically large or far away.
    let ok = [pt(60.0, 60.0), pt(100.0, 60.0), pt(100.0, 100.0), pt(60.0, 100.0)];
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut quad = ok;
        quad[2].x = bad;
        assert!(StylePlan::new(quad, [200, 200], 1.0, StyledBudget::MAX_PIXELS, reach).is_none());
    }
    assert!(StylePlan::new([pt(80.0, 80.0); 4], [200, 200], 1.0, StyledBudget::MAX_PIXELS, reach).is_none(), "a collapsed box");
    assert!(
        StylePlan::new([pt(-90.0, 0.0), pt(-50.0, 0.0), pt(-50.0, 20.0), pt(-90.0, 20.0)], [200, 200], 1.0, StyledBudget::MAX_PIXELS, reach)
            .is_none()
    );
    for quad in [
        [pt(-1e300, -1e300), pt(1e300, -1e300), pt(1e300, 1e300), pt(-1e300, 1e300)],
        [pt(1e18, 1e18), pt(1e18 + 40.0, 1e18), pt(1e18 + 40.0, 1e18 + 40.0), pt(1e18, 1e18 + 40.0)],
        [pt(0.0, 0.0), pt(1e12, 0.0), pt(1e12, 1.0), pt(0.0, 1.0)],
    ] {
        if let Some(plan) = StylePlan::new(quad, [200, 200], 1.0, StyledBudget::MAX_PIXELS, reach) {
            assert!(area(&plan) <= StyledBudget::MAX_PIXELS, "{plan:?}");
        }
    }
    // A zoom with no meaning counts as 100 %, and a budget with none stays within the most work.
    for zoom in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(StylePlan::new(ok, [200, 200], zoom, StyledBudget::MAX_PIXELS, reach).map(|plan| plan.scale), Some(1.0), "zoom {zoom}");
    }
    for pixels in [f64::NAN, f64::INFINITY, -5.0, 0.0] {
        if let Some(plan) = StylePlan::new(ok, [200, 200], 1.0, pixels, reach) {
            assert!(area(&plan) <= StyledBudget::MAX_PIXELS, "{pixels}");
        }
    }

    // At 50 % the work is at screen resolution.
    let plan = StylePlan::new(ok, [200, 200], 0.5, StyledBudget::MAX_PIXELS, reach).expect("planned");
    assert_eq!(plan.scale, 0.5);
    assert_eq!(plan.bounds, PixelRect::new(30, 30, 50, 50));

    // The part past the canvas edge is not shown but still shapes the style: the content reaches past the edge.
    let hanging = [pt(-20.0, 80.0), pt(20.0, 80.0), pt(20.0, 100.0), pt(-20.0, 100.0)];
    let plan = StylePlan::new(hanging, [200, 200], 1.0, StyledBudget::MAX_PIXELS, reach).expect("planned");
    assert_eq!(plan.shown.x0, 0);
    assert!(plan.content.x0 < 0, "{plan:?}");
}

// A monitor conversion changes the uploaded texture and leaves the stored levels as they were: the styled composite and
// the mips read the document's pixels.
#[trial]
fn a_monitor_conversion_changes_only_the_shown_upload() {
    let ctx = egui::Context::default();
    let raw = solid([2, 2], Color32::from_rgb(140, 89, 51));
    let textures = PreviewTextures::new(&mut Serials::default(), "monitor", PreviewPixels::new(raw.clone(), [1.0, 1.0]));
    // The trial stands in for the renderer: it takes the texture deltas it reads and applies none.
    let uploaded = |id: egui::TextureId| {
        let mut delta = ctx.tex_manager().write().take_delta();
        let texel = delta.set.get(&id).and_then(|sets| sets.first()).map(|set| match &set.image {
            egui::ImageData::Color(image) => image.pixels[0],
        });
        delta.clear();
        texel
    };
    let (plain, _) = textures.pick(&ctx, TextureChoice::new(0, false), None).expect("uploaded");
    assert_eq!(uploaded(plain), Some(raw.pixels[0]), "without a conversion the document's texels go up");
    let green = PreviewDisplay::new(7, |image| ColorImage::new(image.size, vec![Color32::GREEN; image.pixels.len()]));
    let (shown, _) = textures.pick(&ctx, TextureChoice::new(0, false), Some(&green)).expect("uploaded");
    assert_eq!(uploaded(shown), Some(Color32::GREEN), "the monitor's texels go up");
    assert_eq!(textures.level_image(0).expect("level 0").0.pixels, raw.pixels, "the stored level stays the document's");
}

// A warp that stretches a cell far gets more samples across it than a plain one: the stretch decides the sampling. A
// cell's samples follow the larger side of its extent on screen, so the layer is low enough that the narrow column's
// width decides them.
#[trial]
fn a_stretched_warp_cell_gets_samples_for_its_stretch() {
    let source = DocRect::new(0.0, 0.0, 900.0, 30.0);
    let plain = WarpMesh::identity_with_knots(source, vec![0.0, 0.45, 0.5, 1.0], vec![0.0, 1.0]).expect("knots");
    // The narrow column's right anchor and its handle pulled 300 to the left.
    let mut stretched = plain.clone();
    let width = stretched.width();
    for j in 0..stretched.height() {
        for i in [5, 6] {
            stretched.points[j * width + i].x -= 300.0;
        }
    }
    // The samples across the narrow column (0.45..0.5 of the width), read from the top row's rest positions.
    let narrow = |mesh: &WarpMesh| {
        let built = PreviewMap::mesh(mesh, source).build(1.0, [1.0, 1.0]);
        built.rest.iter().filter(|rest| rest.y == source.y0 && (405.0..450.0).contains(&rest.x)).count()
    };
    assert!(narrow(&stretched) > narrow(&plain), "{} samples against {}", narrow(&stretched), narrow(&plain));
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
    PreviewTextures::new(&mut Serials::default(), "styled", PreviewPixels::new(solid([4, 4], Color32::from_rgb(200, 100, 50)), [1.0, 1.0]))
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

/// Previews made one after another, as Free Transform sessions are, each composited in its own frame and then
/// closed: the context keeps fewer composites than there were previews, since the closed ones are evicted when the
/// store needs the room.
#[trial]
fn closed_styled_previews_leave_their_composites_to_eviction() {
    const PREVIEWS: usize = 600;
    let (mut fixture, ctx) = context();
    let (textures, source) = (styled_textures(), Fake::new(2));
    let allocated = || ctx.tex_manager().read().num_allocated();
    let before = allocated();
    for n in 0..PREVIEWS {
        let preview = StyledPreview::new(format!("session-{n}"));
        assert!(preview.composite(&ctx, &frame(&textures, square(10.0, 10.0, 8.0), 1.0), &source, None).is_some());
        fixture.idle(1);
    }
    let kept = allocated() - before;
    assert!(kept < PREVIEWS, "{kept} composites kept for {PREVIEWS} closed previews");
}

// The budget is 30 ms dragging and 120 ms at rest over the cost per pixel plus the resample's, clamped to 0.25 to 8
// million pixels.

// With nearest interpolation every zoom and scale gives level 0 with nearest filtering.

/// A preview with the scale it is painted at.
type PaintedPreview = (Rc<PreviewTextures>, f32);
/// The texture and size a painted preview showed.
type ShownPreview = (egui::TextureId, [f32; 2]);

/// What the next frames paint: the previews, each at its scale, and the display every one is converted for.
#[derive(Default, PlainData)]
struct Painting {
    #[plain_data(assume = "the previews' textures are a cache: what they show follows their pixels, which do not change")]
    previews: Vec<PaintedPreview>,
    #[plain_data(assume = "a display is replaced, never changed: its key follows its conversion")]
    display: Option<PreviewDisplay>,
}

/// What each painted preview shows: the texture and uv extent `PreviewTextures::pick` gives.
fn picks(ui: &egui::Ui, painting: &Painting) -> Vec<Option<ShownPreview>> {
    let display = painting.display.as_ref();
    painting.previews.iter().map(|(textures, scale)| textures.pick(ui.ctx(), PreviewTextures::choose(1.0, *scale, false), display)).collect()
}

/// A memoized view of the painting.
#[derive(MemoProps)]
struct Previews<'a> {
    painting: &'a Tracked<Painting>,
}

fn previews(props: &mut Previews<'_>, ui: &mut egui::Ui) -> Vec<Option<ShownPreview>> {
    picks(ui, props.painting)
}

/// Previews painted every frame of a fixture whose host runs jobs only when a trial says (upstream's 5 × 3 white
/// image).
struct Mips {
    fixture: Fixture,
    jobs: Arc<ManualJobs>,
    /// What the next frames paint.
    painting: Rc<RefCell<Tracked<Painting>>>,
    /// What each painted preview showed in the last frame.
    shown: Rc<RefCell<Vec<Option<ShownPreview>>>>,
    /// The previews' uploads in the last frame, read in the frame.
    uploads: Rc<Cell<usize>>,
}

fn white(name: &str) -> Rc<PreviewTextures> {
    Rc::new(PreviewTextures::new(&mut Serials::default(), name, PreviewPixels::new(solid([5, 3], Color32::WHITE), [1.0, 1.0])))
}

impl Mips {
    fn new() -> Self {
        Self::with(false)
    }

    /// The previews painted inside a memoized view, which replays its last run while its painting is unchanged and no
    /// texture it shows is loading.
    fn memoized() -> Self {
        Self::with(true)
    }

    fn with(memoized: bool) -> Self {
        let jobs = Arc::new(ManualJobs::new());
        let painting = Rc::new(RefCell::new(Tracked::new(Painting::default())));
        let shown = Rc::new(RefCell::new(Vec::new()));
        let uploads = Rc::new(Cell::new(0));
        let (paint, seen, counted) = (Rc::clone(&painting), Rc::clone(&shown), Rc::clone(&uploads));
        let mut fixture = Fixture::new(move |ui| {
            let painting = paint.borrow();
            let shows = if memoized {
                MemoView::new("previews", Previews { painting: &painting }, previews).show(ui).unwrap_or_default()
            } else {
                picks(ui, &painting)
            };
            *seen.borrow_mut() = shows;
            counted.set(PreviewTextures::statistics(ui.ctx()).uploads);
        });
        fixture.host(Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).jobs(jobs.clone()).build()));
        fixture.settle();
        Mips { fixture, jobs, painting, shown, uploads }
    }

    /// One frame painting `previews`; a change of the previews or their scales is an edit of the painting.
    fn paint(&mut self, previews: &[(&Rc<PreviewTextures>, f32)]) {
        let next: Vec<_> = previews.iter().map(|(textures, scale)| (Rc::clone(textures), *scale)).collect();
        let changed = {
            let painting = self.painting.borrow();
            let painted = &painting.previews;
            painted.len() != next.len() || painted.iter().zip(&next).any(|(a, b)| !Rc::ptr_eq(&a.0, &b.0) || a.1 != b.1)
        };
        if changed {
            self.painting.borrow_mut().edit(|painting| painting.previews = next);
        }
        self.fixture.frame(Vec::new());
    }

    /// Converts every painted preview for `display` from the next frame on.
    fn display(&mut self, display: PreviewDisplay) {
        self.painting.borrow_mut().edit(|painting| painting.display = Some(display));
    }

    /// The texture and uv extent the `at`th preview showed.
    fn shown(&self, at: usize) -> (egui::TextureId, [f32; 2]) {
        self.shown.borrow()[at].expect("a texture")
    }

    fn uploads(&self) -> usize {
        self.uploads.get()
    }
}

fn near_uv(uv: [f32; 2], want: [f32; 2]) -> bool {
    (uv[0] - want[0]).abs() < 1e-6 && (uv[1] - want[1]).abs() < 1e-6
}

/// P91 (#13382): the frame that first needs a mip level builds none on the UI thread; level 0 stands in, the frames
/// while its job waits do no work for it, and the landing frame uploads it once. In a memoized view the frame after
/// the job lands runs the view again with no other input and shows the level.
#[trial]
fn a_mip_level_first_needed_mid_drag_is_built_off_the_ui_thread() {
    for memoized in [false, true] {
        mips_first_needed_mid_drag(if memoized { Mips::memoized() } else { Mips::new() });
    }
}

fn mips_first_needed_mid_drag(mut mips: Mips) {
    let t = white("drag");
    mips.paint(&[(&t, 1.0)]);
    let (level0, _) = mips.shown(0);
    let built = t.levels_built();
    mips.paint(&[(&t, 0.3)]);
    assert_eq!(mips.shown(0).0, level0, "level 0 stands in");
    assert_eq!(t.levels_built(), built, "the frame built no level");
    assert_eq!(mips.jobs.waiting(), 1, "one job builds level 1");
    for _ in 0..3 {
        mips.paint(&[(&t, 0.3)]);
        assert_eq!(mips.jobs.waiting(), 1, "a frame while the job waits queues no second one");
        assert_eq!(t.levels_built(), built, "and builds no level");
        assert_eq!(mips.uploads(), 0, "and uploads nothing");
    }
    mips.jobs.run_all();
    mips.paint(&[(&t, 0.3)]);
    let (level1, uv) = mips.shown(0);
    assert_ne!(level1, level0, "level 1 has its own texture");
    assert!(near_uv(uv, [5.0 / 6.0, 3.0 / 4.0]), "level 1's extent: {uv:?}");
    assert_eq!(mips.uploads(), 1, "the landing frame uploads one texture");
    mips.paint(&[(&t, 0.3)]);
    assert_eq!((mips.shown(0).0, mips.uploads()), (level1, 0), "a later frame uploads nothing");
}

#[trial]
fn the_closest_finer_level_stands_in_while_a_level_builds() {
    let mut mips = Mips::new();
    let t = white("finer");
    mips.paint(&[(&t, 1.0)]);
    mips.paint(&[(&t, 0.3)]);
    mips.jobs.run_all();
    mips.paint(&[(&t, 0.3)]);
    let level1 = mips.shown(0);
    mips.paint(&[(&t, 0.1)]);
    assert_eq!(mips.shown(0), level1, "level 1 stands in for level 3");
    mips.jobs.run_all();
    mips.paint(&[(&t, 0.1)]);
    let (level3, uv) = mips.shown(0);
    assert_ne!(level3, level1.0);
    assert!(near_uv(uv, [5.0 / 8.0, 3.0 / 8.0]), "level 3's extent: {uv:?}");
}

#[trial]
fn a_level_with_nothing_finer_uploaded_for_the_display_is_built_at_once() {
    let mut mips = Mips::new();
    let t = white("at-once");
    mips.paint(&[(&t, 0.3)]);
    assert_eq!(mips.jobs.waiting(), 0, "a first frame at level 1 builds it in the frame");
    assert!(near_uv(mips.shown(0).1, [5.0 / 6.0, 3.0 / 4.0]), "level 1 shows at once");
    mips.paint(&[(&t, 1.0)]);
    mips.display(PreviewDisplay::new(7, ColorImage::clone));
    mips.paint(&[(&t, 0.3)]);
    assert_eq!(mips.jobs.waiting(), 0, "nothing is uploaded for the new display, so level 1 is built in the frame");
    assert!(near_uv(mips.shown(0).1, [5.0 / 6.0, 3.0 / 4.0]));
}

/// A preview closed while its level builds neither uploads the level nor keeps it; one still painted does.
#[trial]
fn levels_finishing_after_the_box_closed_are_not_uploaded_and_while_it_is_open_are() {
    let mut entries = Vec::new();
    for open in [false, true] {
        let mut mips = Mips::new();
        let (t, other) = (white("closing"), white("other"));
        mips.paint(&[(&t, 1.0)]);
        mips.paint(&[(&t, 0.3)]);
        for _ in 0..2 {
            if open { mips.paint(&[(&t, 0.3)]) } else { mips.paint(&[]) }
        }
        mips.jobs.run_all();
        // The other preview's second frame asks the loader, which stores or drops the finished level.
        for scale in [1.0, 0.3] {
            let mut previews = vec![(&other, scale)];
            if open {
                previews.push((&t, 0.3));
            }
            mips.paint(&previews);
        }
        entries.push(PreviewTextures::statistics(mips.fixture.ctx()).entries);
    }
    assert_eq!(entries[1], entries[0] + 1, "only the open preview's level is uploaded: {entries:?}");
}

/// A new preview under an old name with other pixels shows its own pixels, not the old textures.
#[trial]
fn a_new_preview_under_the_same_name_shows_its_own_pixels() {
    let ctx = egui::Context::default();
    let uploaded = |id: egui::TextureId| {
        let mut delta = ctx.tex_manager().write().take_delta();
        let texel = delta.set.get(&id).and_then(|sets| sets.first()).map(|set| match &set.image {
            egui::ImageData::Color(image) => image.pixels[0],
        });
        delta.clear();
        texel
    };
    let mut serials = Serials::default();
    let first = PreviewTextures::new(&mut serials, "same", PreviewPixels::new(solid([2, 2], Color32::WHITE), [1.0, 1.0]));
    let (id, _) = first.pick(&ctx, TextureChoice::new(0, false), None).expect("uploaded");
    assert_eq!(uploaded(id), Some(Color32::WHITE));
    drop(first);
    let second = PreviewTextures::new(&mut serials, "same", PreviewPixels::new(solid([2, 2], Color32::BLACK), [1.0, 1.0]));
    let (id, _) = second.pick(&ctx, TextureChoice::new(0, false), None).expect("uploaded");
    assert_eq!(uploaded(id), Some(Color32::BLACK), "the new pixels go up");
}
