//! The print dialog's preview: the image box drag.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::EditEvent;
use craft_print::ui::{HostedPrint, PrintDialog, PrintFinish, PrintSheet, Printer, Settings, State};
use craft_ui::contract::Bind;
use craft_ui::dialog::{DialogHost, HeldOk, StackDialog as _};
use craft_ui::patterns::form_dialogs::{DialogFields, FieldValue};
use craft_ui::patterns::presets::{NamedPresetEvent, PresetRow};
use craft_ui::primitives::TextureCache;
use craft_ui::testing::{Fixture, trial};
use egui::load::SizedTexture;
use egui::{Color32, ColorImage, Pos2, Rect, Ui, Vec2, pos2, vec2};

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
