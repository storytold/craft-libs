//! `Canvas::editor`: the canvas's editor part holds key focus and the input method, and hands the pass's inputs to the
//! body through its `ToolPass`.

use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{CanvasView, EditorInput, Proposal, TextPos};
use craft_ui::canvas::{Canvas, tool_pass};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::primitives::{EditorPurpose, EditorState};
use craft_ui::runtime::access::Role;
use craft_ui::runtime::focus;
use craft_ui::testing::focus::egui_focused;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::TextField;
use egui::{Event, Id, ImeEvent, Key, Modifiers, Rect, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The page the canvas shows, in document units.
const PAGE: Size = Size::new(400.0, 300.0);

/// The fixture's room, the canvas's whole rect, in points.
const ROOM: Vec2 = Vec2::new(400.0, 300.0);

/// Where the body reports its caret, from the canvas's top left corner, in points.
const CARET: Vec2 = Vec2::new(40.0, 30.0);

/// What the trial's canvas reported: the inputs its body read, in order; whether its measuring run published a pass to
/// the body (`None` before one ran); the caret the body last reported; and the canvas's id.
#[derive(Default)]
struct Seen {
    inputs: RefCell<Vec<EditorInput>>,
    measured: Cell<Option<bool>>,
    caret: Cell<Option<Rect>>,
    id: Cell<Option<Id>>,
}

fn canvas<'a>(view: CanvasView) -> Canvas<'a> {
    Canvas::new("Page").document(PAGE).editor(EditorPurpose::Text).view(view)
}

#[trial]
fn a_canvas_editor_takes_focus_on_a_background_press_and_hands_its_inputs_to_the_tool_in_order() {
    let seen = Rc::new(Seen::default());
    let shown = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let view = CanvasView::new(room, Point::new(PAGE.width / 2.0, PAGE.height / 2.0), 1.0);
        // A measuring run of the same canvas, as a parent that sizes its children runs one before it shows them.
        let mut measured = canvas(view);
        let _ = measured.measure_in(ui, Proposal::Ideal, |_, body| shown.measured.set(Some(tool_pass(&body.env).is_some())));
        let output = canvas(view).show_at(ui, room, |ui, body| {
            let Some(pass) = tool_pass(&body.env) else { return };
            shown.inputs.borrow_mut().extend(pass.editor_input.iter().cloned());
            if let Some(editor) = &pass.editor {
                let caret = Rect::from_min_size(body.rect.min + CARET, Vec2::new(0.0, 16.0));
                editor.report(ui.ctx(), EditorState::new(caret, &[], TextPos::new(0, 0)));
                shown.caret.set(Some(caret));
            }
        });
        shown.id.set(Some(output.output.response.id));
    });
    fixture.room(ROOM);
    fixture.settle();
    assert_eq!(seen.measured.get(), Some(false), "a measuring run publishes no pass to the body");

    fixture.click_at((ROOM / 2.0).to_pos2());
    let canvas_id = seen.id.get().expect("the canvas showed");
    assert_eq!(fixture.focused_stop().map(|(id, _)| id), Some(canvas_id), "the background press focuses the canvas");
    let widget = egui_focused(fixture.ctx()).expect("a widget holds key focus");
    assert_ne!(widget, canvas_id, "the editor part's widget holds it, not the canvas's own");
    let ime = fixture.ime().expect("the focused editor owns the input method").clone();
    assert_eq!(ime.owner, widget, "the input-method owner is the editor part's widget");
    let caret = seen.caret.get().expect("the body reported through the pass's editor");
    assert_eq!(ime.caret, caret, "the report through `pass.editor` reaches the input-method note");

    seen.inputs.borrow_mut().clear();
    fixture.frame(vec![Event::Text("ab".into())]);
    fixture.key(Key::ArrowLeft, Modifiers::NONE);
    fixture.frame(vec![Event::Ime(ImeEvent::Preedit { text: "k".to_owned(), active_range_chars: None })]);
    let inputs = seen.inputs.take();
    assert!(
        matches!(
            inputs.as_slice(),
            [EditorInput::Text(text), EditorInput::Move(..), EditorInput::Preedit { text: composing, .. }]
                if &**text == "ab" && &**composing == "k"
        ),
        "typed text, a move and a composition reach the body's pass in the order typed, once each: {inputs:?}"
    );
}

/// Guards `EditorHandle::report` for an editor whose view describes its text (a page editor): its report has no lines,
/// so its caret reaches the input method and no empty text is announced on the canvas's node.
#[trial]
fn an_editor_report_with_no_lines_reaches_the_input_method_and_publishes_no_text() {
    let id: Rc<Cell<Option<Id>>> = Rc::default();
    let shown = Rc::clone(&id);
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let view = CanvasView::new(room, Point::new(PAGE.width / 2.0, PAGE.height / 2.0), 1.0);
        let output = canvas(view).show_at(ui, room, |ui, body| {
            let Some(pass) = tool_pass(&body.env) else { return };
            if let Some(editor) = &pass.editor {
                let caret = Rect::from_min_size(body.rect.min + CARET, Vec2::new(0.0, 16.0));
                editor.report(ui.ctx(), EditorState::new(caret, &[], TextPos::new(0, 0)));
            }
        });
        shown.set(Some(output.output.response.id));
    });
    fixture.room(ROOM);
    fixture.settle();
    fixture.click_at((ROOM / 2.0).to_pos2());
    let canvas_id = id.get().expect("the canvas showed");
    assert!(fixture.ime().is_some_and(|ime| ime.caret.is_finite()), "the caret reaches the input method");
    assert_ne!(fixture.tree().value(canvas_id), Some(""), "no empty text is announced on the canvas");
    assert_eq!(fixture.tree().text_selection(canvas_id), None, "and no caret in it");
}

/// Guards `focus::has` on a canvas: its editor part's widget holds egui focus under its own id, so a query of egui
/// focus by the canvas's id would answer no while the canvas has focus.
#[trial]
fn focus_has_answers_for_a_canvas_whose_editor_part_holds_focus() {
    let ids: Rc<Cell<Option<(Id, Id)>>> = Rc::default();
    let shown = Rc::clone(&ids);
    let mut fixture = Fixture::new(move |ui| {
        let mut notes = String::new();
        let field = TextField::new("Notes", Bind::bound(&mut notes)).show(ui).response;
        let room = ui.available_rect_before_wrap();
        let view = CanvasView::new(room, Point::new(PAGE.width / 2.0, PAGE.height / 2.0), 1.0);
        let output = canvas(view).show_at(ui, room, |_, _| {});
        shown.set(Some((output.output.response.id, field.id)));
    });
    fixture.room(ROOM);
    fixture.settle();
    let (canvas_id, field_id) = ids.get().expect("the canvas and the field showed");
    assert!(!focus::has(fixture.ctx(), canvas_id), "nothing has focus yet");

    let canvas_rect = fixture.by_role(Role::CANVAS).into_iter().next().expect("a canvas").rect;
    fixture.click_at(canvas_rect.center());
    assert!(focus::has(fixture.ctx(), canvas_id), "the canvas has focus while its editor part holds it");
    assert!(!focus::has(fixture.ctx(), field_id), "the field does not");

    let field_rect = fixture.by_role(Role::TEXT_INPUT).into_iter().next().expect("the field").rect;
    fixture.click_at(field_rect.center());
    assert!(focus::has(fixture.ctx(), field_id), "a click on the field moves focus there");
    assert!(!focus::has(fixture.ctx(), canvas_id), "and the canvas has it no more");
}
