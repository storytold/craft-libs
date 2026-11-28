//! The print dialog: the image box drag, the held Print and a sheet of pages.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::channel;

use craft_behavior::EditEvent;
use craft_print::ui::{
    Destination, DriverControls, HostedPrint, PrintDialog, PrintFinish, PrintLayout, PrintSection, PrintSheet, Printer, Settings, SheetPage, State,
};
use craft_ui::contract::Bind;
use craft_ui::dialog::{DialogHost, DialogResult, HeldOk, StackDialog as _, dialog_id};
use craft_ui::patterns::form_dialogs::{DialogFields, FieldValue, PanePager, Param, ParamKind};
use craft_ui::patterns::presets::{NamedPresetEvent, PresetRow};
use craft_ui::primitives::TextureCache;
use craft_ui::runtime::env::{Label, Scale};
use craft_ui::testing::{Fixture, trial};
use egui::load::SizedTexture;
use egui::{Color32, ColorImage, Pos2, Rect, Shape, TextureId, Ui, Vec2, pos2, vec2};

/// The image box's size in paper points.
const IMAGE_SIZE: Vec2 = Vec2::new(200.0, 300.0);

/// Where the sheet puts the image box, in paper points.
const IMAGE_AT: Pos2 = Pos2::new(72.0, 72.0);

type Log = Rc<RefCell<Vec<EditEvent<Pos2>>>>;

fn picture(ui: &Ui) -> SizedTexture {
    let id = TextureCache::texture(ui.ctx(), "print-dialog-trial", || ColorImage::filled([20, 30], Color32::from_rgb(0x4a, 0x7f, 0xc1)));
    SizedTexture::new(id, Vec2::new(20.0, 30.0))
}

/// A print dialog over a fixed sheet of `paper` points; the events of the image box drag land in the log.
fn rig(paper: Vec2) -> (Fixture, Log) {
    let log: Log = Rc::default();
    let seen = log.clone();
    let mut settings = Settings::default();
    let mut fields = DialogFields::default();
    let printers = vec![Printer::new("Office")];
    let mut fixture = Fixture::new(move |ui| {
        let image = Rect::from_min_size(IMAGE_AT, IMAGE_SIZE);
        let sheet = PrintSheet::new(paper, image, picture(ui), "200 × 300 pt".into());
        let out = PrintDialog::new(Bind::bound(&mut settings), &printers, Bind::bound(&mut fields)).sheet(&sheet).ready(true).show(ui);
        seen.borrow_mut().extend(out.moved);
        // Every frame runs twice; the second pass has no new input, and a repeated move would show in the log.
        if ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the frame runs twice");
        }
    });
    fixture.room(Vec2::new(1400.0, 1000.0));
    fixture.settle();
    (fixture, log)
}

/// One drag gesture on a sheet of `paper` points: the preview scale differs with the paper.
fn check(paper: Vec2) {
    let (mut fixture, log) = rig(paper);
    assert!(fixture.by_label("Print preview image").is_some(), "the preview is named as production names it");
    let image = fixture.by_label("Image box").expect("the image box is in the access tree");
    let scale = image.rect.width() / IMAGE_SIZE.x;
    let from = image.center();
    let by = vec2(40.0, 20.0);

    // A click with no drag begins nothing, and neither does a press beside the image box.
    fixture.click_at(from);
    assert!(log.borrow().is_empty(), "a click moves nothing: {:?}", log.borrow());
    fixture.drag_at(pos2(image.rect.min.x - 4.0, image.center().y), by);
    assert!(log.borrow().is_empty(), "a press outside the image box moves nothing: {:?}", log.borrow());

    fixture.drag_hold_at(from, by);
    {
        let log = log.borrow();
        assert!(matches!(log.first(), Some(EditEvent::Begin { .. })), "the drag begins a gesture: {log:?}");
        let Some(EditEvent::Update { value, .. }) = log.last() else { panic!("the drag updates: {log:?}") };
        let expected = IMAGE_AT + by / scale;
        assert!((*value - expected).length() < 0.01, "position {value:?} should be {expected:?} at scale {scale}");
        let shown: Vec<String> = log.iter().map(|event| format!("{event:?}")).collect();
        assert!(shown.windows(2).all(|pair| pair[0] != pair[1]), "a rerun pass repeated an event: {log:?}");
    }
    fixture.release_at(from + by);
    let log = log.borrow();
    let Some(EditEvent::Commit { after, .. }) = log.last() else { panic!("the release commits: {log:?}") };
    assert!((*after - (IMAGE_AT + by / scale)).length() < 0.01, "committed position {after:?} should be {:?}", IMAGE_AT + by / scale);
}

/// The image box drag reports the box's new position on the paper in paper points, where the sheet put it plus the
/// travel from the press, at two preview scales (screen points are divided by the scale); a click and a press outside
/// the box report nothing.
#[trial]
fn the_image_box_drag_reports_the_new_position() {
    check(vec2(612.0, 792.0));
    check(vec2(306.0, 396.0));
}

