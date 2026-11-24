//! The zoom field's own text format and its refusal signal, the presses that keep its typed zoom unapplied, and the
//! edit commands it keeps from the app with only a caret.

use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{CanvasView, PartKey, Sense};
use craft_host::{AppInfo, Host};
use craft_ui::buttons::Button;
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PartsCx, Props as _, Role, Show as _, ShowContainer as _};
use craft_ui::layout_components::Row;
use craft_ui::runtime::commands::{CommandRef, CommandState, EditCommands, FireSource, fire};
use craft_ui::runtime::env::Env;
use craft_ui::runtime::frame;
use craft_ui::style::StyleCx;
use craft_ui::testing::{Fixture, pointer, trial};
use craft_ui::text::{InvalidZoom, ZoomField};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Event, Key, Modifiers, Pos2, Rect, Response, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

/// The document the field zooms, in document units: its lowest zoom is far below every zoom the trials type.
const DOCUMENT: Size = Size::new(2000.0, 1500.0);

/// The field's label.
const ZOOM: &str = "Zoom";

/// The inert part of [`Pad`].
const PAD: PartKey = PartKey::new("pad");

/// A component with one inert part that declares `holds_drafts`, as the pasteboard and the title bar do.
#[derive(craft_ui::contract::Component)]
struct Pad {
    #[common]
    common: Common,
}

impl Pad {
    fn new() -> Self {
        Pad { common: Common::new() }.label("Pad")
    }
}

impl Component for Pad {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: PadStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(80.0, 40.0)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.part(PAD, rect).sense(Sense::empty()).holds_drafts();
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// What the trials' app keeps between passes.
#[derive(Clone)]
struct Kept {
    /// The view the field zooms.
    view: Rc<Cell<CanvasView>>,
    /// The zoom in percent of the latest commit the field reported, until a trial clears it.
    committed: Rc<Cell<Option<f64>>>,
    /// The latest cut entry the field reported, until a trial clears it.
    invalid: Rc<RefCell<Option<InvalidZoom>>>,
    /// The interface scale the field reads.
    pixels_per_point: Rc<Cell<f32>>,
    /// Where the pad sat.
    pad: Rc<Cell<Rect>>,
}

impl Kept {
    fn new() -> Self {
        let canvas = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        Kept {
            view: Rc::new(Cell::new(CanvasView::new(canvas, Point::new(1000.0, 750.0), 1.0))),
            committed: Rc::new(Cell::new(None)),
            invalid: Rc::new(RefCell::new(None)),
            pixels_per_point: Rc::new(Cell::new(1.0)),
            pad: Rc::new(Cell::new(Rect::NOTHING)),
        }
    }

    fn percent(&self, view: &CanvasView) -> f64 {
        zoom::device(view, self.pixels_per_point.get()) * 100.0
    }

    /// The zoom of the kept view, in percent.
    fn zoom(&self) -> f64 {
        self.percent(&self.view.get())
    }

    /// The zoom field over the kept view, with the pad and a button beside it when `beside`.
    fn show(&self, ui: &mut Ui, beside: bool) {
        self.pixels_per_point.set(Env::of(ui).scale.0);
        Row::new().show(ui, |ui, _| {
            let out = ZoomField::new(ZOOM, Some(self.view.get()), DOCUMENT).show(ui);
            if let Some(landed) = out.view.landed() {
                self.view.set(*landed);
            }
            if let Some(committed) = out.view.committed() {
                self.committed.set(Some(self.percent(committed)));
            }
            if out.invalid_zoom.is_some() {
                *self.invalid.borrow_mut() = out.invalid_zoom;
            }
            if beside {
                self.pad.set(Pad::new().show(ui).rect);
                let _ = Button::new("Apply").show(ui);
            }
        });
    }
}

/// A host whose locale writes the decimal separator as a comma.
fn decimal_comma() -> Arc<Host> {
    let (i18n, problems) = craft_i18n::I18n::from_owned([("de".to_owned(), String::new(), "@id\ttrial.save\tSpeichern\n".to_owned())], &["de"]);
    assert!(problems.is_empty(), "{problems:?}");
    i18n.set_format(craft_i18n::Format::from_tag("de"));
    Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).i18n(Arc::new(i18n)).build())
}

