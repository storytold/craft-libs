//! The hosted form dialogs on a `DialogHost`: each opens there, its Cancel reaches the app, and it opens again after
//! closing; OK answers with the dialog's own result type.

use craft_behavior::PixelSize;
use craft_ui::contract::Bind;
use craft_ui::dialog::{DialogHost, DialogResult, HeldOk, StackDialog};
use craft_ui::patterns::form_dialogs::{
    AboutInfo, AboutLink, CanvasSize, CanvasSizeValues, Credits, DialogFields, ExportOptions, ExportOptionsKind, FormatOptions, GuideLayoutDialog,
    GuideLayoutValues, GuideRect, GuideUnits, HostedAbout, HostedCanvasSize, HostedExportAs, HostedFormatOptions, HostedGuideLayoutDialog,
    HostedImageSize, HostedNewDocument, HostedNewGuideDialog, HostedParamDialog, ImageSize, ImageSizeValues, JpegOptions, NewDocumentDialog,
    NewGuideDialog, NewGuideValues, Param, ParamDialog, ParamKind,
};
use craft_ui::runtime::env::UnitKind;
use craft_ui::runtime::measure;
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
fn image_size_of(values: ImageSizeValues) -> HostedImageSize {
    HostedImageSize::new(values, |values| ImageSize::new(values))
}