/// The drag keeps reporting while the pointer is far outside the sheet: the position follows the pointer's travel from
/// the press wherever it goes, so a box dragged off the page keeps its capture, and the release commits it there.
#[trial]
fn the_image_box_drag_keeps_its_capture_off_the_sheet() {
    let (mut fixture, log) = rig(vec2(612.0, 792.0));
    let image = fixture.by_label("Image box").expect("the image box is in the access tree");
    let scale = image.rect.width() / IMAGE_SIZE.x;
    let from = image.center();
    let away = vec2(-400.0, 0.0);
    fixture.drag_hold_at(from, away);
    let expected = IMAGE_AT + away / scale;
    {
        let log = log.borrow();
        let Some(EditEvent::Update { value, .. }) = log.last() else { panic!("the drag updates: {log:?}") };
        assert!((*value - expected).length() < 0.01, "the position follows a pointer outside the sheet: {value:?} should be {expected:?}");
        assert!(value.x < IMAGE_AT.x - 100.0, "the box left the sheet's left edge: {value:?}");
    }
    fixture.release_at(from + away);
    let log = log.borrow();
    let Some(EditEvent::Commit { after, .. }) = log.last() else { panic!("the release commits: {log:?}") };
    assert!((*after - expected).length() < 0.01, "committed {after:?} should be {expected:?}");
}

/// The listing of the dialog's values carries what the app's `json_form` adds, over the settings keys.
#[trial]
fn the_json_form_adds_to_the_listing() {
    let fields = DialogFields::from_pairs([("paper", FieldValue::Text("custom".into())), ("paperWidth", FieldValue::Number(432.0))]);
    let dialog = HostedPrint::new(State::default(), fields, Vec::new()).json_form(|fields, map| {
        if fields.text("paper") == Some("custom") {
            map.insert("paper".to_owned(), serde_json::json!([fields.number("paperWidth"), 792.0]));
        }
    });
    let listed = dialog.json_fields().expect("a listing");
    assert_eq!(listed["paper"], serde_json::json!([432.0, 792.0]));
    assert!(listed.get("copies").is_some(), "the settings keys stay");
}

