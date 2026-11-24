//! The export dialogs' own logic: the settings a document can take and what Export As answers.

use craft_behavior::PixelSize;
use craft_ui::contract::{Bind, Role, SaveKey};
use craft_ui::dialog::{DialogHost, DialogResult, StackDialog};
use craft_ui::patterns::form_dialogs::{
    ExportAsDialog, ExportAsResult, ExportOptions, ExportOptionsKind, ExportSize, FormatOptions, FormatOptionsDialog, HostedExportAs,
    HostedFormatOptions, JpegOptions, OutputPathRow, Resample, ScaleRow, ScaleSize, SizeEstimate,
};
use craft_ui::runtime::requests::PathRequest;
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Vec2};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

fn document() -> PixelSize {
    PixelSize::new(1200, 800)
}

/// The file name Export asks the host to save under for a document called `name`.
fn requested_file(name: &str) -> String {
    let name = name.to_owned();
    let asked = Rc::new(RefCell::new(None::<PathRequest>));
    let request = Rc::clone(&asked);
    let mut options = ExportOptions::default();
    let mut fixture = Fixture::new(move |ui| {
        let mut out = ExportAsDialog::new(name.clone(), document(), Bind::bound(&mut options)).show(ui);
        if let Some(filed) = out.requests.take_path() {
            *request.borrow_mut() = Some(filed);
        }
    });
    fixture.settle();
    let export = fixture.by_label("Export…").expect("the Export button");
    fixture.click(&export);
    fixture.settle();
    let filed = asked.borrow().clone().expect("Export files a save request");
    filed.file_name.expect("the request names a file")
}

/// The suggested file name is the document's with what no file name holds turned into underscores; spaces and dots
/// at its ends stay, as production PhotoCraft keeps them, and a name of nothing but spaces and dots is Untitled.
#[trial]
fn the_suggested_file_name_keeps_its_ends_and_drops_what_no_file_name_holds() {
    assert_eq!(requested_file("Poster"), "Poster.png");
    assert_eq!(requested_file(" Artboard 1 "), " Artboard 1 .png");
    assert_eq!(requested_file(" a/b "), " a_b .png");
    assert_eq!(requested_file("a/b:c*?"), "a_b_c__.png");
    assert_eq!(requested_file(" . "), "Untitled.png");
    assert_eq!(requested_file("..."), "Untitled.png");
    let long = "x".repeat(100);
    assert_eq!(requested_file(&long).len(), 80 + ".png".len(), "at most 80 characters");
}

/// Export As picks its format from a select: JPG brings its Quality row. With Constrain Proportions on, a typed width
/// takes the height with it; off, the height stays.
#[trial]
fn export_as_picks_a_format_and_constrains_the_size_by_its_check_box() {
    let options = Rc::new(RefCell::new(ExportOptions::default()));
    let shared = Rc::clone(&options);
    let mut fixture = Fixture::new(move |ui| {
        let mut current = shared.borrow_mut();
        let _ = ExportAsDialog::new("Poster", document(), Bind::bound(&mut *current)).show(ui);
    });
    fixture.settle();
    assert!(fixture.by_label("Quality").is_none(), "PNG has no quality");
    let format = fixture.by_label("Format").expect("the format select");
    fixture.click(&format).settle();
    let jpg = fixture.by_label("JPG").expect("the select offers JPG");
    fixture.click(&jpg).settle();
    assert_eq!(options.borrow().format, "jpg");
    assert!(fixture.by_label("Quality").is_some(), "JPG shows its Quality row");

    let type_width = |fixture: &mut Fixture, text: &str| {
        let width = fixture.by_label("Width").expect("the width field");
        fixture.click(&width).settle();
        fixture.key(Key::A, Modifiers::COMMAND);
        fixture.type_text(text);
        fixture.key(Key::Enter, Modifiers::NONE).settle();
    };
    type_width(&mut fixture, "600");
    assert_eq!(options.borrow().size, ExportSize::pixels(600, 400), "constrained, the height follows the width");

    let constrain = fixture.by_label("Constrain Proportions").expect("the check box under Height");
    fixture.click(&constrain).settle();
    assert!(!options.borrow().lock_aspect, "the check box turns the constraint off");
    type_width(&mut fixture, "900");
    assert_eq!(options.borrow().size, ExportSize::pixels(900, 400), "unconstrained, the height stays");
}