fn image_size() -> HostedImageSize {
    image_size_of(ImageSizeValues::new(PICTURE))
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

/// A fixture that runs `host` in a measuring run, then in the real run, every pass, as a host inside a measured body
/// runs: the measuring run comes first, before the real run takes the pass's input.
pub(crate) fn measured_fixture(host: &Rc<RefCell<DialogHost>>) -> Fixture {
    let shown = Rc::clone(host);
    let mut fixture = Fixture::new(move |ui| {
        let ctx = ui.ctx().clone();
        let builder = egui::UiBuilder::new().sizing_pass().invisible();
        measure::run(&ctx, || ui.scope_builder(builder, |ui| shown.borrow_mut().run(ui)));
        shown.borrow_mut().run(ui);
    });
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

/// A path the host answers in a frame whose first run measures Export As still finishes the dialog with it. Guards
/// against a hosted dialog that took its host's answers in the measuring run, whose form drops them.
#[trial]
fn export_as_takes_a_path_answer_in_the_real_run_after_a_measuring_run() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = measured_fixture(&host);
    host.borrow_mut().open(HostedExportAs::new("Poster", PICTURE, ExportOptions::default()));
    fixture.idle(10);
    let export = fixture.by_label("Export…").expect("the Export button");
    fixture.click(&export).idle(4);
    let request = host.borrow_mut().get_mut::<HostedExportAs>().and_then(HostedExportAs::pick_path).expect("Export asks for a path");
    let path = PathBuf::from("/tmp/poster.png");
    if let Some(dialog) = host.borrow_mut().get_mut::<HostedExportAs>() {
        dialog.answer_path(request.key, path.clone());
    }
    fixture.idle(6);
    let answered = host.borrow_mut().take::<HostedExportAs>().and_then(DialogResult::ok).map(|result| result.path);
    assert_eq!(answered, Some(path), "the answered path finishes the dialog with it");
}

/// Export As: Enter alone does not export; Convert to sRGB starts on and its box gives the document's profile; + adds
/// 0.5x "@0.5x", then 0.75x, and a row's trash button removes it while more than one is left; two rows ask for a folder
/// and answer one output per row; without folder export, two rows refuse Export.
#[trial]
fn export_as_writes_one_file_per_scale_all_row_and_only_from_its_button() {
    use craft_ui::patterns::form_dialogs::{ExportProfile, ScaleRow, ScaleSize};
    use craft_ui::runtime::requests::PathKind;
    assert_eq!(ExportOptions::default().profile, ExportProfile::Srgb, "Convert to sRGB is on by default");
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    host.borrow_mut().open(HostedExportAs::new("Poster", PICTURE, ExportOptions::default()));
    fixture.idle(10);
    let options = |host: &Rc<RefCell<DialogHost>>| host.borrow().get::<HostedExportAs>().map(|dialog| dialog.options().clone()).expect("open");
    let pick = |host: &Rc<RefCell<DialogHost>>| host.borrow_mut().get_mut::<HostedExportAs>().and_then(HostedExportAs::pick_path);

    fixture.key(Key::Enter, Modifiers::NONE).idle(4);
    assert!(pick(&host).is_none() && host.borrow().is_open::<HostedExportAs>(), "Enter does not start the export");

    let srgb = fixture.by_label("Convert to sRGB").expect("the Color Space box");
    fixture.click(&srgb).idle(4);
    assert_eq!(options(&host).profile, ExportProfile::Document, "unticked keeps the document's profile");

    let one = fixture.by_label("Remove size 1").expect("the first row's trash button");
    fixture.click(&one).idle(4);
    assert_eq!(options(&host).scale_rows().len(), 1, "the last row stays");
    let add = fixture.by_label("Add a size").expect("the + button");
    fixture.click(&add).idle(4);
    let add = fixture.by_label("Add a size").expect("the + button");
    fixture.click(&add).idle(4);
    assert_eq!(
        options(&host).scale_all,
        [ScaleRow::default(), ScaleRow::new(ScaleSize::X0_5, "@0.5x"), ScaleRow::new(ScaleSize::X0_75, "@0.75x")],
        "+ adds the next sizes"
    );
    let third = fixture.by_label("Remove size 3").expect("the third row's trash button");
    fixture.click(&third).idle(4);
    assert_eq!(options(&host).scale_all.len(), 2);

    let export = fixture.by_label("Export…").expect("the Export button");
    fixture.click(&export).idle(4);
    let request = pick(&host).expect("Export asks where to write");
    assert!(matches!(request.kind, PathKind::Folder), "two rows ask for a folder");
    if let Some(dialog) = host.borrow_mut().get_mut::<HostedExportAs>() {
        dialog.answer_path(request.key.clone(), PathBuf::from("/tmp/out"));
    }
    fixture.idle(6);
    let outputs = host.borrow_mut().take::<HostedExportAs>().and_then(DialogResult::ok).map(|result| result.outputs).expect("answered");
    let listed: Vec<(PixelSize, &str)> = outputs.iter().map(|output| (output.size, output.suffix.as_str())).collect();
    assert_eq!(listed, [(PICTURE, ""), (PixelSize::new(800, 450), "@0.5x")], "one output per row");

    let mut two = ExportOptions::default();
    two.scale_all = vec![ScaleRow::default(), ScaleRow::new(ScaleSize::X2, "@2x")];
    host.borrow_mut().open(HostedExportAs::new("Poster", PICTURE, two).folder_export(false));
    fixture.idle(10);
    let export = fixture.by_label("Export…").expect("the Export button");
    fixture.click(&export).idle(4);
    let ctx = fixture.ctx().clone();
    assert!(pick(&host).is_none() && !host.borrow_mut().confirm::<HostedExportAs>(&ctx), "without a folder two rows refuse Export");
}

/// A layer export of several files asks for a folder where the app can write one, the browser's build keeps the save
/// request, and an export of one file asks for that file by name.
#[trial]
fn an_export_of_several_files_asks_for_a_folder() {
    use craft_ui::runtime::requests::PathKind;
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    let pick = |host: &Rc<RefCell<DialogHost>>| host.borrow_mut().get_mut::<HostedExportAs>().and_then(HostedExportAs::pick_path);
    for (files, folder_export, folder) in [(2, true, true), (2, false, false), (1, true, false)] {
        host.borrow_mut().open(HostedExportAs::new("Poster", PICTURE, ExportOptions::default()).files(files).folder_export(folder_export));
        fixture.idle(10);
        let export = fixture.by_label("Export…").expect("the Export button");
        fixture.click(&export).idle(4);
        let request = pick(&host).expect("Export asks where to write");
        let case = format!("{files} files, folder export {folder_export}");
        assert_eq!(matches!(request.kind, PathKind::Folder), folder, "{case}: a folder");
        if !folder {
            assert_eq!(request.file_name.as_deref(), Some("Poster.png"), "{case}: the file is named");
        }
        fixture.key(Key::Escape, Modifiers::NONE).idle(4);
        assert!(host.borrow().is_empty(), "{case}: Escape cancels");
    }
}

/// Enter in a field of Export As commits that field and leaves the dialog open. + adds sizes in their order up to the
/// row limit, then is disabled, and a suffix is cut at its limit.
#[trial]
fn export_as_keeps_scale_all_within_its_limits_and_enter_in_a_field_only_commits_it() {
    use craft_ui::patterns::form_dialogs::{ScaleRow, ScaleSize};
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    host.borrow_mut().open(HostedExportAs::new("Poster", PICTURE, ExportOptions::default()));
    fixture.settle();
    let options = |host: &Rc<RefCell<DialogHost>>| host.borrow().get::<HostedExportAs>().map(|dialog| dialog.options().clone()).expect("open");

    let scale = fixture.by_label("Scale").expect("the Scale field");
    fixture.click(&scale).settle();
    fixture.key(Key::A, Modifiers::COMMAND).type_text("50");
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert!(host.borrow().is_open::<HostedExportAs>(), "Enter in the Scale field leaves the dialog open");
    assert_eq!(options(&host).output_size(PICTURE), PixelSize::new(800, 450), "Enter commits the field");

    for _ in 1..ScaleRow::MAX_ROWS {
        let add = fixture.by_label("Add a size").expect("the + button");
        fixture.click(&add).settle();
    }
    let rows = options(&host).scale_rows();
    assert_eq!(rows.len(), ScaleRow::MAX_ROWS, "+ adds rows up to the limit");
    let added: Vec<ScaleSize> = rows.iter().skip(1).map(|row| row.size).collect();
    let order: Vec<ScaleSize> = ScaleSize::ADD_ORDER.iter().copied().filter(|size| *size != rows[0].size).take(added.len()).collect();
    assert_eq!(added, order, "+ adds the sizes in their order");
    let add = fixture.by_label("Add a size").expect("the + button");
    assert!(fixture.tree().is_disabled_key(add.node), "+ is disabled at the limit");

    let suffix = fixture.by_label("Suffix 2").expect("the second row's suffix");
    fixture.click(&suffix).settle();
    fixture.key(Key::A, Modifiers::COMMAND).type_text(&"s".repeat(ScaleRow::MAX_SUFFIX + 10));
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    assert_eq!(options(&host).scale_rows()[1].suffix.chars().count(), ScaleRow::MAX_SUFFIX, "a suffix is cut at its limit");
}

/// Image Size's readout counts the flat image in whole-byte rows (a Bitmap row of 9 pixels takes 2 bytes) and says
/// "(was …)" only once the pixel size differs from the document's.
#[trial]
fn image_size_readout_counts_whole_byte_rows_and_says_was_only_after_a_change() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    host.borrow_mut()
        .open(HostedImageSize::new(ImageSizeValues::new(PixelSize::new(9, 8)), |values| ImageSize::new(values).bytes_per_pixel(1.0 / 8.0)));
    fixture.settle();
    let texts = |fixture: &Fixture| fixture.texts().iter().map(|painted| painted.text.clone()).collect::<Vec<_>>();
    assert!(texts(&fixture).iter().any(|text| text == "16 bytes"), "2 bytes a row, 8 rows: {:?}", texts(&fixture));
    assert!(!texts(&fixture).iter().any(|text| text.contains("(was")), "no (was …) at the document's size");
    let width = fixture.by_label("Width").expect("the Width field");
    fixture.click(&width).settle();
    fixture.key(Key::A, Modifiers::COMMAND).type_text("18");
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    assert!(texts(&fixture).iter().any(|text| text.contains("(was 16 bytes)")), "{:?}", texts(&fixture));
}