/// A hosted print dialog on a three-sheet document, run through a `DialogHost`.
fn hosted() -> (Fixture, Rc<RefCell<DialogHost>>) {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    host.borrow_mut().open(HostedPrint::new(State::default(), DialogFields::default(), Vec::new()));
    let shared = host.clone();
    let mut fixture = Fixture::new(move |ui| {
        let image = Rect::from_min_size(IMAGE_AT, IMAGE_SIZE);
        let sheet = PrintSheet::new(vec2(612.0, 792.0), image, picture(ui), "200 × 300 pt".into()).count(3);
        let mut host = shared.borrow_mut();
        if let Some(dialog) = host.get_mut::<HostedPrint>() {
            dialog.set_sheet(Some(sheet));
        }
        host.run(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0));
    fixture.settle();
    (fixture, host)
}

fn shown_sheet(host: &Rc<RefCell<DialogHost>>) -> usize {
    host.borrow().get::<HostedPrint>().expect("the dialog is open").sheet_index()
}

/// `choice: "done"` makes the next `confirm` answer Done once, any other choice is refused, and the sheet step moves the
/// shown sheet by its buttons and by the `sheet` field, which clamps to the last sheet.
#[trial]
fn done_by_automation_and_the_sheet_step() {
    let (mut fixture, host) = hosted();
    let ctx = egui::Context::default();
    assert_eq!(shown_sheet(&host), 0);
    for expected in [1, 2, 2] {
        let next = fixture.by_label("Next").expect("the step row has Next");
        fixture.click(&next);
        assert_eq!(shown_sheet(&host), expected, "Next steps to the next sheet and stops at the last");
    }
    let previous = fixture.by_label("Previous").expect("the step row has Previous");
    fixture.click(&previous);
    assert_eq!(shown_sheet(&host), 1);

    let mut host = host.borrow_mut();
    let dialog = host.get_mut::<HostedPrint>().expect("the dialog is open");
    dialog.set_json_field(&ctx, "sheet", serde_json::json!(9)).expect("an index past the end is clamped");
    assert_eq!(dialog.sheet_index(), 2);
    assert!(dialog.set_json_field(&ctx, "sheet", serde_json::json!("two")).is_err());

    assert!(dialog.set_json_field(&ctx, "choice", serde_json::json!("print")).is_err(), "only done is a choice");
    let ready = State::default().ready();
    dialog.set_json_field(&ctx, "choice", serde_json::json!("done")).expect("done is a choice");
    assert_eq!(dialog.confirm(&ctx).map(|answer| answer.finish), Some(PrintFinish::Done), "Done answers even while a job cannot start");
    assert_eq!(dialog.confirm(&ctx).map(|answer| answer.finish), ready.then_some(PrintFinish::Print), "the choice is used up");
}

/// A refused held OK keeps the dialog open with the reason, and a preset the row does not list is refused by name. A
/// preset is named by its key or by its name in any case, and applies by its key.
#[trial]
fn a_refused_print_keeps_the_dialog_open_and_presets_are_named() {
    let ctx = egui::Context::default();
    let mut dialog = HostedPrint::new(State::default(), DialogFields::default(), Vec::new()).held_ok();
    dialog.set_json_field(&ctx, "choice", serde_json::json!("done")).expect("done is a choice");
    assert!(dialog.confirm(&ctx).is_none(), "a held OK files its answer");
    assert!(matches!(dialog.held_state(), Some(HeldOk::Waiting)));
    assert!(dialog.take_ok().is_some());
    dialog.answer_ok(Err("No printer".into()));
    assert!(matches!(dialog.held_state(), Some(HeldOk::Refused(_))), "the refusal keeps the dialog open");

    dialog.set_presets(vec![PresetRow::item("proof".to_owned(), "Proof", 0)], None, Vec::new());
    assert!(dialog.set_json_field(&ctx, "preset", serde_json::json!("missing")).is_err());
    dialog.set_json_field(&ctx, "preset", serde_json::json!("proof")).expect("a listed preset applies");
    assert_eq!(dialog.take_preset_events(), vec![NamedPresetEvent::Apply("proof".to_owned())]);
    dialog.set_presets(vec![PresetRow::item("saved-7".to_owned(), "Proofs", 0)], None, Vec::new());
    dialog.set_json_field(&ctx, "preset", serde_json::json!("proofs")).expect("a preset named in another case applies");
    assert_eq!(dialog.take_preset_events(), vec![NamedPresetEvent::Apply("saved-7".to_owned())], "by its row's key");
}

/// The sections layout of a hosted print dialog shows its list of sections beside the shown section, and a click on
/// another section shows that one. The list sat inside the dialog's scrolling body, where a collection may not.
#[trial]
fn the_sections_layout_shows_the_picked_section() {
    use craft_print::ui::{PrintLayout, PrintSection};
    use craft_ui::patterns::form_dialogs::{Param, ParamKind};
    let options = || vec![("all".to_owned(), "All".into()), ("range".to_owned(), "Range".into())];
    let general = vec![
        Param::new("copies", "Copies", ParamKind::int(Some(1), Some(99), 1)),
        Param::new("artboards", "Artboards", ParamKind::radio(options(), None)),
        Param::section("Media"),
        Param::new("size", "Size", ParamKind::choice(options(), None)),
    ];
    let sections = vec![
        PrintSection::new("General", general),
        PrintSection::new("Printing Marks", vec![Param::number("markWeight", "Mark weight", 0.25, 0.25, 2)]),
    ];
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let header = vec![Param::number("pagesPerSheet", "Pages per sheet", 1.0, 1.0, 0)];
    host.borrow_mut().open(HostedPrint::new(State::default(), DialogFields::default(), sections).layout(PrintLayout::Sections).header(header));
    let shared = host.clone();
    let mut fixture = Fixture::new(move |ui| shared.borrow_mut().run(ui));
    fixture.room(Vec2::new(900.0, 600.0)).settle();
    assert!(fixture.by_label("Copies").is_some(), "the first section shows");
    let marks = fixture.by_label("Printing Marks").expect("the list names the other section");
    fixture.click(&marks).settle();
    assert!(fixture.by_label("Mark weight").is_some(), "the picked section shows");
    assert!(fixture.by_label("Copies").is_none(), "in place of the first");
}

/// In the sections layout the printer panel shows only the printer, so the app's own Copies row is the one automation
/// sets and lists; the dialog's hidden copies setting keeps its value.
#[trial]
fn a_section_row_takes_a_printer_setting_set_by_automation_in_the_sections_layout() {
    use craft_print::ui::{PrintLayout, PrintSection};
    use craft_ui::patterns::form_dialogs::{Param, ParamKind};
    let ctx = egui::Context::default();
    let general = vec![Param::new("copies", "Copies", ParamKind::int(Some(1), Some(99), 1))];
    let mut dialog =
        HostedPrint::new(State::default(), DialogFields::default(), vec![PrintSection::new("General", general)]).layout(PrintLayout::Sections);
    dialog.set_json_field(&ctx, "copies", serde_json::json!(4)).expect("the row takes the copies");
    assert_eq!(dialog.fields().number("copies"), Some(4.0), "the app's row holds the value");
    let listed = dialog.json_fields().expect("a listing");
    assert_eq!(listed["copies"].as_f64(), Some(4.0), "the listing reports the row: {listed}");
}

/// The listing carries the preview pager's sheet and count, and the keys of the note rows the values show: a note
/// whose row is hidden is not listed.
#[trial]
fn the_listing_carries_the_pager_and_the_shown_notes() {
    let notes = vec![
        Param::read_only_text("note.shown", Label::default(), "Printed as asked."),
        Param::read_only_text("note.hidden", Label::default(), "Two-sided is off.").visible_when("two_sided", FieldValue::Bool(true)),
        Param::parse_list(r#"{"range":1..9=1}"#).remove(0),
    ];
    let mut dialog = HostedPrint::new(State::default(), DialogFields::default(), vec![PrintSection::new("Paper", notes)]);
    dialog.set_preview_pager(Some(PanePager { shown: 2, count: 3, ..PanePager::default() }));
    let listed = dialog.json_fields().expect("a listing");
    assert_eq!((listed["sheet"].as_u64(), listed["sheets"].as_u64()), (Some(2), Some(3)));
    assert_eq!(listed["notes"], serde_json::json!(["note.shown"]), "only the shown note rows, and no other row");

    dialog.fields_mut().set("two_sided", FieldValue::Bool(true));
    assert_eq!(dialog.json_fields().expect("a listing")["notes"], serde_json::json!(["note.shown", "note.hidden"]));
}

/// A section value set through automation is recorded as an edit of its row, as the user's edit is, so the app's pass
/// that writes its own values back leaves it; a settings key is no row.
#[trial]
fn a_section_value_set_through_automation_is_an_edit() {
    let rows = PrintSection::new("Pages", Param::parse_list(r#"{"range":1..9=1}"#));
    let mut dialog = HostedPrint::new(State::default(), DialogFields::default(), vec![rows]);
    let ctx = egui::Context::default();
    dialog.set_json_field(&ctx, "range", serde_json::json!(4)).expect("the range row takes a number");
    dialog.set_json_field(&ctx, "copies", serde_json::json!(2)).expect("copies takes a count");
    assert_eq!(dialog.requests_mut().take_edited(), ["range"]);
    assert_eq!(dialog.fields_mut().number("range"), Some(4.0));
}

/// A held print dialog over the printer "Office", in a `DialogHost` run every pass. While `twice` is on, egui runs each
/// frame twice and gives the frame's input to its first pass only.
fn held_print(twice: &Rc<Cell<bool>>) -> (Fixture, Rc<RefCell<DialogHost>>) {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let (shown, again) = (Rc::clone(&host), Rc::clone(twice));
    let mut fixture = Fixture::new(move |ui| {
        shown.borrow_mut().run(ui);
        if again.get() && ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the frame runs twice");
        }
    });
    fixture.room(Vec2::new(1400.0, 1000.0));
    let mut state = State::default();
    state.settings.destination = Destination::Printer("Office".into());
    let mut dialog = HostedPrint::new(state, DialogFields::default(), Vec::new()).held_ok();
    let picture = SizedTexture::new(TextureId::default(), IMAGE_SIZE);
    dialog.set_sheet(PrintSheet::new(vec2(612.0, 792.0), Rect::from_min_size(IMAGE_AT, IMAGE_SIZE), picture, "200 × 300 pt".into()));
    let (send, printers) = channel();
    send.send(vec![Printer::new("Office")]).expect("the list is sent");
    assert!(dialog.listen(printers), "no discovery runs before the dialog opens");
    host.borrow_mut().open(dialog);
    fixture.idle(10);
    (fixture, host)
}

/// `act` on the open `HostedPrint`; `None` when none is open.
fn on_print<T>(host: &Rc<RefCell<DialogHost>>, act: impl FnOnce(&mut HostedPrint) -> T) -> Option<T> {
    host.borrow_mut().get_mut::<HostedPrint>().map(act)
}

/// Whether the Print button is disabled.
fn print_disabled(fixture: &Fixture) -> bool {
    let print = fixture.by_label("Print").expect("the Print button");
    fixture.tree().is_disabled_key(print.node)
}

/// Why the app refuses the held Print.
const REFUSED: &str = "The printer cannot take this paper size.";

/// Print on a held dialog files its answer once, in a frame that runs twice too, and waits disabled; the app's refusal
/// shows under the rows with Print enabled again until a value changes; an accepted Print closes the dialog with the
/// held answer.
#[trial]
fn a_refused_print_keeps_a_held_dialog_open_until_a_value_changes() {
    let twice = Rc::new(Cell::new(false));
    let (mut fixture, host) = held_print(&twice);
    let id = dialog_id("print");
    let print = fixture.by_label("Print").expect("the Print button");
    twice.set(true);
    fixture.click(&print);
    assert_eq!(fixture.passes(), 2, "the frame ran a second pass");
    twice.set(false);
    fixture.idle(4);
    let filed = on_print(&host, HostedPrint::take_ok).flatten().expect("Print files the held answer");
    assert_eq!(filed.settings.destination, Destination::Printer("Office".into()));
    assert!(on_print(&host, HostedPrint::take_ok).flatten().is_none(), "the answer is filed once");
    assert!(host.borrow().is_open::<HostedPrint>(), "a held Print keeps the dialog open");
    assert!(print_disabled(&fixture), "Print waits disabled for the app's answer");
    assert_eq!(host.borrow().held_state(id), Some(HeldOk::Waiting));

    on_print(&host, |dialog| dialog.answer_ok(Err(Label::from(REFUSED))));
    assert_eq!(host.borrow().held_state(id), Some(HeldOk::Refused(Label::from(REFUSED))));
    fixture.idle(4);
    assert!(host.borrow().is_open::<HostedPrint>(), "a refusal keeps the dialog open");
    assert!(fixture.by_label(REFUSED).is_some(), "the refusal shows under the rows");
    assert!(!print_disabled(&fixture), "a refusal enables Print again");
    let grayscale = fixture.by_label("Grayscale").expect("the Grayscale check box");
    fixture.click(&grayscale).idle(4);
    assert!(fixture.by_label(REFUSED).is_none(), "a changed value ends the refusal");
    assert_eq!(host.borrow().held_state(id), None, "nothing is held once the refusal ended");

    let print = fixture.by_label("Print").expect("the Print button");
    fixture.click(&print).idle(4);
    let filed = on_print(&host, HostedPrint::take_ok).flatten().expect("Print files again");
    assert!(filed.settings.grayscale, "the held answer has the changed value");
    on_print(&host, |dialog| dialog.answer_ok(Ok(())));
    assert_eq!(host.borrow().held_state(id), Some(HeldOk::Accepted), "the accepted Print closes at the next pass");
    fixture.idle(4);
    let answered = host.borrow_mut().take::<HostedPrint>().and_then(DialogResult::ok).expect("the accepted Print closes the dialog");
    assert_eq!(answered.finish, PrintFinish::Print);
    assert!(answered.settings.grayscale, "the dialog closes with the held answer");
}

/// The app can mark the settings it owns invalid (an empty page range): Print is disabled until they are valid again.
#[trial]
fn print_is_disabled_while_the_app_calls_the_settings_invalid() {
    let twice = Rc::new(Cell::new(false));
    let (mut fixture, host) = held_print(&twice);
    assert!(!print_disabled(&fixture), "the settings are valid until the app says otherwise");
    on_print(&host, |dialog| dialog.set_settings_valid(false));
    fixture.idle(4);
    assert!(print_disabled(&fixture), "Print is disabled on invalid settings");
    on_print(&host, |dialog| dialog.set_settings_valid(true));
    fixture.idle(4);
    assert!(!print_disabled(&fixture), "valid again, Print is enabled");
}

/// Taking back a held Print, as the app does when its Save as PDF picker is cancelled, enables Print again with the
/// values kept and no refusal, and the next Print files again.
#[trial]
fn a_withdrawn_print_enables_print_again_with_the_values_kept() {
    let twice = Rc::new(Cell::new(false));
    let (mut fixture, host) = held_print(&twice);
    let grayscale = fixture.by_label("Grayscale").expect("the Grayscale check box");
    fixture.click(&grayscale).idle(4);
    let print = fixture.by_label("Print").expect("the Print button");
    fixture.click(&print).idle(4);
    assert!(on_print(&host, HostedPrint::take_ok).flatten().is_some(), "Print files the held answer");
    assert!(print_disabled(&fixture), "Print waits disabled for the app's answer");

    on_print(&host, HostedPrint::release_ok);
    assert_eq!(host.borrow().held_state(dialog_id("print")), None, "nothing is held and no refusal stands");
    fixture.idle(4);
    assert!(host.borrow().is_open::<HostedPrint>(), "the dialog stays open");
    assert!(!print_disabled(&fixture), "Print is enabled again");
    assert_eq!(on_print(&host, |dialog| dialog.state().settings.grayscale), Some(true), "the values stay");
    let print = fixture.by_label("Print").expect("the Print button");
    fixture.click(&print).idle(4);
    assert!(on_print(&host, HostedPrint::take_ok).flatten().is_some(), "the next Print files again");
}

/// Letter paper in points.
const LETTER: Vec2 = Vec2::new(612.0, 792.0);

/// A sheet of two pages: a picture turned a quarter counter-clockwise, and a page with no picture yet that hangs over
/// the paper's right edge. The turned page's picture keeps its top edge up the cell's left side, the stand-in shows its
/// number, and both are cut at the paper's edge.
#[trial]
fn a_sheet_of_pages_places_each_page_inside_the_paper() {
    // The cell x 72..312, y 72..432: the picture's top-left is the cell's bottom-left, its top-right the cell's top-left.
    let turned = [pos2(72.0, 432.0), pos2(72.0, 72.0), pos2(312.0, 72.0), pos2(312.0, 432.0)];
    // x 400..700 on paper 612 wide; the centre (550, 600) stays on the paper.
    let overhanging = [pos2(400.0, 500.0), pos2(700.0, 500.0), pos2(700.0, 700.0), pos2(400.0, 700.0)];
    let made = Rc::new(Cell::new(None::<TextureId>));
    let seen = Rc::clone(&made);
    let mut settings = Settings::default();
    let mut fields = DialogFields::default();
    let printers = vec![Printer::new("Office")];
    let mut fixture = Fixture::new(move |ui| {
        let texture = picture(ui).id;
        seen.set(Some(texture));
        let whole = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
        let pages = vec![SheetPage::new(turned, "1").picture(texture, whole), SheetPage::new(overhanging, "2")];
        let sheet = PrintSheet::pages(LETTER, pages, "Letter, 2 pages".into());
        let _ =
            PrintDialog::new(Bind::bound(&mut settings), &printers, Bind::bound(&mut fields)).sheet(&sheet).layout(PrintLayout::Document).show(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0));
    fixture.settle();
    assert!(fixture.by_label("Image box").is_none(), "a sheet of pages has no image box to drag");
    let well = fixture.by_label("Print preview image").expect("the preview is in the access tree").rect;
    // The paper stands inset in the well by 4% of its width.
    let room = well.shrink(well.width() * 0.04);
    let scale = (room.width() / LETTER.x).min(room.height() / LETTER.y);
    let origin = room.center() - LETTER * scale / 2.0;
    let screen = |at: Pos2| origin + at.to_vec2() * scale;
    let paper = Rect::from_min_size(origin, LETTER * scale);

    let shapes = fixture.frame_shapes();
    let texture = made.get().expect("the page picture was made");
    let quad = shapes
        .iter()
        .find_map(|shape| match shape {
            Shape::Mesh(mesh) if mesh.texture_id == texture => Some(mesh.clone()),
            _ => None,
        })
        .expect("the first page is painted as a textured quad");
    let corners: Vec<Pos2> = quad.vertices.iter().map(|vertex| vertex.pos).collect();
    let expected: Vec<Pos2> = turned.iter().map(|corner| screen(*corner)).collect();
    assert_eq!(corners.len(), 4, "one quad: {corners:?}");
    assert!(corners.iter().zip(&expected).all(|(at, want)| (*at - *want).length() < 0.01), "the quad's corners {corners:?} are {expected:?}");
    let uvs: Vec<Pos2> = quad.vertices.iter().map(|vertex| vertex.uv).collect();
    assert_eq!(uvs, vec![pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)], "the picture's corners pair with the page's in order");

    let reach = screen(pos2(700.0, 700.0));
    let stand_in = shapes.iter().any(|shape| match shape {
        Shape::Mesh(mesh) => mesh.texture_id != texture && mesh.vertices.iter().any(|vertex| (vertex.pos - reach).length() < 0.5),
        _ => false,
    });
    assert!(stand_in, "the page with no picture is painted as a stand-in");
    let number = fixture.texts().iter().find(|run| run.text == "2").expect("the stand-in shows its number");
    assert!((number.rect.center() - screen(pos2(550.0, 600.0))).length() < 1.0, "the number is centred on the page: {:?}", number.rect);
    assert!(paper.expand(0.5).contains_rect(number.clip), "the page is cut at the paper's edge: clip {:?}, paper {paper:?}", number.clip);
    assert!(!number.clip.contains(reach), "the overhang past the paper is not shown");
}

/// The paper of a sheet of pages is one white sheet in every theme, so the gaps between its pages are paper, not the
/// dark of a dark theme.
#[trial]
fn the_paper_of_a_sheet_is_white_in_a_dark_theme() {
    use craft_theme::{Contrast, Mode};
    let mut settings = Settings::default();
    let mut fields = DialogFields::default();
    let printers = vec![Printer::new("Office")];
    let page = |at: f32| SheetPage::new([pos2(at, 100.0), pos2(at + 200.0, 100.0), pos2(at + 200.0, 400.0), pos2(at, 400.0)], "1");
    let mut fixture = Fixture::new(move |ui| {
        let sheet = PrintSheet::pages(LETTER, vec![page(50.0), page(300.0)], "Letter".into());
        let _ =
            PrintDialog::new(Bind::bound(&mut settings), &printers, Bind::bound(&mut fields)).sheet(&sheet).layout(PrintLayout::Document).show(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0));
    fixture.theme(Mode::Dark, Contrast::Standard).settle();
    let room = fixture.by_label("Print preview image").expect("the preview is in the access tree").rect;
    let scale = (room.width() / LETTER.x).min(room.height() / LETTER.y);
    let origin = room.center() - LETTER * scale / 2.0;
    let gap = origin + vec2(275.0, 250.0) * scale;
    let shapes = fixture.frame_shapes();
    let fill_at_gap = shapes.iter().rev().find_map(|shape| match shape {
        Shape::Rect(rect) if rect.rect.contains(gap) && rect.fill != Color32::TRANSPARENT => Some(rect.fill),
        Shape::Mesh(mesh) if mesh.calc_bounds().contains(gap) && mesh.texture_id == TextureId::default() => mesh.vertices.first().map(|v| v.color),
        _ => None,
    });
    assert_eq!(fill_at_gap, Some(Color32::WHITE), "the gap between two pages is white paper");
}

/// A message standing in for the sheet is read out as text where it is painted, inside the preview.
#[trial]
fn a_message_in_place_of_the_sheet_is_in_the_access_tree() {
    const WHY: &str = "The page range selects no pages.";
    let mut settings = Settings::default();
    let mut fields = DialogFields::default();
    let printers = vec![Printer::new("Office")];
    let mut fixture = Fixture::new(move |ui| {
        let sheet = PrintSheet::message(LETTER, WHY.into(), "No pages".into());
        let _ =
            PrintDialog::new(Bind::bound(&mut settings), &printers, Bind::bound(&mut fields)).sheet(&sheet).layout(PrintLayout::Document).show(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0));
    fixture.settle();
    let room = fixture.by_label("Print preview image").expect("the preview is in the access tree").rect;
    let message = fixture.by_label(WHY).expect("the message is in the access tree");
    assert!(room.expand(0.5).contains_rect(message.rect), "the message {:?} is read where the preview {room:?} paints it", message.rect);
}

/// Properties… on a hosted print dialog is filed in the pass that receives the press and taken once, in a frame that
/// runs twice too: a press taken again, or lost to the second pass, leaves the app opening the driver's options twice
/// or never.
#[trial]
fn a_properties_press_is_taken_once() {
    let twice = Rc::new(Cell::new(true));
    let (mut fixture, host) = held_print(&twice);
    on_print(&host, |dialog| dialog.set_driver(DriverControls::Properties));
    fixture.idle(4);
    assert_eq!(on_print(&host, HostedPrint::take_settings_clicked), Some(false), "nothing is pressed yet");
    let properties = fixture.by_label("Properties…").expect("the Properties… button");
    fixture.click(&properties);
    assert_eq!(fixture.passes(), 2, "the frame ran a second pass");
    fixture.idle(4);
    assert_eq!(on_print(&host, HostedPrint::take_settings_clicked), Some(true), "the press is filed for the app");
    assert_eq!(on_print(&host, HostedPrint::take_settings_clicked), Some(false), "the press is taken once");
    fixture.idle(4);
    assert_eq!(on_print(&host, HostedPrint::take_settings_clicked), Some(false), "later passes file no press");
}

/// The page box the hosted dialog drew at `scale` pixels per point, taken from the dialog.
fn paper_px_at(scale: f32) -> [u32; 2] {
    let twice = Rc::new(Cell::new(false));
    let (mut fixture, host) = held_print(&twice);
    fixture.scale(Scale::new(scale));
    fixture.idle(4);
    on_print(&host, HostedPrint::take_paper_px).flatten().expect("the page box is reported once drawn")
}

/// The preview reports the page box it drew, in physical pixels, so the app renders previews at that size. The preview's
/// width is fixed in points, so the same dialog at twice the scale has a box twice as large, within a pixel. A constant,
/// or a box in points, would not follow the scale.
#[trial]
fn the_paper_box_follows_the_scale() {
    let [one_w, one_h] = paper_px_at(1.0);
    let [two_w, two_h] = paper_px_at(2.0);
    assert!(one_w > 0 && one_h > 0, "a box at scale 1: {one_w}×{one_h}");
    assert!(two_w.abs_diff(one_w * 2) <= 1 && two_h.abs_diff(one_h * 2) <= 1, "{one_w}×{one_h} at 1.0, {two_w}×{two_h} at 2.0");
}

/// The page box is filed in the pass that draws it and taken once: `None` after, until the next pass that draws it.
#[trial]
fn the_paper_box_is_taken_once() {
    let twice = Rc::new(Cell::new(false));
    let (mut fixture, host) = held_print(&twice);
    let first = on_print(&host, HostedPrint::take_paper_px).flatten();
    assert!(first.is_some(), "the box is filed once the dialog is drawn");
    assert_eq!(on_print(&host, HostedPrint::take_paper_px), Some(None), "the box is taken once");
    fixture.idle(2);
    assert_eq!(on_print(&host, HostedPrint::take_paper_px).flatten(), first, "the next drawn pass files it again");
}

/// The page quads of a one-page sheet shown over `settings`.
fn page_tint(settings: Settings) -> Color32 {
    let made = Rc::new(Cell::new(None::<TextureId>));
    let seen = Rc::clone(&made);
    let mut settings = settings;
    let mut fields = DialogFields::default();
    let printers = vec![Printer::new("Office")];
    let mut fixture = Fixture::new(move |ui| {
        let texture = picture(ui).id;
        seen.set(Some(texture));
        let whole = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
        let page = SheetPage::new([pos2(72.0, 72.0), pos2(300.0, 72.0), pos2(300.0, 400.0), pos2(72.0, 400.0)], "1").picture(texture, whole);
        let sheet = PrintSheet::pages(LETTER, vec![page], "Letter".into());
        let _ =
            PrintDialog::new(Bind::bound(&mut settings), &printers, Bind::bound(&mut fields)).sheet(&sheet).layout(PrintLayout::Document).show(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0));
    fixture.settle();
    let texture = made.get().expect("the page picture was made");
    fixture
        .frame_shapes()
        .iter()
        .find_map(|shape| match shape {
            Shape::Mesh(mesh) if mesh.texture_id == texture => mesh.vertices.first().map(|vertex| vertex.color),
            _ => None,
        })
        .expect("the page is painted as a textured quad")
}

/// The preview of a job set to print in grayscale tints the page pictures grey, as production's does; a colour job
/// leaves the pictures' own colours.
#[trial]
fn the_preview_tints_the_pages_of_a_grayscale_job() {
    assert_eq!(page_tint(Settings::default()), Color32::WHITE, "a colour job shows the pictures as they are");
    let tint = page_tint(Settings { grayscale: true, ..Settings::default() });
    assert!(tint.r() == tint.g() && tint.g() == tint.b() && tint.r() < 250 && tint.r() > 150, "a grayscale job tints the pictures grey: {tint:?}");
}

/// A document print dialog over a Pages to Print section holding a radio of the same name.
fn document_dialog() -> Fixture {
    let mut settings = Settings::default();
    let mut fields = DialogFields::default();
    let printers = vec![Printer::new("Office")];
    let options = |names: [&'static str; 2]| names.into_iter().map(|name| (name.to_owned(), Label::from(name))).collect::<Vec<_>>();
    let sections = vec![PrintSection::new(
        "Pages to Print",
        vec![Param::new("which", "Pages to Print", ParamKind::radio(options(["All pages", "Current page"]), Some("All pages".into())))],
    )];
    let mut fixture = Fixture::new(move |ui| {
        let page = SheetPage::new([pos2(72.0, 72.0), pos2(300.0, 72.0), pos2(300.0, 400.0), pos2(72.0, 400.0)], "1");
        let sheet = PrintSheet::pages(LETTER, vec![page], "Letter".into());
        let _ = PrintDialog::new(Bind::bound(&mut settings), &printers, Bind::bound(&mut fields))
            .sheet(&sheet)
            .layout(PrintLayout::Document)
            .sections(&sections)
            .ready(true)
            .show(ui);
    });
    fixture.room(Vec2::new(1400.0, 900.0));
    fixture.settle();
    fixture
}

/// A document print dialog is compact, as production's: the printer rows head it with no "Printer Setup" heading and no
/// Refresh button, a section's radio group does not repeat the section's heading, and its options stand on one line.
#[trial]
fn a_document_print_dialog_has_compact_inline_rows() {
    let fixture = document_dialog();
    let texts = fixture.texts();
    let count = |text: &str| texts.iter().filter(|run| run.text == text).count();
    assert_eq!(count("Printer Setup"), 0, "the printer rows carry no heading");
    assert!(fixture.by_label("Refresh").is_none(), "the printers are found as the dialog opens: no Refresh");
    // The heading is an overline, in capitals (production's "PAGES TO PRINT"); the radio's caption is not repeated.
    assert_eq!(count("PAGES TO PRINT") + count("Pages to Print"), 1, "the heading is shown once, not again as the radio's caption");
    assert_eq!(count("PAGES TO PRINT"), 1, "the heading is an overline in capitals");
    let all = texts.iter().find(|run| run.text == "All pages").expect("the first option is painted");
    let current = texts.iter().find(|run| run.text == "Current page").expect("the second option is painted");
    assert!((all.rect.center().y - current.rect.center().y).abs() < 2.0, "the options stand on one line: {:?} {:?}", all.rect, current.rect);
}

/// The paper of the preview stands inset in its well, as production's does: the well shows round the sheet on every side.
#[trial]
fn the_paper_stands_inset_in_the_preview_well() {
    let mut settings = Settings::default();
    let mut fields = DialogFields::default();
    let printers = vec![Printer::new("Office")];
    let mut fixture = Fixture::new(move |ui| {
        let page = SheetPage::new([pos2(50.0, 100.0), pos2(250.0, 100.0), pos2(250.0, 400.0), pos2(50.0, 400.0)], "1");
        let sheet = PrintSheet::pages(LETTER, vec![page], "Letter".into());
        let _ =
            PrintDialog::new(Bind::bound(&mut settings), &printers, Bind::bound(&mut fields)).sheet(&sheet).layout(PrintLayout::Document).show(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0));
    fixture.theme(craft_theme::Mode::Dark, craft_theme::Contrast::Standard).settle();
    let room = fixture.by_label("Print preview image").expect("the preview is in the access tree").rect;
    let shapes = fixture.frame_shapes();
    let paper = shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Rect(rect) if rect.fill == Color32::WHITE && rect.rect.intersects(room) => Some(rect.rect),
            _ => None,
        })
        .max_by(|a, b| a.area().total_cmp(&b.area()))
        .expect("a white sheet is painted");
    assert!(paper.left() > room.left() + 2.0 && room.right() > paper.right() + 2.0, "the well shows beside the sheet: {room:?} {paper:?}");
    assert!(paper.top() > room.top() + 2.0 && room.bottom() > paper.bottom() + 2.0, "the well shows above and below the sheet: {room:?} {paper:?}");
}
