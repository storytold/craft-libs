//! Stories of the transform box: Free Transform with its reference point, the Move tool's controls, Distort, the
//! Ellipse tool's centre mark, the rotate handle and the Line tool's live-line controls, with box or free ends.

use super::{BOX, LINE_END, LineEnd, LineEnds, LineLook, ROTATE_HANDLE, RotateZone, TransformBox, TransformBoxOutput};
use craft_behavior::GesturePhase;
use craft_behavior::geometry::quad::{BoxPart, BoxRules, BoxState, TransformMode};
use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::testing::canvas_story::{OUTLINE, StoryDocument, StoryPicture, StoryShape, paint_shapes, show_document, show_quad};
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::{Color32, Ui, vec2};

/// The picture before any transform, in document units.
const PICTURE: DocRect = DocRect::new(200.0, 120.0, 440.0, 280.0);
/// The layer the box transforms, shown where the box is: a blue picture with a dark grey edge.
fn layer() -> StoryPicture<impl Fn(Point) -> Color32> {
    let (width, height) = (PICTURE.width(), PICTURE.height());
    let edge = StoryShape::line(&DocRect::new(0.0, 0.0, width, height), OUTLINE, Color32::from_gray(40));
    let fill = Color32::from_rgba_unmultiplied(70, 130, 180, 220);
    StoryPicture::new(width as u32, height as u32, move |p| paint_shapes(std::slice::from_ref(&edge), fill, p))
}

/// A box over `state` with the story's picture as its start rect.
fn free_box(state: Bind<'_, BoxState>) -> TransformBox<'_> {
    TransformBox::new("Free Transform", state, PICTURE, 0.0)
}

/// Shows the box `make` builds over the story's box as the only canvas control of a canvas.
fn on_canvas(ui: &mut Ui, data: &mut StoryData, make: impl for<'a> Fn(Bind<'a, BoxState>) -> TransformBox<'a>) -> StoryOutput {
    shown_on_canvas(ui, data, make).map(|output| StoryOutput::from(output.edit)).unwrap_or_default()
}

/// [`on_canvas`] with the box's whole output.
fn shown_on_canvas(ui: &mut Ui, data: &mut StoryData, make: impl for<'a> Fn(Bind<'a, BoxState>) -> TransformBox<'a>) -> Option<TransformBoxOutput> {
    let (state, state_at) = data.value_at("box", BoxState::from_rect(PICTURE));
    let room = craft_ui::testing::stage(ui);
    let view = craft_ui::testing::canvas_story::story_view(room, ui.pixels_per_point(), PICTURE.center(), 100.0);
    let layer = layer();
    let shown = Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
        show_document(ui, body.rect, &StoryDocument::STANDARD);
        show_quad(ui, body.rect, "Layer", state.quad, &layer);
        make(state_at.bind(&mut *state)).show_at(ui, body.rect)
    });
    shown.inner
}

#[story(TransformBox<'static>, covers = [Default], drive = Drive::Drag(Grip::part(BOX.at(BoxPart::Corner(2).index())).toward(vec2(1.0, 1.0))))]
fn free_transform(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data, |state| free_box(state).reference_point(true))
}

#[story(TransformBox<'static>, covers = [Default])]
fn move_tool_controls(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data, |state| free_box(state).rotate_zone(RotateZone::Band).inside_moves(false).reference_point(false))
}

#[story(TransformBox<'static>, covers = [Default])]
fn distort(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data, |state| free_box(state).rules(BoxRules::in_mode(TransformMode::Distort)))
}

/// The box of an upright paragraph frame: free sides, no rotation, no reference point.
#[story(TransformBox<'static>, covers = [Default])]
fn text_frame(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data, |state| TransformBox::text_frame("Text frame", state, PICTURE))
}

/// The same frame with its text overflowing: the lower right handle is enlarged and carries the plus.
#[story(TransformBox<'static>, covers = [Default])]
fn text_frame_overflow(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data, |state| TransformBox::text_frame("Text frame", state, PICTURE).overflow(true))
}

/// The Move tool's controls with the Ellipse tool's centre mark at the middle of the box.
#[story(TransformBox<'static>, covers = [Default])]
fn ellipse_centre_mark(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data, |state| free_box(state).rotate_zone(RotateZone::Band).inside_moves(false).reference_point(false).centre_mark(true))
}

/// The Line tool's controls on a live line across the box: a square at each end and the round widget at its middle,
/// no frame. Each end drags as the corner it points to.
#[story(TransformBox<'static>, covers = [Default], drive = Drive::Drag(Grip::part(BOX.at(BoxPart::Corner(1).index())).toward(vec2(1.0, -1.0))))]
fn live_line(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let quad = data.value("box", BoxState::from_rect(PICTURE)).quad;
    let line = LineLook::new(quad[3], quad[1]);
    on_canvas(ui, data, move |state| free_box(state).rotate_zone(RotateZone::Band).inside_moves(false).reference_point(false).line(line))
}

/// Free Transform turned only by the rotate handle on its stem above the top side; the rest of the outside is the
/// tool's.
#[story(TransformBox<'static>, covers = [Default], drive = Drive::Drag(Grip::part(ROTATE_HANDLE.at(0)).toward(vec2(1.0, 0.0))))]
fn rotate_handle(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data, |state| free_box(state).reference_point(true).rotate_zone(RotateZone::Handle))
}

/// A live line whose ends are free points: dragging an end moves only that end, at any angle, and the story commits
/// it as a host does; the box stays.
#[story(TransformBox<'static>, covers = [Default], drive = Drive::Drag(Grip::part(LINE_END.at(1)).toward(vec2(1.0, -1.0))))]
fn line_free_ends(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let quad = BoxState::from_rect(PICTURE).quad;
    let ends = *data.value("ends", [quad[3], quad[1]]);
    let line = LineLook::new(ends[0], ends[1]).ends(LineEnds::Free);
    let shown =
        shown_on_canvas(ui, data, move |state| free_box(state).rotate_zone(RotateZone::Band).inside_moves(false).reference_point(false).line(line));
    let Some(output) = shown else { return StoryOutput::default() };
    if let Some(drag) = output.line_end {
        let index = usize::from(drag.end == LineEnd::To);
        match drag.phase {
            GesturePhase::Cancel => {
                let before = *data.value("before", ends);
                *data.value("ends", ends) = before;
            }
            phase => {
                if phase == GesturePhase::Begin {
                    *data.value("before", ends) = ends;
                }
                data.value("ends", ends)[index] = drag.at;
            }
        }
    }
    StoryOutput::from(output.edit)
}