/// The JPEG options dialog over `quality`, recording what OK answers.
fn jpeg_dialog(quality: u8) -> (Fixture, Rc<RefCell<Vec<FormatOptions>>>) {
    let answers = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&answers);
    let mut fixture = Fixture::new(move |ui| {
        let shown = FormatOptionsDialog::new(ExportOptionsKind::Jpeg).initial(FormatOptions::Jpeg(JpegOptions::new(quality))).show(ui);
        seen.borrow_mut().extend(shown.result);
    });
    fixture.idle(30);
    (fixture, answers)
}

fn ok(fixture: &mut Fixture) {
    let ok = fixture.by_label("OK").expect("the dialog shows OK");
    fixture.click(&ok).idle(10);
}

#[trial]
fn jpeg_quality_answers_its_opening_value_and_a_preset_pick() {
    let (mut fixture, answers) = jpeg_dialog(6);
    ok(&mut fixture);
    assert_eq!(answers.borrow().as_slice(), [FormatOptions::Jpeg(JpegOptions::new(6))], "OK without an edit answers the opening quality");

    let (mut fixture, answers) = jpeg_dialog(3);
    let preset = fixture.by_label("Preset").expect("the preset list sits beside the quality field");
    fixture.click(&preset).idle(10);
    let maximum = fixture.by_label("Maximum").expect("the list offers Maximum");
    fixture.click(&maximum).idle(10);
    ok(&mut fixture);
    assert_eq!(answers.borrow().as_slice(), [FormatOptions::Jpeg(JpegOptions::new(10))], "picking Maximum sets the quality to 10");
}

#[trial]
fn the_output_file_name_browses_on_a_click_and_edits_the_template_on_alt_click() {
    let path = Rc::new(RefCell::new("render.mov".to_owned()));
    let browsed = Rc::new(RefCell::new(0));
    let (held, count) = (Rc::clone(&path), Rc::clone(&browsed));
    let mut fixture = Fixture::new(move |ui| {
        let shown = OutputPathRow::new(Bind::bound(&mut *held.borrow_mut())).show(ui);
        *count.borrow_mut() += usize::from(shown.browse.is_some());
    });
    fixture.idle(10);
    let link = fixture.by_label("render.mov").expect("the file name is a link");
    fixture.click(&link).idle(4);
    assert_eq!(*browsed.borrow(), 1, "a click on the file name asks for the file dialog");

    fixture.hold(Modifiers::ALT);
    let link = fixture.by_label("render.mov").expect("the link stays after the click");
    fixture.click(&link).idle(2);
    fixture.hold(Modifiers::NONE).idle(4);
    assert_eq!(*browsed.borrow(), 1, "Alt-click opens no file dialog");
    assert!(fixture.by_role(Role::LINK).is_empty(), "Alt-click puts the template field in the link's place");
    fixture.type_text("_v2").idle(2);
    fixture.key(Key::Escape, Modifiers::NONE).idle(4);
    assert_eq!(*path.borrow(), "render.mov", "Escape leaves the value as it was");
    assert!(!fixture.by_role(Role::LINK).is_empty(), "Escape brings the link back");

    fixture.hold(Modifiers::ALT);
    let link = fixture.by_label("render.mov").expect("the link is back");
    fixture.click(&link).idle(2);
    fixture.hold(Modifiers::NONE).idle(4);
    fixture.type_text("_v2").idle(2);
    fixture.key(Key::Enter, Modifiers::NONE).idle(4);
    assert!(path.borrow().contains("_v2"), "Enter commits the typed template, got {}", path.borrow());
    assert!(!fixture.by_role(Role::LINK).is_empty(), "the committed template shows as the link");
}

/// A radio row is its own section: Targa Options shows "Resolution" once, as the title of the group it names.
#[trial]
fn a_radio_row_shows_its_name_once_as_the_title_of_its_group() {
    let mut fixture = Fixture::new(|ui| {
        FormatOptionsDialog::new(ExportOptionsKind::Targa).opened(Bind::given(true)).show(ui);
    });
    fixture.settle();
    let shown = fixture.texts().iter().filter(|run| run.text.trim_end_matches(':') == "Resolution").count();
    assert_eq!(shown, 1, "the name shows once");
    let groups = fixture.by_role(Role::RADIO_GROUP);
    let [group] = groups.as_slice() else { panic!("one radio group") };
    assert_eq!(fixture.tree().name_of(group.node), Some("Resolution"), "the group is named by the title");
}

