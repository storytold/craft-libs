//! The hosted Save PDF dialog on a `DialogHost`: the fields automation lists and sets, Alt-Reset, and the Summary text.

use craft_pdf::settings::{Compatibility, PdfPreset, PdfSettings, Standard};
use craft_ui::dialog::{DialogHost, DialogId};
use craft_ui::patterns::form_dialogs::{HostedPdfExport, PdfExportMode, PdfExportWords, summary_text};
use craft_ui::patterns::presets::PresetError;
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Fixture, trial};
use egui::{Context, Modifiers, Vec2};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;

/// A saved preset that differs from the defaults in its version and its trim marks.
fn mine() -> PdfPreset {
    let mut settings = PdfSettings { compatibility: Compatibility::Pdf15, ..PdfSettings::default() };
    settings.marks.trim = true;
    PdfPreset { name: "Mine".to_owned(), description: String::new(), settings }
}

/// A dialog that opens on `Mine` with its settings, over four artboards.
fn dialog() -> HostedPdfExport {
    HostedPdfExport::new("VectorCraft", mine().settings).saved_presets(vec![mine()]).preset("Mine").artboard_count(4)
}

/// The listed value of `field` of the open dialog.
fn listed(host: &DialogHost, id: DialogId, field: &str) -> Value {
    let fields = host.open_dialogs().into_iter().find(|open| open.id == id).and_then(|open| open.fields).unwrap_or(Value::Null);
    fields.pointer(&format!("/{}", field.replace('.', "/"))).cloned().unwrap_or(Value::Null)
}

/// Writes `value` to `field` of the open dialog `id`.
fn write_field(host: &mut DialogHost, ctx: &Context, id: DialogId, field: &str, value: Value) -> Result<(), String> {
    host.set_json_field(ctx, id, field, value)
}

#[trial]
fn the_hosted_dialog_takes_the_automation_fields() {
    let ctx = Context::default();
    let mut host = DialogHost::new();
    let id = host.open(dialog());
    for name in ["section", "preset", "artboards", "range", "bleedLinked", "marks", "compression"] {
        assert_ne!(listed(&host, id, name), Value::Null, "{name} is listed");
    }
    let set = |host: &mut DialogHost, field: &str, value: Value| write_field(host, &ctx, id, field, value);

    // The preset applies a listed name and refuses an unknown one.
    assert!(set(&mut host, "preset", json!("No such preset")).is_err());
    assert_eq!(listed(&host, id, "preset"), json!("Mine"));
    assert_eq!(set(&mut host, "marks.trim", json!(false)), Ok(()));
    assert_eq!(listed(&host, id, "marks.trim"), json!(false));
    assert_eq!(set(&mut host, "preset", json!("Mine")), Ok(()));
    assert_eq!(listed(&host, id, "marks.trim"), json!(true), "the preset's settings are back");

    // A nested write reaches the setting; an unknown step is refused.
    assert_eq!(set(&mut host, "marks.weight", json!(2.5)), Ok(()));
    assert_eq!(listed(&host, id, "marks.weight"), json!(2.5));
    assert!(set(&mut host, "marks.noSuchSetting", json!(1)).is_err());

    // The bleed link is read from the values: a non-bool is refused and `true` evens the four sides.
    assert_eq!(set(&mut host, "bleed.useDocument", json!(false)), Ok(()));
    assert_eq!(set(&mut host, "bleed.top", json!(3.0)), Ok(()));
    assert_eq!(listed(&host, id, "bleedLinked"), json!(false));
    assert!(set(&mut host, "bleedLinked", json!("yes")).is_err());
    assert_eq!(set(&mut host, "bleedLinked", json!(true)), Ok(()));
    assert_eq!(listed(&host, id, "bleed.right"), json!(3.0));
    assert_eq!(listed(&host, id, "bleedLinked"), json!(true));
}