/// Alt+1..8 pick the Resample method (#15054), each press with the text it typed taken out of the input, so the
/// focused field types nothing; a press that picks nothing takes its text too.
#[trial]
fn image_size_alt_digits_pick_resample_without_typing_into_a_field() {
    use craft_ui::patterns::form_dialogs::Resample;
    use craft_ui::testing::input::{key_press, text};
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    let resample = |host: &Rc<RefCell<DialogHost>>| {
        host.borrow().open_dialogs().into_iter().next().and_then(|open| open.fields).and_then(|fields| fields.get("resample").cloned())
    };
    let field_text = |fixture: &Fixture, label: &str| {
        let field = fixture.by_label(label).expect("the field");
        fixture.tree().value_of(field.node).unwrap_or_default().to_owned()
    };
    let alt_digit = |fixture: &mut Fixture, key: Key, typed: &str, modifiers: Modifiers| {
        fixture.frame(vec![key_press(key, modifiers), text(typed)]).settle();
    };
    let alt = Modifiers::ALT;

    host.borrow_mut().open(image_size());
    fixture.settle();
    let width = fixture.by_label("Width").expect("the Width field");
    fixture.click(&width).settle();
    let before = field_text(&fixture, "Width");
    alt_digit(&mut fixture, Key::Num7, "7", alt);
    assert_eq!(resample(&host), Some(json!("nearestNeighbor")), "Alt+7 picks Nearest Neighbor");
    assert_eq!(field_text(&fixture, "Width"), before, "Alt+7 types nothing into the focused field");
    alt_digit(&mut fixture, Key::Num8, "•", alt);
    assert_eq!(resample(&host), Some(json!("bilinear")), "Alt+8 picks Bilinear");
    alt_digit(&mut fixture, Key::Num3, "3", alt);
    assert_eq!(resample(&host), Some(json!("bilinear")), "Alt+3 picks nothing");
    assert_eq!(field_text(&fixture, "Width"), before, "Alt+3 types nothing either");
    alt_digit(&mut fixture, Key::Num7, "7", alt | Modifiers::CTRL);
    assert_eq!(resample(&host), Some(json!("bilinear")), "Ctrl+Alt+7 (AltGr) leaves the method alone");
    fixture.key(Key::A, Modifiers::COMMAND).type_text("50");
    assert_eq!(field_text(&fixture, "Width"), "50", "plain typing still edits the field");
    let ctx = fixture.ctx().clone();
    assert!(host.borrow_mut().cancel::<HostedImageSize>(&ctx), "the dialog closes");
    host.borrow_mut().take::<HostedImageSize>();
    fixture.settle();

    // Resample off: the keys pick nothing and type nothing into the resolution field.
    host.borrow_mut().open(image_size_of(ImageSizeValues::new(PICTURE).resampling(false)));
    fixture.settle();
    let resolution = fixture.by_label("Resolution").expect("the Resolution field");
    fixture.click(&resolution).settle();
    let before = field_text(&fixture, "Resolution");
    alt_digit(&mut fixture, Key::Num7, "7", alt);
    assert_eq!(resample(&host), Some(Value::Null), "Resample stays off");
    assert_eq!(field_text(&fixture, "Resolution"), before, "the resolution field types nothing");
    let ctx = fixture.ctx().clone();
    assert!(host.borrow_mut().cancel::<HostedImageSize>(&ctx), "the dialog closes");
    host.borrow_mut().take::<HostedImageSize>();
    fixture.settle();

    // A method the dialog does not offer is not picked.
    host.borrow_mut().open(HostedImageSize::new(ImageSizeValues::new(PICTURE), |values| {
        ImageSize::new(values).methods(vec![Resample::Automatic, Resample::NearestNeighbor])
    }));
    fixture.settle();
    alt_digit(&mut fixture, Key::Num5, "5", alt);
    assert_eq!(resample(&host), Some(json!("automatic")), "Alt+5 picks nothing when Bicubic Sharper is not listed");
    alt_digit(&mut fixture, Key::Num7, "7", alt);
    assert_eq!(resample(&host), Some(json!("nearestNeighbor")), "Alt+7 picks a listed method");
}

