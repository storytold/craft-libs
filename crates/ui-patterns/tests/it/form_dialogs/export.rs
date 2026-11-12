//! The export dialogs' own logic: the settings a document can take and what Export As answers.

use craft_behavior::PixelSize;
use craft_ui::contract::{Bind, Role};
use craft_ui::patterns::form_dialogs::{
    ExportAsDialog, ExportOptions, ExportOptionsKind, FormatOptions, FormatOptionsDialog, JpegOptions, OutputPathRow,
};
use craft_ui::runtime::requests::PathRequest;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};
use std::cell::RefCell;
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

/// The default file name is the document's, made safe on every platform: reserved characters become underscores, dots and
/// spaces at the ends go, and a name with nothing left is Untitled.
#[trial]
fn the_default_file_name_is_made_safe() {
    assert_eq!(requested_file("Poster"), "Poster.png");
    assert_eq!(requested_file("  ..My: poster/v2?.. "), "My_ poster_v2_.png");
    assert_eq!(requested_file(" . "), "Untitled.png");
    let long = "x".repeat(100);
    assert_eq!(requested_file(&long).len(), 80 + ".png".len(), "at most 80 characters");
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
