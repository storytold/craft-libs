//! New Document: saved presets, the Save Document Preset pane, Bitmap's depth, the Web and Mobile cards' Artboards and
//! the pixel aspect ratio.

use craft_ui::contract::{Bind, SaveKey};
use craft_ui::patterns::form_dialogs::{BackgroundContents, BitDepth, ColorMode, DocumentOptions, NewDocumentDialog};
use craft_ui::patterns::presets::{GroupPath, NamedPresetEvent, PresetLibrary};
use craft_ui::runtime::memory::MemoryStage;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// Opens the Save Document Preset pane with the icon beside Name, types `name` over the proposed one and saves.
fn save_preset(fixture: &mut Fixture, name: &str) {
    let save = fixture.by_label("Save Document Preset").expect("the icon beside Name");
    fixture.click(&save).idle(10);
    assert!(fixture.by_label("Preset Name").is_some(), "the pane asks for a name");
    fixture.type_text(name).idle(4);
    let confirm = fixture.by_label("Save Preset").expect("the pane has Save Preset");
    fixture.click(&confirm).idle(10);
}

fn shows(fixture: &Fixture, text: &str) -> bool {
    fixture.texts().iter().any(|painted| painted.text == text)
}

#[trial]
fn a_saved_rows_delete_button_removes_that_preset() {
    let mut options = DocumentOptions::default();
    let mut fixture = Fixture::new(move |ui| {
        let _ = NewDocumentDialog::new(Bind::bound(&mut options)).show(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
    save_preset(&mut fixture, "Poster");
    save_preset(&mut fixture, "Flyer");
    assert!(shows(&fixture, "Poster") && shows(&fixture, "Flyer"), "the Saved tab lists both presets");
    let delete = fixture.by_label("Delete").expect("the first saved row has a Delete button");
    fixture.click(&delete).idle(10);
    assert!(!shows(&fixture, "Poster"), "the pressed row's preset is gone");
    assert!(shows(&fixture, "Flyer"), "the other preset stays");
}

#[trial]
fn a_bound_library_takes_the_saved_and_deleted_presets_and_reports_each_once() {
    let library = Rc::new(RefCell::new(PresetLibrary::<DocumentOptions>::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut options = DocumentOptions::default();
    let (held, reported) = (library.clone(), events.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut bound = held.borrow().clone();
        let shown = NewDocumentDialog::new(Bind::bound(&mut options)).saved(Bind::bound(&mut bound)).show(ui);
        reported.borrow_mut().extend(shown.presets);
        *held.borrow_mut() = bound;
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);

    save_preset(&mut fixture, "Poster");
    let names: Vec<String> = library.borrow().items().iter().map(|item| item.name.clone()).collect();
    assert_eq!(names, ["Poster"], "the preset went into the bound library");
    assert!(
        matches!(events.borrow().as_slice(), [NamedPresetEvent::Create { name, .. }] if name == "Poster"),
        "one create event, not one per pass or frame: {:?}",
        events.borrow()
    );

    let delete = fixture.by_label("Delete").expect("the saved row has a Delete button");
    fixture.click(&delete).idle(10);
    assert_eq!(library.borrow().total(), 0, "the delete went into the bound library");
    assert!(
        matches!(&events.borrow()[1..], [NamedPresetEvent::Delete(name)] if name == "Poster"),
        "one delete event keyed by the name before it: {:?}",
        events.borrow()
    );

    let kept: PresetLibrary<DocumentOptions> = MemoryStage::peek_saved(fixture.ctx(), &SaveKey::new("craft_ui.newDocument.presets"));
    assert_eq!(kept.total(), 0, "a bound library leaves the dialog's own saved list alone");
}

/// The icon beside Name opens the pane with the first free `Saved N` selected; typing replaces it, and Enter saves the
/// preset without creating the document.
#[trial]
fn the_save_icon_proposes_a_free_name_and_enter_saves_without_creating() {
    let library = Rc::new(RefCell::new(PresetLibrary::<DocumentOptions>::new()));
    library.borrow_mut().add(&GroupPath::root(), "Saved 1", DocumentOptions::default()).unwrap();
    let created = Rc::new(RefCell::new(false));
    let mut options = DocumentOptions::default();
    let (held, made) = (library.clone(), created.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut bound = held.borrow().clone();
        let shown = NewDocumentDialog::new(Bind::bound(&mut options)).saved(Bind::bound(&mut bound)).show(ui);
        *made.borrow_mut() |= shown.result.is_some();
        *held.borrow_mut() = bound;
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
    let save = fixture.by_label("Save Document Preset").expect("the icon beside Name");
    fixture.click(&save).idle(10);
    assert!(shows(&fixture, "Saved 2"), "the first free name is proposed");
    fixture.type_text("Poster").idle(4);
    fixture.key(Key::Enter, Modifiers::NONE).idle(10);
    let names: Vec<String> = library.borrow().items().iter().map(|item| item.name.clone()).collect();
    assert_eq!(names, ["Saved 1", "Poster"], "Enter saved the typed name");
    assert!(!*created.borrow(), "Enter in the pane does not create the document");
}

/// Bitmap takes only 1 bit and no transparent background, in the dialog and through automation; leaving it gives 8 bit.
#[trial]
fn bitmap_is_one_bit_without_a_transparent_background() {
    let options = |mode: ColorMode, depth: BitDepth| {
        let mut options = DocumentOptions::default();
        (options.color_mode, options.depth) = (mode, depth);
        options
    };
    let mut bitmap = options(ColorMode::Bitmap, BitDepth::Sixteen);
    bitmap.background = BackgroundContents::Transparent;
    let fitted = bitmap.fitted();
    assert_eq!((fitted.depth, fitted.background), (BitDepth::One, BackgroundContents::White));
    let back = options(ColorMode::Rgb, fitted.depth).fitted();
    assert_eq!(back.depth, BitDepth::Eight, "leaving Bitmap goes back to 8 bit");

    let shared = Rc::new(RefCell::new(options(ColorMode::Bitmap, BitDepth::Sixteen)));
    let held = shared.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut options = held.borrow().clone();
        let _ = NewDocumentDialog::new(Bind::bound(&mut options)).show(ui);
        *held.borrow_mut() = options;
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
    assert_eq!(shared.borrow().depth, BitDepth::One, "the dialog fits the depth to Bitmap");
    assert!(shows(&fixture, "1 bit"));
}

/// A Web card turns Artboards on and a Photo card turns it off again; Artboards the user ticked stays on.
#[trial]
fn web_cards_turn_artboards_on_and_only_take_back_their_own() {
    let shared = Rc::new(RefCell::new(DocumentOptions::default()));
    let held = shared.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut options = held.borrow().clone();
        let _ = NewDocumentDialog::new(Bind::bound(&mut options)).show(ui);
        *held.borrow_mut() = options;
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
    let pick = |fixture: &mut Fixture, tab: &str, card: &str| {
        let target = fixture.by_label(tab).unwrap_or_else(|| panic!("the {tab} tab"));
        fixture.click(&target).idle(10);
        // A card's node is named by its title and size line together; its title is clicked where it is painted.
        let title = fixture.texts().iter().find(|painted| painted.text == card).map(|painted| painted.rect.center());
        fixture.click_at(title.unwrap_or_else(|| panic!("the {card} card in {tab}"))).idle(10);
    };
    pick(&mut fixture, "Web", "Web Most Common");
    assert!(shared.borrow().artboard, "a Web card turns Artboards on");
    pick(&mut fixture, "Photo", "Landscape, 6 x 4");
    assert!(!shared.borrow().artboard, "a Photo card takes it back");
    let artboards = fixture.by_label("Artboards").expect("the Artboards box");
    fixture.click(&artboards).idle(10);
    pick(&mut fixture, "Photo", "Landscape, 7 x 5");
    assert!(shared.borrow().artboard, "the user's tick stays");
}

/// The Pixel Aspect Ratio row shows an id it no longer lists as Square Pixels, and the app's custom entries follow the
/// built-in ones.
#[trial]
fn pixel_aspect_lists_the_custom_entries_and_shows_an_unknown_id_as_square() {
    let mut options = DocumentOptions::default();
    options.pixel_aspect = "custom:1.5:Gone".to_owned();
    assert_eq!(DocumentOptions::default().pixel_aspect, "square");
    let mut fixture = Fixture::new(move |ui| {
        let _ =
            NewDocumentDialog::new(Bind::bound(&mut options)).custom_pixel_aspects(vec![("custom:1.2:Mine".to_owned(), "Mine".to_owned())]).show(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
    assert!(shows(&fixture, "Square Pixels"), "an unlisted id shows as Square Pixels");
    let row = fixture.by_label("Pixel Aspect Ratio").expect("the row under Color Profile");
    fixture.click(&row).idle(10);
    assert!(shows(&fixture, "DVCPRO HD 1080 (1.5)") && shows(&fixture, "Mine"), "the built-in ratios, then the app's");
}

/// The tab titles left to right, as painted.
fn tab_order(fixture: &Fixture) -> Vec<String> {
    let names = ["Recent", "Saved", "Photo", "Print", "Art & Illustration", "Web", "Mobile", "Film & Video"];
    let mut tabs: Vec<_> = fixture.texts().iter().filter(|painted| names.contains(&painted.text.as_str())).collect();
    tabs.sort_by(|a, b| a.rect.left().total_cmp(&b.rect.left()));
    tabs.dedup_by(|a, b| a.text == b.text);
    tabs.into_iter().map(|painted| painted.text.clone()).collect()
}

#[trial]
fn saved_after_recent_lists_saved_second() {
    // The strip shows the first tabs; the rest wait behind its overflow button.
    for (after_recent, expected) in [(false, ["Recent", "Photo", "Print"]), (true, ["Recent", "Saved", "Photo"])] {
        let mut options = DocumentOptions::default();
        let mut fixture = Fixture::new(move |ui| {
            let _ = NewDocumentDialog::new(Bind::bound(&mut options)).saved_after_recent(after_recent).show(ui);
        });
        fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
        assert_eq!(tab_order(&fixture)[..3], expected, "saved_after_recent({after_recent})");
    }
}

#[trial]
fn grey_create_on_empty_saved_greys_create_only_there() {
    for grey in [false, true] {
        let mut options = DocumentOptions::default();
        let created = Rc::new(RefCell::new(0));
        let count = created.clone();
        let mut fixture = Fixture::new(move |ui| {
            // Saved after Recent, so its tab is in the strip to click.
            let shown = NewDocumentDialog::new(Bind::bound(&mut options)).saved_after_recent(true).grey_create_on_empty_saved(grey).show(ui);
            *count.borrow_mut() += usize::from(shown.result.is_some());
        });
        fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
        let create_disabled = |fixture: &Fixture| fixture.tree().is_disabled_key(fixture.by_label("Create").expect("Create").node);
        assert!(!create_disabled(&fixture), "Recent keeps Create on");
        let saved = fixture.by_label("Saved").expect("the Saved tab");
        fixture.click(&saved).idle(10);
        assert!(shows(&fixture, "No saved presets yet."), "the Saved tab is empty");
        assert_eq!(create_disabled(&fixture), grey, "Create on an empty Saved tab, grey_create_on_empty_saved({grey})");
        if grey {
            let create = fixture.by_label("Create").expect("Create");
            fixture.click(&create).idle(10);
            assert_eq!(*created.borrow(), 0, "a greyed Create creates nothing");
            // A saved preset gives the tab something to create from.
            save_preset(&mut fixture, "Poster");
            assert!(!create_disabled(&fixture), "a saved preset turns Create back on");
        } else {
            let create = fixture.by_label("Create").expect("Create");
            fixture.click(&create).idle(10);
            assert_eq!(*created.borrow(), 1, "without the option Create creates from the details");
        }
    }
}
