//! The hosted form dialogs on a `DialogHost`: each opens there, its Cancel reaches the app, and it opens again after
//! closing; OK answers with the dialog's own result type.

use craft_behavior::PixelSize;
use craft_ui::dialog::{DialogHost, DialogResult, HeldOk, StackDialog};
use craft_ui::patterns::form_dialogs::{
    AboutInfo, AboutLink, CanvasSize, CanvasSizeValues, Credits, ExportOptions, ExportOptionsKind, FormatOptions, GuideLayoutDialog,
    GuideLayoutValues, GuideRect, GuideUnits, HostedAbout, HostedCanvasSize, HostedExportAs, HostedFormatOptions, HostedGuideLayoutDialog,
    HostedImageSize, HostedNewDocument, HostedNewGuideDialog, ImageSize, ImageSizeValues, JpegOptions, NewDocumentDialog, NewGuideDialog,
    NewGuideValues,
};
use craft_ui::runtime::env::UnitKind;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Vec2};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::fmt::Debug;
use std::path::PathBuf;
use std::rc::Rc;

const PICTURE: PixelSize = PixelSize::new(1600, 900);

fn about() -> HostedAbout {
    let info = AboutInfo::new("PhotoCraft", "An image editor.", "1.0").links([AboutLink::new("Website", "https://example.org")]);
    HostedAbout::new(info, Credits::new(Vec::new(), Vec::new(), 0))
}

#[allow(clippy::redundant_closure, reason = "the builder must be generic over the bind's lifetime, which the bare path is not")]
fn canvas_size() -> HostedCanvasSize {
    HostedCanvasSize::new(CanvasSizeValues::new(PICTURE), |values| CanvasSize::new(values))
}

#[allow(clippy::redundant_closure, reason = "the builder must be generic over the bind's lifetime, which the bare path is not")]
fn image_size() -> HostedImageSize {
    HostedImageSize::new(ImageSizeValues::new(PICTURE), |values| ImageSize::new(values))
}

/// The document of the guide dialogs: 3000 × 2000 px at 300 ppi, measured in pixels.
fn guide_units() -> GuideUnits {
    GuideUnits::new(UnitKind::Pixels, 3000.0, 2000.0, 300.0)
}

fn guide_layout() -> HostedGuideLayoutDialog {
    HostedGuideLayoutDialog::new(GuideLayoutValues::default(), |values| {
        GuideLayoutDialog::new(guide_units(), GuideRect::new(0.0, 0.0, 3000.0, 2000.0), values)
    })
}

fn new_guide() -> HostedNewGuideDialog {
    HostedNewGuideDialog::new(NewGuideValues::default(), |values| NewGuideDialog::new(guide_units(), values))
}

/// Drives `dialog` as automation does, before any pass: the host lists its fields with `field` among them, an unknown
/// field is refused and changes nothing, `field` takes `value` and is listed with it, and OK answers. Returns the
/// answer.
fn automate<D: StackDialog>(dialog: D, field: &str, value: Value) -> Option<D::Result> {
    let ctx = egui::Context::default();
    let mut host = DialogHost::new();
    let id = host.open(dialog);
    let listed = |host: &DialogHost| host.open_dialogs().into_iter().find(|open| open.id == id).and_then(|open| open.fields);
    let before = listed(&host).unwrap_or(Value::Null);
    assert!(before.get(field).is_some(), "{field} is listed in {before}");
    assert!(host.set_json_field(&ctx, id, "noSuchField", json!(1)).is_err(), "an unknown field is refused");
    assert_eq!(listed(&host).unwrap_or(Value::Null), before, "a refused field changes nothing");
    assert_eq!(host.set_json_field(&ctx, id, field, value.clone()), Ok(()), "{field} takes {value}");
    assert_eq!(listed(&host).and_then(|fields| fields.get(field).cloned()), Some(value), "{field} is listed with the set value");
    assert!(host.confirm::<D>(&ctx), "OK is enabled");
    assert!(host.is_empty(), "OK closes the dialog");
    host.take::<D>().and_then(DialogResult::ok)
}

/// A fixture that runs `host` every pass, in a room large enough for every form.
fn fixture(host: &Rc<RefCell<DialogHost>>) -> Fixture {
    let shown = Rc::clone(host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(1400.0, 1000.0));
    fixture
}

/// Opens `make()` on a host, then Escape; twice, so a dialog that closed once opens again. `cancel` is what the app
/// takes after Escape.
fn opens_and_cancels<D: StackDialog>(make: fn(&egui::Context) -> D, title: &str, cancel: DialogResult<D::Result>)
where
    D::Result: Clone + Debug + PartialEq,
{
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    for round in ["first", "second"] {
        let made = make(fixture.ctx());
        host.borrow_mut().open(made);
        fixture.idle(10);
        assert!(fixture.by_label(title).is_some(), "{title} shows on the host ({round} open)");
        fixture.key(Key::Escape, Modifiers::NONE).idle(4);
        assert!(host.borrow().is_empty(), "Escape closes {title} ({round} open)");
        assert_eq!(host.borrow_mut().take::<D>(), Some(cancel.clone()), "{title}'s Escape reaches the app ({round} open)");
    }
}

