//! The print dialog's preview: the image box drag.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::EditEvent;
use craft_print::ui::{HostedPrint, PrintDialog, PrintSheet, Printer, Settings, State};
use craft_ui::contract::Bind;
use craft_ui::dialog::StackDialog as _;
use craft_ui::patterns::form_dialogs::{DialogFields, FieldValue};
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
        let out = PrintDialog::new(Bind::bound(&mut settings), &printers, Bind::bound(&mut fields), &sheet).ready(true).show(ui);
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