fn painted(fixture: &Fixture) -> Vec<String> {
    fixture.texts().iter().map(|run| run.text.clone()).collect()
}

/// The footer note follows the answers for the current values: a request per revision, a stale answer dropped, and the
/// last answer kept while a newer revision is pending.
#[trial]
fn a_format_estimate_shows_only_the_answer_for_the_current_values() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(1400.0, 1000.0));
    let jpeg = HostedFormatOptions::new(fixture.ctx(), ExportOptionsKind::Jpeg).initial(FormatOptions::Jpeg(JpegOptions::new(6)));
    host.borrow_mut().open(jpeg);
    fixture.idle(10);
    let waiting = painted(&fixture);

    let wanted = |host: &Rc<RefCell<DialogHost>>| host.borrow_mut().get_mut::<HostedFormatOptions>().and_then(HostedFormatOptions::wanted_estimate);
    let answer = |host: &Rc<RefCell<DialogHost>>, revision: u64, estimate: Option<SizeEstimate>| {
        if let Some(dialog) = host.borrow_mut().get_mut::<HostedFormatOptions>() {
            dialog.set_estimate(revision, estimate);
        }
    };
    let first = wanted(&host).expect("the dialog asks for an estimate of its opening values").revision;
    fixture.idle(6);
    assert!(wanted(&host).is_none(), "the same values are asked for once");

    answer(&host, first, Some(SizeEstimate::Exact(5_000_000)));
    fixture.idle(4);
    let answered = painted(&fixture);
    assert_ne!(answered, waiting, "the answer replaces the footer note that waited for it");

    let sent = host.borrow_mut().get_mut::<HostedFormatOptions>().map(|dialog| dialog.set_json_field(fixture.ctx(), "format", "progressive".into()));
    assert_eq!(sent, Some(Ok(())), "the encoding is a field of the form");
    fixture.idle(4);
    let second = wanted(&host).expect("a changed value asks again").revision;
    assert!(second > first, "the new request has a newer revision");
    assert_eq!(painted(&fixture), answered, "while the new revision is pending, the last answer stays");

    answer(&host, first, Some(SizeEstimate::Exact(9_000_000_000)));
    fixture.idle(4);
    assert_eq!(painted(&fixture), answered, "an answer for an older revision is dropped");

    answer(&host, second, None);
    fixture.idle(4);
    let none = painted(&fixture);
    assert_ne!(none, answered, "the answer for the current values shows");

    answer(&host, first, Some(SizeEstimate::Exact(1)));
    fixture.idle(4);
    assert_eq!(painted(&fixture), none, "an older answer does not replace a newer one");
}

const REMEMBERED: SaveKey = SaveKey::new("test.export.remembered");

/// Opens Export As over `start` with the remembered key and exports to a path.
fn export_from(host: &Rc<RefCell<DialogHost>>, fixture: &mut Fixture, start: ExportOptions) -> ExportAsResult {
    host.borrow_mut().open(HostedExportAs::new("Poster", document(), start).remember(REMEMBERED));
    fixture.idle(10);
    let export = fixture.by_label("Export…").expect("the Export button");
    fixture.click(&export).idle(4);
    let request = host.borrow_mut().get_mut::<HostedExportAs>().and_then(HostedExportAs::pick_path).expect("Export asks for a path");
    if let Some(dialog) = host.borrow_mut().get_mut::<HostedExportAs>() {
        dialog.answer_path(request.key.clone(), PathBuf::from("/tmp/poster"));
    }
    fixture.idle(6);
    host.borrow_mut().take::<HostedExportAs>().and_then(DialogResult::ok).expect("the answered path finishes the dialog")
}

/// The options Export As opens with when the app gave `start`, then cancelled.
fn opened_with(host: &Rc<RefCell<DialogHost>>, fixture: &mut Fixture, start: ExportOptions) -> ExportOptions {
    host.borrow_mut().open(HostedExportAs::new("Poster", document(), start).remember(REMEMBERED));
    fixture.idle(10);
    let options = host.borrow_mut().get_mut::<HostedExportAs>().map(|dialog| dialog.options().clone()).expect("the dialog is open");
    fixture.key(Key::Escape, Modifiers::NONE).idle(4);
    assert!(host.borrow().is_empty(), "Escape closes the dialog");
    options
}