#[trial]
fn the_hosted_dialog_takes_the_artboards_and_refuses_what_a_standard_disables() {
    let ctx = Context::default();
    let mut host = DialogHost::new();
    let id = host.open(dialog());
    let set = |host: &mut DialogHost, field: &str, value: Value| write_field(host, &ctx, id, field, value);

    // The artboards and the range.
    assert!(set(&mut host, "artboards", json!("everything")).is_err());
    assert_eq!(set(&mut host, "artboards", json!("range")), Ok(()));
    assert_eq!(set(&mut host, "range", json!("1-2")), Ok(()));
    assert_eq!(listed(&host, id, "artboards"), json!("range"));
    assert_eq!(listed(&host, id, "range"), json!("1-2"));
    assert_eq!(set(&mut host, "section", json!("security")), Ok(()));
    assert_eq!(listed(&host, id, "section"), json!("security"));

    // A write to a setting a standard disables is inactive and changes nothing; the standard moves the version.
    assert_eq!(set(&mut host, "standard", json!("pdfX3")), Ok(()));
    assert_eq!(listed(&host, id, "compatibility"), json!(Standard::PdfX3.version().id()));
    for field in ["longPagesInPoints", "output.profiles"] {
        let before = listed(&host, id, field);
        let refused = set(&mut host, field, json!(false)).expect_err(field);
        assert!(refused.contains("takes no input"), "{field}: {refused}");
        assert_eq!(listed(&host, id, field), before, "{field} is unchanged");
    }
}

/// Automation sets the two passwords the settings never serialise, as typing them does, never lists them, keeps them
/// through writes to the other settings, and is refused them under a standard, as the rows are.
#[trial]
fn the_hosted_dialog_takes_the_passwords_and_never_lists_them() {
    let ctx = Context::default();
    let mut host = DialogHost::new();
    let id = host.open(dialog());
    let set = |host: &mut DialogHost, field: &str, value: Value| write_field(host, &ctx, id, field, value);
    assert_eq!(set(&mut host, "security.openPassword", json!("open")), Ok(()));
    assert_eq!(set(&mut host, "security.permissionsPassword", json!("own")), Ok(()));
    assert!(set(&mut host, "security.openPassword", json!(7)).is_err(), "a password is text");
    assert_eq!(set(&mut host, "security.copy", json!(false)), Ok(()));
    let security = host.get_mut_by_id::<HostedPdfExport>(id).expect("open").settings().security.clone();
    assert_eq!((security.open_password.as_str(), security.permissions_password.as_str()), ("open", "own"), "kept through another write");
    assert_eq!(listed(&host, id, "security.openPassword"), Value::Null, "a password is never listed");
    assert_eq!(set(&mut host, "standard", json!("pdfX3")), Ok(()));
    assert!(set(&mut host, "security.openPassword", json!("other")).is_err(), "a standard takes no password");
}

/// OK files the settings for the app, which takes them once while the dialog waits for its answer.
#[trial]
fn ok_files_the_settings_once_for_the_app() {
    let ctx = Context::default();
    let mut host = DialogHost::new();
    let id = host.open(dialog());
    host.confirm_by_id(&ctx, id);
    let pdf = host.get_mut_by_id::<HostedPdfExport>(id).expect("the dialog stays open for the app's answer");
    let filed = pdf.take_ok().expect("OK filed an answer");
    assert_eq!(filed.result.options, mine().settings, "with the dialog's settings");
    assert!(pdf.take_ok().is_none(), "handed out once");
}

/// In the preset editor, OK by automation files nothing for an empty name or one the app's check refuses, as the
/// footer's OK is off then.
#[trial]
fn ok_in_the_preset_editor_files_nothing_for_a_refused_name() {
    let ctx = Context::default();
    let mut host = DialogHost::new();
    let editor = dialog().mode(PdfExportMode::PresetEditor).check_name(|name| (name == "Mine").then_some(PresetError::NameTaken));
    let id = host.open(editor);
    for (name, files) in [("", false), ("Mine", false), ("Fresh", true)] {
        assert_eq!(write_field(&mut host, &ctx, id, "name", json!(name)), Ok(()), "{name}");
        host.confirm_by_id(&ctx, id);
        let filed = host.get_mut_by_id::<HostedPdfExport>(id).expect("open").take_ok();
        assert_eq!(filed.is_some(), files, "{name:?}");
    }
}

