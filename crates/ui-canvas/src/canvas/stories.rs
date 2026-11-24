//! Stories of the canvas family.

use super::{
    Canvas, CanvasTool, ContextSource, CropOverlay, MiddleButton, NavigationOptions, NoTool, ToolAction, ToolContext, ToolHost, ToolPath,
    ToolPointer, WheelScheme, tool_pass,
};
use crate::buttons::Button;
use crate::canvas::testing::canvas_story::{StoryDocument, show_document};
use crate::contract::{Bind, BodyCx, CanvasGesture, Gap, Sampler as _, Show as _, ShowContainer as _, clicked, tooltip};
use crate::dialog::Dialog;
use crate::display::Text;
use crate::layout_components::Stack;
use crate::primitives::{Draw, EditorPurpose, EditorState};
use crate::runtime::access::Role;
use crate::runtime::env::Env;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, pointer, stage, story};
use craft_behavior::geometry::CropBox;
use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_behavior::{CancelReason, EditorInput, TextPos};
use egui::{Key, Modifiers, Rect, Ui, vec2};

/// The zoomed story's zoom, in percent of one device pixel per document pixel.
const ZOOMED: f64 = 200.0;

fn crop() -> CropBox {
    CropBox::upright(DocRect::new(240.0, 145.0, 400.0, 255.0))
}

#[story(Canvas, covers = [Default])]
fn crop_tool(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    let view = StoryDocument::STANDARD.view(room, ui.pixels_per_point(), 100.0);
    let shown = Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
        show_document(ui, body.rect, &StoryDocument::STANDARD);
        CropOverlay::new("Crop", Bind::given(crop())).show_at(ui, body.rect)
    });
    shown.output.into()
}

#[story(Canvas, covers = [Default])]
fn zoomed(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    let view = StoryDocument::STANDARD.view(room, ui.pixels_per_point(), ZOOMED);
    let shown = Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
        show_document(ui, body.rect, &StoryDocument::STANDARD);
        CropOverlay::new("Crop", Bind::given(crop())).show_at(ui, body.rect)
    });
    shown.output.into()
}

#[story(Canvas, covers = [Default])]
fn freehand(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    Canvas::new("Painting")
        .view(StoryDocument::STANDARD.view(room, ui.pixels_per_point(), 100.0))
        .show_at(ui, room, |ui, body| show_document(ui, body.rect, &StoryDocument::STANDARD))
        .output
        .into()
}

/// The canvas under an open modal dialog that keeps it live: the wheel scrolls and zooms it, the middle button and
/// Space pan it, and a click on the image picks a point into the dialog. Every other input stays the dialog's.
#[story(Canvas, covers = [], drive = Drive::Never)]
fn under_a_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    let pixels_per_point = ui.pixels_per_point();
    let mut open = *data.value("open", false);
    let mut picked = *data.value::<Option<Point>>("picked", None);
    let row = |ui: &mut Ui, _: &mut BodyCx<'_>| {
        if clicked(&Button::new("Pick a Point…").show(ui)) {
            open = true;
        }
    };
    let stage = Stack::new().fill().footer(row).footer_gap(Gap::Stack).show_at(ui, room, |ui, stack| {
        let (view, view_at) = data.value_at("view", StoryDocument::STANDARD.view(stack.rect, pixels_per_point, 100.0));
        Canvas::new("Document")
            .view(view_at.bind(view))
            .show_at(ui, stack.rect, |ui, body| show_document(ui, body.rect, &StoryDocument::STANDARD))
            .output
    });
    let Some(canvas) = stage.inner else { return StoryOutput::default() };
    // The press the dialog lets through reaches the canvas's background, where the app's tool would take it.
    if open && let (Some(at), Some(space)) = (canvas.click_at(), canvas.space) {
        picked = Some(space.view.to_doc(at));
    }
    let id = canvas.response.id;
    Dialog::new("Pick a Point").opened(Bind::bound(&mut open)).show(ui, |ui, cx| {
        cx.keep_view_open(id);
        cx.pass_canvas_gesture(id, CanvasGesture::PRIMARY);
        let text = picked.map_or_else(|| "Click the image to pick a point.".to_owned(), |at| format!("Picked {:.0}, {:.0}", at.x, at.y));
        Text::new(text).show(ui);
    });
    *data.value("open", false) = open;
    *data.value::<Option<Point>>("picked", None) = picked;
    canvas.into()
}

/// The centre of the story's canvas.
fn canvas_centre(fixture: &Fixture) -> Option<egui::Pos2> {
    fixture.by_role(Role::CANVAS).first().map(|canvas| canvas.rect.center())
}

/// Middle-clicks the canvas's centre: the autoscroll mark stands where the click came up.
fn middle_click_centre(fixture: &mut Fixture) {
    let Some(at) = canvas_centre(fixture) else { return };
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::middle(at, true)]);
    fixture.frame(vec![pointer::middle(at, false)]);
}

/// Clicks the canvas, so it has focus, and presses Shift+F10.
fn press_menu_key(fixture: &mut Fixture) {
    let Some(at) = canvas_centre(fixture) else { return };
    fixture.click_at(at);
    fixture.key(Key::F10, Modifiers::SHIFT).settle();
}

/// A document editor's wheel: the wheel scrolls, Shift + wheel scrolls sideways, and Ctrl or Cmd + wheel steps the zoom
/// ladder.
#[story(Canvas, covers = [Default])]
fn document_wheel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    let navigation = NavigationOptions { wheel: WheelScheme::Document, ..NavigationOptions::default() };
    let (view, view_at) = data.value_at("view", scrolled_view(room, ui.pixels_per_point()));
    Canvas::new("Document")
        .navigation(navigation)
        .view(view_at.bind(&mut *view))
        .show_at(ui, room, |ui, body| show_document(ui, body.rect, &StoryDocument::STANDARD))
        .output
        .into()
}