#[trial]
fn each_hosted_form_opens_on_the_host_and_escape_reaches_the_app() {
    opens_and_cancels(|_| canvas_size(), "Canvas Size", DialogResult::Cancel);
    opens_and_cancels(|ctx| HostedFormatOptions::new(ctx, ExportOptionsKind::Jpeg), "JPEG Options", DialogResult::Cancel);
    opens_and_cancels(|_| HostedExportAs::new("Poster", PICTURE, ExportOptions::default()), "Export As", DialogResult::Cancel);
    // Escape closes About like OK: a notice has no Cancel.
    opens_and_cancels(|_| about(), "About PhotoCraft", DialogResult::Ok(()));
}

#[trial]
fn hosted_forms_answer_ok_with_their_values() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);

    host.borrow_mut().open(canvas_size());
    fixture.idle(10).key(Key::Enter, Modifiers::NONE).idle(4);
    let size = host.borrow_mut().take::<HostedCanvasSize>().and_then(DialogResult::ok).map(|result| result.size);
    assert_eq!(size, Some(PICTURE), "Enter answers Canvas Size with its size");

    let jpeg = HostedFormatOptions::new(fixture.ctx(), ExportOptionsKind::Jpeg).initial(FormatOptions::Jpeg(JpegOptions::new(10)));
    host.borrow_mut().open(jpeg);
    fixture.idle(10).key(Key::Enter, Modifiers::NONE).idle(4);
    let quality = match host.borrow_mut().take::<HostedFormatOptions>() {
        Some(DialogResult::Ok(FormatOptions::Jpeg(options))) => Some(options.quality),
        _ => None,
    };
    assert_eq!(quality, Some(10), "OK answers with the JPEG options it opened with");

    host.borrow_mut().open(about());
    fixture.idle(10);
    let website = fixture.by_label("Website").expect("About shows its link");
    fixture.click(&website).idle(4);
    let link = host.borrow_mut().get_mut::<HostedAbout>().and_then(HostedAbout::take_link);
    assert_eq!(link.as_deref(), Some("https://example.org"), "the link waits for the app");

    // PatternFillDialog and CreateShapeDialog live in craft-draw, which depends on this crate; their automation is
    // driven in craft-draw's tests/it (shape_drag.rs, pattern_fill.rs).
    let image = automate(image_size(), "width", json!(800.0)).map(|result| result.size);
    assert_eq!(image, Some(PixelSize::new(800, 450)), "Image Size answers the set width, the height following the lock");
    let canvas = automate(canvas_size(), "width", json!(2000.0)).map(|result| result.size);
    assert_eq!(canvas, Some(PixelSize::new(2000, 900)), "Canvas Size answers the set width; its sides are not linked");
    let layout = automate(guide_layout(), "centerColumns", json!(true)).map(|result| (result.options.center_columns, result.guides.is_empty()));
    assert_eq!(layout, Some((true, false)), "Guide Layout answers its options and the guides they make");
    let guide = automate(new_guide(), "position", json!(250.0)).map(|guide| guide.position);
    assert_eq!(guide, Some(250.0), "New Guide answers the guide at the set position");
}

// The JPEG quality row lists the quality the dialog opened with from the moment it opens, a set made before the first
// pass stays, and the quality automation sets, kept in the form automation gives it, is the one OK answers.
#[trial]
fn the_jpeg_quality_lists_its_opening_value_and_ok_answers_a_set_one() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    let ctx = fixture.ctx().clone();
    let id = host.borrow_mut().open(HostedFormatOptions::new(&ctx, ExportOptionsKind::Jpeg).initial(FormatOptions::Jpeg(JpegOptions::new(10))));
    let quality = |host: &Rc<RefCell<DialogHost>>| {
        host.borrow().open_dialogs().into_iter().find(|open| open.id == id).and_then(|open| open.fields).and_then(|fields| fields["quality"].as_u64())
    };
    assert_eq!(quality(&host), Some(10), "the row lists the start quality before the first pass");
    assert_eq!(host.borrow_mut().set_json_field(&ctx, id, "quality", json!(3)), Ok(()));
    fixture.idle(2);
    assert_eq!(quality(&host), Some(3), "the set made before the first pass stays");
    assert!(host.borrow_mut().confirm::<HostedFormatOptions>(&ctx), "OK is enabled");
    let answered = match host.borrow_mut().take::<HostedFormatOptions>() {
        Some(DialogResult::Ok(FormatOptions::Jpeg(options))) => Some(options.quality),
        _ => None,
    };
    assert_eq!(answered, Some(3), "OK answers the set quality");
}