/// A dialog with one colour row whose dropper opens armed.
fn dropper(fields: &mut DialogFields) -> ParamDialog<'_> {
    let tint = Param::new("tint", "Tint", ParamKind::color(Some([0.0, 0.0, 1.0]))).eyedropper().armed(true);
    ParamDialog::new("dropper", "Dropper", vec![tint]).values(Bind::bound(fields))
}

// While a colour row samples, Escape ends the sampling and the dialog stays open with its edit; the next Escape cancels.
#[trial]
fn escape_ends_a_sampling_row_before_it_cancels_the_dialog() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "dropper", DialogFields::new(), dropper));
    fixture.idle(10);
    let ended = || host.borrow_mut().get_mut::<HostedParamDialog>().is_some_and(HostedParamDialog::take_sampling_ended);
    assert!(!ended(), "nothing ended yet");

    fixture.key(Key::Escape, Modifiers::NONE).idle(4);
    assert!(host.borrow().is_open::<HostedParamDialog>(), "the first Escape leaves the dialog open");
    assert!(ended(), "and ends the sampling");
    assert!(!ended(), "once");

    fixture.key(Key::Escape, Modifiers::NONE).idle(4);
    assert!(host.borrow().is_empty(), "the next Escape cancels the dialog");
}

/// Opens a dialog on a host.
type Opener = fn(&mut DialogHost);