/// The standard document zoomed in and scrolled off its origin, so a story that scrolls opens away from the start.
fn scrolled_view(room: Rect, pixels_per_point: f32) -> craft_behavior::CanvasView {
    let mut view = StoryDocument::STANDARD.view(room, pixels_per_point, ZOOMED);
    view.center = Point::new(view.center.x + 60.0, view.center.y + 40.0);
    view
}

/// The middle button's autoscroll, started by a middle click: the mark stands where the click came up, and the view
/// scrolls toward the pointer once it leaves the mark's dead zone.
#[story(Canvas, covers = [], drive = Drive::Custom(middle_click_centre))]
fn middle_autoscroll(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    let navigation = NavigationOptions { middle_button: MiddleButton::AutoScroll, ..NavigationOptions::default() };
    let (view, view_at) = data.value_at("view", scrolled_view(room, ui.pixels_per_point()));
    let shown = Canvas::new("Document")
        .navigation(navigation)
        .view(view_at.bind(&mut *view))
        .show_at(ui, room, |ui, body| show_document(ui, body.rect, &StoryDocument::STANDARD));
    shown.output.into()
}

/// A canvas with an editor part: a press on it gives the part key focus and the input method, the body reads the typed
/// text from its pass and reports its caret back through the pass's editor.
#[story(Canvas, covers = [Default])]
fn editor_part(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    let view = StoryDocument::STANDARD.view(room, ui.pixels_per_point(), 100.0);
    let typed = data.value("typed", String::from("Typed text"));
    let shown = Canvas::new("Page").editor(EditorPurpose::Text).view(view).show_at(ui, room, |ui, body| {
        show_document(ui, body.rect, &StoryDocument::STANDARD);
        let Some(pass) = tool_pass(&body.env) else { return };
        for input in pass.editor_input.iter() {
            if let EditorInput::Text(text) = input {
                typed.push_str(text);
            }
        }
        // A caret one text line tall at the canvas's top left, past what was typed, as a document editor reports its
        // own caret.
        let line = Env::of(ui).tokens.typography.body.line_height;
        let at = Rect::from_min_size(body.rect.min, vec2(body.rect.width(), line));
        let caret = Rect::from_min_size(at.min, vec2(0.0, line));
        if let Some(editor) = &pass.editor {
            editor.report(ui.ctx(), EditorState::new(caret, &[], TextPos::new(0, typed.len())));
        }
        if !typed.is_empty() {
            Text::new(typed.clone()).show_at(ui, at);
        }
    });
    shown.output.into()
}

/// A tool that asks for a pointer tip on every hover and takes presses, as a document editor's text tool tips a link.
struct TippingTool;

impl CanvasTool for TippingTool {
    fn hover(&mut self, _: &ToolPointer, cx: &mut ToolContext<'_>) {
        cx.tip("Ctrl+click to follow the link");
    }

    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Follow Link command")
    }
}

/// Rests the pointer on the canvas's centre past the tooltip delay, so the tool's tip shows.
fn rest_on_centre(fixture: &mut Fixture) {
    let Some(at) = canvas_centre(fixture) else { return };
    fixture.frame(vec![pointer::moved(at)]);
    fixture.advance(tooltip::DELAY);
    fixture.settle();
}

/// A tool's pointer tip ([`ToolContext::tip`]): it shows at the pointer after the tooltip delay while the pointer rests
/// on the canvas, and a press hides it.
#[story(Canvas, covers = [], drive = Drive::Custom(rest_on_centre))]
fn tool_tip(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    let view = StoryDocument::STANDARD.view(room, ui.pixels_per_point(), 100.0);
    let shown = Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
        show_document(ui, body.rect, &StoryDocument::STANDARD);
        if let Some(pass) = tool_pass(&body.env) {
            let _ = ToolHost::new().run(ui, &pass, &mut TippingTool);
        }
    });
    shown.output.into()
}

/// The canvas menu asked for with Shift+F10: the line under the canvas says how the tool host reported the request
/// ([`ContextRequest::source`](super::ContextRequest::source)).
#[story(Canvas, covers = [], drive = Drive::Custom(press_menu_key))]
fn context_menu_key(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    let pixels_per_point = ui.pixels_per_point();
    let asked = data.value("asked", String::from("Right-click the canvas or press the Menu key.")).clone();
    let mut source = None;
    let row = |ui: &mut Ui, _: &mut BodyCx<'_>| {
        Text::new(asked.clone()).show(ui);
    };
    let stage = Stack::new().fill().footer(row).footer_gap(Gap::Stack).show_at(ui, room, |ui, stack| {
        let view = StoryDocument::STANDARD.view(stack.rect, pixels_per_point, 100.0);
        Canvas::new("Document")
            .view(view)
            .show_at(ui, stack.rect, |ui, body| {
                show_document(ui, body.rect, &StoryDocument::STANDARD);
                if let Some(pass) = tool_pass(&body.env) {
                    source = ToolHost::new().run(ui, &pass, &mut NoTool).context.map(|request| request.source).or(source);
                }
            })
            .output
    });
    let words = match source {
        Some(ContextSource::Key) => Some("The menu was asked for with a key: it opens at the view's centre."),
        Some(ContextSource::Pointer) => Some("The menu was asked for with the pointer: it opens where the pointer is."),
        None => None,
    };
    if let Some(words) = words {
        *data.value("asked", String::new()) = words.to_owned();
    }
    let Some(canvas) = stage.inner else { return StoryOutput::default() };
    canvas.into()
}