/// The default options with `format` at `quality`.
fn options(format: &str, quality: u8) -> ExportOptions {
    let mut options = ExportOptions::default();
    options.format = format.to_owned();
    options.quality = quality;
    options
}

#[trial]
fn export_as_starts_from_its_remembered_options_and_stores_the_confirmed_ones() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(1400.0, 1000.0));
    let mut jpeg = options("jpg", 40);
    jpeg.scale_all = vec![ScaleRow::default(), ScaleRow::new(ScaleSize::X2, "@2x")];

    let first = export_from(&host, &mut fixture, jpeg.clone());
    assert_eq!(first.options.format, "jpg", "nothing stored: the dialog exports what it was given");
    assert_eq!(opened_with(&host, &mut fixture, ExportOptions::default()).format, "png", "an export not stored leaves the next open as given");

    first.remember(fixture.ctx());
    let reopened = opened_with(&host, &mut fixture, ExportOptions::default());
    assert_eq!((reopened.format.as_str(), reopened.quality), ("jpg", 40), "the stored options replace the given ones at open");
    assert_eq!(reopened.scale_all, jpeg.scale_all, "the stored Scale All rows come back with them");

    let webp = options("webp", 55);
    let _unstored = export_from(&host, &mut fixture, webp);
    let again = opened_with(&host, &mut fixture, ExportOptions::default());
    assert_eq!((again.format.as_str(), again.quality), ("jpg", 40), "a result not stored leaves the stored options as they were");
    assert_eq!(again.scale_all, jpeg.scale_all, "and their Scale All rows");
}

/// Export As lists the app's resample methods in the order it gives, and an unlisted opening method opens as Automatic
/// when listed, else as the first listed one.
#[trial]
fn export_as_lists_resample_methods_in_the_hosts_order() {
    let methods = vec![Resample::Bicubic, Resample::BicubicSmoother, Resample::NearestNeighbor, Resample::PreserveDetails];
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(1400.0, 1000.0));
    let mut start = ExportOptions::default();
    start.resample = Resample::Lanczos;
    host.borrow_mut().open(HostedExportAs::new("Poster", document(), start).methods(methods.clone()));
    fixture.idle(10);
    let opened = host.borrow_mut().get_mut::<HostedExportAs>().map(|dialog| dialog.options().resample).expect("the dialog is open");
    assert_eq!(opened, Resample::Bicubic, "without Automatic in the list, the first listed method opens");

    let select = fixture.by_label("Resample").expect("the resample select");
    fixture.access(AccessActionKind::Focus, &select).idle(2);
    fixture.key(Key::Space, Modifiers::NONE).idle(3);
    let mut options = fixture.by_role(Role::LIST_BOX_OPTION);
    options.sort_by(|a, b| a.center().y.total_cmp(&b.center().y));
    let names: Vec<String> = options.iter().map(|option| fixture.tree().name_of(option.node).expect("the option has a name").to_owned()).collect();
    assert_eq!(names.len(), methods.len(), "the list offers the listed methods only");
    assert_eq!(
        names,
        ["Bicubic (smooth gradients)", "Bicubic Smoother (enlargement)", "Nearest Neighbor (hard edges)", "Preserve Details"],
        "the list keeps the host's order"
    );
}

/// Export As with the app's resample methods offers no other: an unlisted opening method opens as the first listed
/// one, and automation cannot set an unlisted one.
#[trial]
fn export_as_offers_only_the_apps_resample_methods() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(1400.0, 1000.0));
    let mut start = ExportOptions::default();
    start.resample = Resample::Lanczos;
    let methods = vec![Resample::Automatic, Resample::Bicubic];
    host.borrow_mut().open(HostedExportAs::new("Poster", document(), start).methods(methods));
    fixture.idle(10);
    let opened = host.borrow_mut().get_mut::<HostedExportAs>().map(|dialog| dialog.options().resample).expect("the dialog is open");
    assert_eq!(opened, Resample::Automatic, "an unlisted method opens as the first listed one");

    let mut host = host.borrow_mut();
    let dialog = host.get_mut::<HostedExportAs>().expect("the dialog is open");
    assert!(dialog.set_json_field(fixture.ctx(), "resample", "lanczos".into()).is_err(), "an unlisted method is refused");
    dialog.set_json_field(fixture.ctx(), "resample", "bicubic".into()).expect("a listed method is taken");
    assert_eq!(dialog.options().resample, Resample::Bicubic);
}