/// Image Size's and Canvas Size's Width and Height rows (a stack of fields beside the link) show as they measured, at
/// 1x and 1.25x (#15483, #15538): each label sits beside its field, each field and unit select is as tall as a direct
/// row's control (`reference`), and Height's label starts under Width's controls.
#[trial]
fn sizing_dialogs_show_width_and_height_beside_their_labels_at_full_height() {
    for scale in [1.0, 1.25] {
        let opens: [(Opener, &str); 2] = [
            (
                |host| {
                    host.open(image_size());
                },
                "Resolution",
            ),
            (
                |host| {
                    host.open(canvas_size());
                },
                "Canvas extension color",
            ),
        ];
        for (open, reference) in opens {
            let host = Rc::new(RefCell::new(DialogHost::new()));
            let mut fixture = fixture(&host);
            fixture.pixels_per_point(scale);
            open(&mut host.borrow_mut());
            fixture.settle();
            let rect = |name: &str| fixture.by_label(name).unwrap_or_else(|| panic!("{name} is shown")).rect;
            let label_of = |name: &str| {
                // The field's label names the unit select's group, the number field's parent.
                let tree = fixture.tree();
                let mut node = fixture.by_label(name).unwrap_or_else(|| panic!("{name} is shown")).node;
                let label = loop {
                    if let Some(label) = tree.labelled_by_key(node).into_iter().next() {
                        break label;
                    }
                    node = tree.parent_of_key(node).unwrap_or_else(|| panic!("{name} is labelled"));
                };
                tree.bounds_of(label).unwrap_or_else(|| panic!("{name}'s label is shown"))
            };
            let full = rect(reference).height();
            for name in ["Width", "Height"] {
                let (label, field) = (label_of(name), rect(name));
                for part in [field, rect(&format!("{name} unit"))] {
                    assert!(part.height() >= full - 0.5, "{name}'s controls are a full row tall at {scale}x ({reference}): {part:?} {full}");
                }
                assert!(label.max.x <= field.min.x + 0.5, "{name}'s label sits before its field at {scale}x ({reference}): {label:?} {field:?}");
                assert!(
                    label.min.y < field.max.y && label.max.y > field.min.y,
                    "{name}'s label is on its field's line at {scale}x: {label:?} {field:?}"
                );
            }
            let (width, height) = (rect("Width").union(rect("Width unit")), label_of("Height"));
            assert!(height.min.y >= width.max.y - 0.5, "Height's row starts under Width's at {scale}x ({reference}): {height:?} {width:?}");
        }
    }
}