#[trial]
fn reset_restores_the_chosen_preset_and_keeps_the_artboards() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let id = host.borrow_mut().open(dialog());
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(1400.0, 1000.0));
    fixture.idle(10);
    let edit = |host: &Rc<RefCell<DialogHost>>, ctx: &Context| {
        let mut host = host.borrow_mut();
        for (field, value) in [("marks.trim", json!(false)), ("range", json!("2")), ("artboards", json!("range")), ("section", json!("security"))] {
            assert_eq!(host.set_json_field(ctx, id, field, value), Ok(()), "{field}");
        }
    };
    edit(&host, fixture.ctx());
    fixture.idle(4);
    assert!(!host.borrow_mut().get_mut::<HostedPdfExport>().expect("open").settings().marks.trim);

    fixture.hold(Modifiers::ALT).idle(2);
    let reset = fixture.by_label("Reset").expect("Alt turns Cancel into Reset");
    fixture.click(&reset).idle(4);
    {
        let mut host = host.borrow_mut();
        let dialog = host.get_mut::<HostedPdfExport>().expect("Reset keeps the dialog open");
        assert_eq!(dialog.settings(), &mine().settings, "the chosen preset's settings are back");
    }
    assert_eq!(listed(&host.borrow(), id, "artboards"), json!("range"));
    assert_eq!(listed(&host.borrow(), id, "range"), json!("2"));
    assert_eq!(listed(&host.borrow(), id, "section"), json!("security"));

    // With the chosen preset gone from the list, Reset leaves the settings as they are.
    host.borrow_mut().get_mut::<HostedPdfExport>().expect("open").set_saved_presets(Vec::new());
    edit(&host, fixture.ctx());
    fixture.idle(4);
    let reset = fixture.by_label("Reset").expect("Reset while Alt is held");
    fixture.click(&reset).idle(4);
    let mut host = host.borrow_mut();
    assert!(!host.get_mut::<HostedPdfExport>().expect("open").settings().marks.trim, "an unlisted preset restores nothing");
}

/// Custom flattener options mark the chosen preset as modified, as a changed setting does, and turning them off shows
/// the preset's name again.
#[trial]
fn custom_flattener_options_mark_the_preset_as_modified() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    host.borrow_mut().open(dialog());
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(1400.0, 1000.0));
    fixture.idle(10);
    let painted = |fixture: &Fixture, text: &str| fixture.texts().iter().any(|run| run.text == text);
    assert!(painted(&fixture, "Mine") && !painted(&fixture, "Mine (modified)"), "the preset as it opens: {:?}", fixture.texts());

    let custom = |host: &Rc<RefCell<DialogHost>>, on: bool| host.borrow_mut().get_mut::<HostedPdfExport>().expect("open").set_custom_flattener(on);
    custom(&host, true);
    fixture.idle(4);
    assert!(painted(&fixture, "Mine (modified)"), "custom flattener options: {:?}", fixture.texts());
    custom(&host, false);
    fixture.idle(4);
    assert!(painted(&fixture, "Mine") && !painted(&fixture, "Mine (modified)"), "without them: {:?}", fixture.texts());
}

#[trial]
fn summary_text_writes_lf_whatever_the_values_hold() {
    let words = Rc::new(RefCell::new(None));
    let slot = Rc::clone(&words);
    let mut fixture = Fixture::new(move |ui| *slot.borrow_mut() = Some(Env::of(ui).words::<PdfExportWords>()));
    fixture.idle(1);
    let words = words.borrow().clone().expect("a pass ran");
    let mut settings = PdfSettings::default();
    settings.output.output_condition = "first\r\nsecond\rthird".to_owned();
    let text = summary_text(&words, &settings, &PdfSettings::default(), &[]);
    assert!(!text.contains('\r'), "no CR in {text:?}");
    assert!(text.ends_with('\n') && !text.ends_with("\n\n"), "one final LF in {text:?}");
    assert!(text.contains("first\nsecond\nthird"), "{text:?}");
}