// New Document opened with an image on the clipboard lists the clipboard's size and the default name from the moment
// it opens, and a size automation sets before the first pass is the one the dialog shows and Create answers.
#[trial]
fn new_document_lists_its_clipboard_size_at_once_and_keeps_a_set_made_before_the_first_pass() {
    use craft_ui::contract::Bind;
    use craft_ui::patterns::form_dialogs::{DocumentOptions, NewDocumentResult};

    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    let ctx = fixture.ctx().clone();
    let clipboard = PixelSize::new(640, 480);
    let dialog =
        HostedNewDocument::new(&ctx, DocumentOptions::default(), move |options| NewDocumentDialog::new(Bind::bound(options)).clipboard(clipboard));
    let id = host.borrow_mut().open(dialog);
    let fields = |host: &Rc<RefCell<DialogHost>>| host.borrow().open_dialogs().into_iter().find(|open| open.id == id).and_then(|open| open.fields);
    let opened = fields(&host).expect("the dialog lists its settings");
    assert_eq!(
        (opened["size"]["width"].as_u64(), opened["size"]["height"].as_u64()),
        (Some(640), Some(480)),
        "the clipboard's size, before the first pass"
    );
    assert!(opened["name"].as_str().is_some_and(|name| !name.is_empty()), "the default name, before the first pass");
    assert_eq!(host.borrow_mut().set_json_field(&ctx, id, "size", json!({"width": 800, "height": 600})), Ok(()));
    fixture.idle(10);
    let shown = fields(&host).expect("the dialog lists its settings");
    assert_eq!((shown["size"]["width"].as_u64(), shown["size"]["height"].as_u64()), (Some(800), Some(600)), "the first pass keeps the set size");
    assert!(host.borrow_mut().confirm::<HostedNewDocument>(&ctx), "Create is enabled");
    let created = host.borrow_mut().take::<HostedNewDocument>().and_then(DialogResult::ok).map(|result: NewDocumentResult| result.options.size);
    assert_eq!(created, Some(PixelSize::new(800, 600)), "Create answers the set size");
}

// New Guide's OK refuses while the Position field holds text that does not parse, rather than answering the last good
// position; once a parsing text is committed, OK answers it.
#[trial]
fn new_guide_refuses_ok_while_the_position_text_does_not_parse() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    host.borrow_mut().open(new_guide());
    fixture.settle();
    let ctx = fixture.ctx().clone();
    let type_position = |fixture: &mut Fixture, text: &str| {
        let position = fixture.by_label("Position").expect("the Position field");
        fixture.click(&position).settle();
        fixture.key(Key::A, Modifiers::COMMAND).type_text(text);
        fixture.settle();
    };
    type_position(&mut fixture, "far");
    assert!(!host.borrow_mut().confirm::<HostedNewGuideDialog>(&ctx), "text that does not parse is refused");
    assert!(!host.borrow().is_empty(), "the dialog stays open");
    type_position(&mut fixture, "40");
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    assert!(host.borrow_mut().confirm::<HostedNewGuideDialog>(&ctx), "text that parses is taken");
    let guide = host.borrow_mut().take::<HostedNewGuideDialog>().and_then(DialogResult::ok).map(|guide| guide.position);
    assert_eq!(guide, Some(40.0));
}

#[trial]
fn hosted_export_as_finishes_when_the_app_answers_its_save_request() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    host.borrow_mut().open(HostedExportAs::new("Poster", PICTURE, ExportOptions::default()));
    fixture.idle(10);
    assert!(host.borrow_mut().get_mut::<HostedExportAs>().and_then(HostedExportAs::wanted_preview).is_some(), "the dialog asks for a preview");

    let export = fixture.by_label("Export…").expect("the Export button");
    fixture.click(&export).idle(4);
    let request = host.borrow_mut().get_mut::<HostedExportAs>().and_then(HostedExportAs::pick_path).expect("Export asks for a path");
    assert!(host.borrow().is_open::<HostedExportAs>(), "the dialog waits for the path");
    let id = host.borrow().get::<HostedExportAs>().map(StackDialog::id).expect("open");
    assert_eq!(host.borrow().held_state(id), Some(HeldOk::Waiting), "the OK waits for the save dialog");
    if let Some(dialog) = host.borrow_mut().get_mut::<HostedExportAs>() {
        dialog.cancel_requests();
    }
    let ctx = fixture.ctx().clone();
    assert!(!host.borrow_mut().confirm::<HostedExportAs>(&ctx), "automation's OK without a path waits for one");
    let asked =
        host.borrow_mut().get_mut::<HostedExportAs>().and_then(HostedExportAs::pick_path).expect("automation's OK asks for a path as Export does");
    assert_eq!(asked.key, request.key, "the same save request as the button's");

    let path = PathBuf::from("/tmp/poster.png");
    if let Some(dialog) = host.borrow_mut().get_mut::<HostedExportAs>() {
        dialog.answer_path(request.key.clone(), path.clone());
    }
    fixture.idle(6);
    let answered = host.borrow_mut().take::<HostedExportAs>().and_then(DialogResult::ok).map(|result| result.path);
    assert_eq!(answered, Some(path), "the answered path finishes the dialog with it");
}