/// The horizontal line segments one frame of `fixture` paints, as (y, from x, to x).
fn horizontal_lines(fixture: &mut Fixture) -> Vec<(f32, f32, f32)> {
    fixture
        .frame_shapes()
        .into_iter()
        .filter_map(|shape| match shape {
            egui::Shape::LineSegment { points: [a, b], .. } if (a.y - b.y).abs() < 0.5 => Some((a.y, a.x.min(b.x), a.x.max(b.x))),
            _ => None,
        })
        .collect()
}

/// Image Size as 0.23.1 lays it out: the chain sits halfway between Width's and Height's controls with the bracket
/// drawn from each control to it, and Resample starts in the control column with the method on its line.
#[trial]
fn image_size_brackets_width_and_height_and_starts_resample_in_the_control_column() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    host.borrow_mut().open(image_size());
    fixture.settle();
    let rect = |fixture: &Fixture, name: &str| fixture.by_label(name).unwrap_or_else(|| panic!("{name} is shown")).rect;
    let (width, height, link) = (rect(&fixture, "Width unit"), rect(&fixture, "Height unit"), rect(&fixture, "Maintain aspect ratio"));
    let middle = (width.center().y + height.center().y) / 2.0;
    assert!((link.center().y - middle).abs() <= 0.5, "the chain sits halfway between Width and Height: {link:?} {middle}");
    let lines = horizontal_lines(&mut fixture);
    for (name, row) in [("Width", width), ("Height", height)] {
        assert!(
            lines.iter().any(|(y, from, to)| (y - row.center().y).abs() <= 1.0 && *from >= row.max.x && *to >= link.center().x - 1.0),
            "a tick runs from {name}'s control to the chain: {lines:?}"
        );
    }
    let (number, check, method) = (rect(&fixture, "Width"), rect(&fixture, "Resample"), rect(&fixture, "Resample method"));
    assert!((check.min.x - number.min.x).abs() <= 0.5, "Resample starts at the control column: {check:?} {number:?}");
    assert!(
        method.min.x >= check.max.x - 0.5 && method.min.y < check.max.y && method.max.y > check.min.y,
        "the method follows Resample on its line: {method:?} {check:?}"
    );
}

/// Canvas Size draws one rule between its title and Current Size, the title's: the first section's rule above would
/// stand right under it. The rule between the two sections stays.
#[trial]
fn canvas_size_draws_one_rule_under_its_title() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    host.borrow_mut().open(canvas_size());
    fixture.settle();
    let text = |fixture: &Fixture, text: &str| {
        fixture.texts().iter().find(|run| run.text == text).map(|run| run.rect).unwrap_or_else(|| panic!("{text} is painted"))
    };
    let (title, current, new) = (text(&fixture, "Canvas Size"), text(&fixture, "Current Size"), text(&fixture, "New Size"));
    let lines = horizontal_lines(&mut fixture);
    let across = |top: f32, bottom: f32| lines.iter().filter(|(y, from, to)| *y > top && *y < bottom && to - from > current.width()).count();
    assert_eq!(across(title.max.y, current.min.y), 1, "one rule between the title and Current Size: {lines:?}");
    assert_eq!(across(current.max.y, new.min.y), 1, "a rule between Current Size and New Size: {lines:?}");
}
