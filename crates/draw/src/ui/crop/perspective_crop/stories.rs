//! Stories of the perspective crop quad, and of the Perspective Crop tool run live: with no quad a drag draws a box
//! and four clicks place the corners; then the quad takes its own gestures, and Enter or Esc ends the crop.

use super::super::PerspectiveCropTool;
use super::{CORNER, PerspectiveCropQuad, PerspectiveQuad};
use craft_behavior::CanvasView;
use craft_behavior::kurbo::{BezPath, Point};
use craft_ui::canvas::RasterSource as _;
use craft_ui::contract::{Bind, Show as _, Validity};
use craft_ui::runtime::env::Message;
use craft_ui::testing::canvas_story::{OUTLINE, StoryDocument, StoryPicture, StoryShape, on_picture, paint_shapes};
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, ToolStory, story, tool_story};
use egui::{Color32, Ui};

/// A skewed page marked from corner to corner: a quad that can be committed.
const PAGE: [Point; 4] = [Point::new(250.0, 110.0), Point::new(430.0, 130.0), Point::new(400.0, 290.0), Point::new(220.0, 250.0)];
/// The same corners with the last one pulled in: the quad is not convex.
const BENT: [Point; 4] = [Point::new(250.0, 110.0), Point::new(430.0, 130.0), Point::new(400.0, 290.0), Point::new(380.0, 150.0)];

/// The story document with the page the quad is marked on, as the camera saw it: blue with a dark grey edge.
fn paper() -> StoryPicture<impl Fn(Point) -> Color32> {
    let mut outline = BezPath::new();
    outline.move_to(PAGE[0]);
    for corner in &PAGE[1..] {
        outline.line_to(*corner);
    }
    outline.close_path();
    let page = StoryShape::filled(&outline, Color32::from_rgb(70, 130, 180)).outlined(OUTLINE, Color32::from_gray(40));
    let size = StoryDocument::STANDARD.size();
    StoryPicture::new(size.width, size.height, move |p| paint_shapes(std::slice::from_ref(&page), Color32::TRANSPARENT, p))
}

/// Shows the quad `quad` builds and shows as the only canvas control of a canvas over the page. The canvas body runs
/// more than once per pass, so `quad` builds it on every run.
fn on_canvas<R: Into<StoryOutput>>(ui: &mut Ui, mut quad: impl FnMut(&mut Ui, egui::Rect) -> R) -> StoryOutput {
    on_picture(ui, &paper(), |ui, at, _| quad(ui, at))
}

#[story(PerspectiveCropQuad<'static>, covers = [Default], drive = Drive::Drag(Grip::part(CORNER.at(0))))]
fn skewed_page(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let quad = data.value("quad", Some(PAGE));
    on_canvas(ui, |ui, at| PerspectiveCropQuad::new("Perspective crop", Bind::bound(&mut *quad)).show_at(ui, at).edit)
}

#[story(PerspectiveCropQuad<'static>, covers = [Invalid], drive = Drive::Drag(Grip::part(CORNER.at(0))))]
fn bent_page(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let quad = data.value("quad", Some(BENT));
    on_canvas(ui, |ui, at| {
        let bind = Bind::bound(&mut *quad).check(|quad: &PerspectiveQuad| match quad {
            Some(quad) if !super::is_valid(quad) => Validity::Refuse(Message::new("The corners do not make a convex quad")),
            _ => Validity::Ok,
        });
        PerspectiveCropQuad::new("Perspective crop", bind).show_at(ui, at).edit
    })
}

/// The tool from no quad over the page. The quad is the story's document: the tool keeps it as its own and follows
/// the document when Reset puts it back; Enter or Esc on the quad ends the crop, which clears it.
#[story(PerspectiveCropQuad<'static>, covers = [], drive = Drive::Never)]
fn drawing_a_quad(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let picture = paper();
    let body = |ui: &mut Ui, at: egui::Rect, _: &CanvasView, tool: &mut PerspectiveCropTool, quad: &PerspectiveQuad| {
        if tool.quad() != quad {
            tool.set(*quad);
        }
        let shown = PerspectiveCropQuad::new("Perspective crop", Bind::bound(tool.quad_mut())).show_at(ui, at);
        if shown.apply || shown.discard {
            tool.clear();
        }
    };
    tool_story(ui, data, ToolStory::new("Document", &picture), PerspectiveCropTool::new, None, body, |run| {
        let _ = run.host.run(run.tool);
        let _ = run.tool.take_events();
        *run.document = *run.tool.quad();
        StoryOutput::default()
    })
}