/// A focused zoom field at 100 %, its text selected, with `beside` as in [`Kept::show`].
fn focused(comma: bool, beside: bool) -> (Fixture, Kept) {
    let kept = Kept::new();
    let app = kept.clone();
    let mut fixture = Fixture::new(move |ui| app.show(ui, beside));
    if comma {
        fixture.host(decimal_comma());
    }
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::A, Modifiers::COMMAND);
    (fixture, kept)
}

/// The field's text: its text runs, which the node's value reads with the unit after it.
fn text(fixture: &Fixture) -> String {
    let field = fixture.by_label(ZOOM).expect("the zoom field");
    fixture.tree().run_texts_of(field.node).concat()
}

/// How an entry reaches the field.
#[derive(Clone, Copy, Debug)]
enum Input {
    Typed(&'static str),
    Pasted(&'static str),
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[trial]
fn zero_zoom_keeps_the_previous_view() {
    let (mut fixture, kept) = focused(false, false);
    let before = kept.zoom();
    fixture.type_text("0");
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    assert!(close(kept.zoom(), before), "zero must not change the zoom: {}", kept.zoom());
}

#[trial]
fn negative_zoom_keeps_the_previous_view() {
    let (mut fixture, kept) = focused(false, false);
    let before = kept.zoom();
    fixture.type_text("-5");
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    assert!(close(kept.zoom(), before), "a negative zoom must not change the view: {}", kept.zoom());
}

/// The field reads digits, `/` as a ratio and one decimal separator of the locale; it drops every other character
/// and alerts. Tab writes the number in the field's own form, selects it and alerts; a second separator applies the
/// number before it and reports the range. Each row: (decimal comma, the entry, the text after Tab (no Tab: Enter
/// at once), the zoom after Enter, the entry refused, the range reported).
#[trial]
fn the_zoom_field_reads_ratios_and_one_separator_and_refuses_the_rest() {
    use Input::{Pasted, Typed};
    type Row = (bool, Input, Option<&'static str>, f64, bool, bool);
    let rows: [Row; 10] = [
        (false, Typed("045"), Some("45"), 45.0, false, false),
        (false, Typed("1/2"), Some("50"), 50.0, false, false),
        (false, Pasted("5/2"), Some("250"), 250.0, false, false),
        (false, Typed("99999"), Some("12800"), 12800.0, false, false),
        (false, Typed("4a5"), Some("45"), 45.0, true, false),
        (false, Typed("1,5"), Some("15"), 15.0, true, false),
        (true, Typed("45,5"), Some("45,5"), 45.5, false, false),
        (true, Pasted("1.5"), Some("15"), 15.0, true, false),
        (true, Typed("80,,2"), None, 80.0, false, true),
        (false, Pasted("80..2"), None, 80.0, false, true),
    ];
    for (comma, input, after_tab, zoom, refused, cut) in rows {
        let case = format!("{input:?} with a decimal comma: {comma}");
        let (mut fixture, kept) = focused(comma, false);
        frame::take_output(fixture.ctx());
        match input {
            Typed(entry) => fixture.type_text(entry),
            Pasted(entry) => fixture.frame(vec![Event::Paste(entry.to_owned())]),
        };
        fixture.settle();
        assert_eq!(frame::take_output(fixture.ctx()).refused, refused, "the entry alerts: {case}");
        if let Some(rewritten) = after_tab {
            fixture.key(Key::Tab, Modifiers::NONE);
            assert!(frame::take_output(fixture.ctx()).refused, "Tab alerts: {case}");
            assert_eq!(text(&fixture), rewritten, "the text after Tab: {case}");
            assert_eq!(kept.committed.get(), None, "Tab applies nothing: {case}");
        }
        fixture.key(Key::Enter, Modifiers::NONE);
        fixture.settle();
        assert!(kept.committed.get().is_some_and(|committed| close(committed, zoom)), "the zoom after Enter: {case}; {:?}", kept.committed.get());
        assert!(close(kept.zoom(), zoom), "the view: {case}; {}", kept.zoom());
        let invalid = kept.invalid.borrow().clone();
        assert_eq!(invalid.is_some(), cut, "the range is reported: {case}");
        if let Some(invalid) = invalid {
            assert!(invalid.low < 1.0 && close(invalid.high, 12800.0), "the field's zoom range: {invalid:?}");
            // Escape after the cut entry has nothing to restore: the number before the cut stays applied.
            kept.invalid.borrow_mut().take();
            fixture.key(Key::Escape, Modifiers::NONE);
            fixture.settle();
            assert!(close(kept.zoom(), zoom), "Escape after a cut keeps the applied zoom: {case}");
            assert!(kept.invalid.borrow().is_none(), "Escape reports no range: {case}");
        }
    }
}

/// A press on a part declared `holds_drafts` keeps the field's focus and its draft, which it writes in the field's own
/// form and selects; the next Enter applies it. The part is inert and no shape wins the press, so the rule is that of
/// the part under the point. A press on a button applies the draft.
#[trial]
fn a_press_on_a_draft_holding_part_keeps_the_typed_zoom_and_a_press_elsewhere_applies_it() {
    let (mut fixture, kept) = focused(false, true);
    let field = fixture.focused_stop();
    assert!(field.is_some(), "Tab focuses the zoom field");
    fixture.type_text("0200");
    fixture.settle();
    kept.committed.set(None);
    let pad = kept.pad.get();
    assert!(pad.is_positive(), "the pad is shown");
    fixture.click_at(pad.center());
    fixture.settle();
    assert_eq!(fixture.focused_stop(), field, "the press keeps the field's focus");
    assert_eq!(kept.committed.get(), None, "the press applies nothing");
    assert_eq!(text(&fixture), "200", "the draft shows in the field's own form");
    // The draft is selected, so typing replaces it.
    fixture.type_text("3");
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    assert!(kept.committed.get().is_some_and(|committed| close(committed, 3.0)), "Enter applies the draft: {:?}", kept.committed.get());

    let zoom = fixture.by_label(ZOOM).expect("the zoom field");
    fixture.click(&zoom);
    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.type_text("400");
    fixture.settle();
    kept.committed.set(None);
    let button = fixture.by_label("Apply").expect("the button");
    fixture.click(&button);
    fixture.settle();
    assert!(
        kept.committed.get().is_some_and(|committed| close(committed, 400.0)),
        "a press on a button applies the draft: {:?}",
        kept.committed.get()
    );
}

/// The ids of the app's Cut, Copy and Delete that the frames since the last call fired.
fn app_edits_fired(fixture: &Fixture) -> Vec<String> {
    let edits = [EditCommands::Cut, EditCommands::Copy, EditCommands::Delete].map(|command| CommandRef::from(command).id);
    frame::take_output(fixture.ctx()).fired.into_iter().map(|fired| fired.command.id).filter(|id| edits.contains(id)).collect()
}

/// With a caret and no selection, Cut, Copy and Delete do nothing in the field, and their chords, the platform's
/// clipboard events and their menu picks still go to the field, never to the app's commands, which would cut or
/// delete the document's selection; the edit menu shows them disabled. With a selection, Cmd+X cuts the field's text.
#[trial]
fn cut_copy_and_delete_with_a_caret_never_reach_the_app() {
    let (mut fixture, _kept) = focused(false, false);
    for command in [EditCommands::Cut, EditCommands::Copy, EditCommands::Delete] {
        fixture.command_state(command, CommandState::new());
    }
    // Typing replaces the selected text and leaves a caret after it.
    fixture.type_text("5");
    fixture.settle();
    frame::take_output(fixture.ctx());
    fixture.key(Key::X, Modifiers::COMMAND);
    fixture.key(Key::C, Modifiers::COMMAND);
    fixture.frame(vec![Event::Cut]);
    fixture.frame(vec![Event::Copy]);
    fixture.settle();
    assert_eq!(app_edits_fired(&fixture), Vec::<String>::new(), "Cmd+X, Cmd+C and the clipboard events with a caret fire nothing of the app's");
    assert_eq!(text(&fixture), "5", "and leave the field's text alone");

    let at = fixture.by_label(ZOOM).expect("the zoom field").center();
    fixture.frame(vec![pointer::secondary(at, true)]);
    fixture.frame(vec![pointer::secondary(at, false)]);
    fixture.settle();
    let delete = fixture.by_label("Delete").expect("the edit menu's Delete row");
    assert!(fixture.tree().is_disabled_key(delete.node), "the edit menu shows Delete disabled with a caret");
    fixture.click(&delete);
    fixture.settle();
    assert_eq!(app_edits_fired(&fixture), Vec::<String>::new(), "the edit menu's Delete with a caret leaves the app's Delete unfired");
    assert_eq!(text(&fixture), "5", "and the field's text alone");
    // A pick of a disabled row may leave the menu open; Escape then closes it and gives the field the focus back.
    if fixture.by_label("Select All").is_some() {
        fixture.key(Key::Escape, Modifiers::NONE);
        fixture.settle();
    }
    assert!(!fire(fixture.ctx(), EditCommands::Delete, FireSource::Pointer), "Edit › Delete from a menu bar does not run with a caret");
    fixture.settle();
    assert_eq!(app_edits_fired(&fixture), Vec::<String>::new(), "and leaves the app's Delete unfired");
    assert_eq!(text(&fixture), "5", "and the field's text alone");

    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.settle();
    fixture.key(Key::X, Modifiers::COMMAND);
    fixture.settle();
    assert_eq!(text(&fixture), "", "with a selection, Cmd+X cuts the field's text");
    assert_eq!(app_edits_fired(&fixture), Vec::<String>::new(), "and the app's Cut stays unfired");
}

/// A host with a narrow fixed slot can keep the zoom field and its editable text inside it.
#[trial]
fn zoom_field_fits_a_narrow_host_slot() {
    let slot = Rc::new(Cell::new(Rect::NOTHING));
    let held = slot.clone();
    let kept = Kept::new();
    let app = kept.clone();
    let mut fixture = Fixture::new(move |ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(40.0, 24.0), egui::Sense::hover());
        held.set(rect);
        let out = ZoomField::new(ZOOM, Some(app.view.get()), DOCUMENT).show_at(ui, rect);
        if let Some(view) = out.view.committed() {
            app.view.set(*view);
        }
    });
    fixture.settle();
    let field = fixture.by_label(ZOOM).expect("the zoom field");
    assert!(slot.get().contains_rect(field.rect), "zoom field {:?} fits its host slot {:?}", field.rect, slot.get());
    fixture.click(&field).settle();
    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.type_text("200");
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert!((kept.zoom() - 200.0).abs() < 0.01, "the field remains editable");
}

/// A rail-width field shows all of a three-digit zoom and its percent sign.
#[trial]
fn rail_width_zoom_shows_its_whole_value() {
    let mut fixture = Fixture::new(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(40.0, 30.0), egui::Sense::hover());
        let canvas = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let view = CanvasView::new(canvas, Point::new(1000.0, 750.0), 2.44);
        ZoomField::new(ZOOM, Some(view), DOCUMENT).show_at(ui, rect);
    });
    fixture.settle();
    for text in ["244", "%"] {
        let run = fixture.texts().iter().find(|run| run.text == text).expect("the zoom value is painted");
        assert!(!run.elided && run.clip.contains_rect(run.rect), "{text} must be visible: {run:?}");
    }
}
