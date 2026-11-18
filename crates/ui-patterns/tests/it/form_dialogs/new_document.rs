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

/// The names of the standard category `id`, in card order.
fn standard_names(id: &str) -> Vec<&'static str> {
    let categories = craft_ui::patterns::form_dialogs::PresetCategory::standard();
    categories.iter().find(|category| category.id == id).expect("a standard category").presets.iter().map(|preset| preset.name).collect()
}

/// The standard preset `name` of category `id`.
fn standard_preset(id: &str, name: &str) -> craft_ui::patterns::form_dialogs::DocumentPreset {
    let categories = craft_ui::patterns::form_dialogs::PresetCategory::standard();
    let category = categories.into_iter().find(|category| category.id == id).expect("a standard category");
    category.presets.into_iter().find(|preset| preset.name == name).expect("a standard preset")
}

#[trial]
fn photo_print_and_art_list_the_blank_document_presets() {
    assert_eq!(
        standard_names("photo"),
        [
            "Default Size",
            "Landscape, 3 x 2",
            "Landscape, 6 x 4",
            "Landscape, 7 x 5",
            "Landscape, 10 x 8",
            "Portrait, 2 x 3",
            "Portrait, 4 x 6",
            "Portrait, 5 x 7",
            "Portrait, 8 x 10"
        ]
    );
    assert_eq!(standard_names("print"), ["Letter", "Legal", "Tabloid", "A4", "A6", "A5", "A3", "B5", "B4", "B3", "C4", "C5", "C6", "DL"]);
    assert_eq!(standard_names("art"), ["1000 pixel grid", "2000 pixel grid", "Poster", "Postcard", "1080p", "720p"]);
    // ISO 216 / ISO 269 sizes in millimetres, at 300 ppi.
    let mm = |side: f64| (side / 25.4 * 300.0).round() as u32;
    for (name, width, height) in [
        ("A3", 297.0, 420.0),
        ("A4", 210.0, 297.0),
        ("A5", 148.0, 210.0),
        ("A6", 105.0, 148.0),
        ("B3", 353.0, 500.0),
        ("B4", 250.0, 353.0),
        ("B5", 176.0, 250.0),
        ("C4", 229.0, 324.0),
        ("C5", 162.0, 229.0),
        ("C6", 114.0, 162.0),
        ("DL", 110.0, 220.0),
    ] {
        let preset = standard_preset("print", name);
        assert_eq!((preset.width, preset.height, preset.pixels_per_inch), (mm(width), mm(height), 300.0), "{name}");
    }
    let photo = standard_preset("photo", "Portrait, 8 x 10");
    assert_eq!((photo.width, photo.height), (2400, 3000));
    let postcard = standard_preset("art", "Postcard");
    assert_eq!((postcard.width, postcard.height, postcard.pixels_per_inch), (1200, 1800, 300.0), "4 x 6 in, portrait");
    let video = standard_preset("art", "720p");
    assert_eq!((video.width, video.height, video.pixels_per_inch), (1280, 720, 72.0));
}

#[trial]
fn the_pixel_grids_keep_their_pixels_at_the_print_resolution() {
    use craft_ui::runtime::env::UnitKind;
    let grid = standard_preset("art", "1000 pixel grid");
    assert!(grid.is_print() && !grid.in_inches());
    for (print, screen) in [(300.0, 72.0), (200.0, 96.0)] {
        let options = grid.options(print, screen);
        assert_eq!(
            (options.size.width, options.size.height, options.pixels_per_inch, options.unit),
            (1000, 1000, print, UnitKind::Pixels),
            "{print} ppi"
        );
    }
    // A print preset keeps its inches at the print resolution; a screen preset its pixels at the screen resolution.
    let postcard = standard_preset("art", "Postcard").options(200.0, 96.0);
    assert_eq!((postcard.size.width, postcard.size.height, postcard.pixels_per_inch, postcard.unit), (800, 1200, 200.0, UnitKind::Inches));
    let video = standard_preset("art", "720p").options(200.0, 96.0);
    assert_eq!((video.size.width, video.size.height, video.pixels_per_inch, video.unit), (1280, 720, 96.0, UnitKind::Pixels));
}

#[trial]
fn the_art_tab_shows_the_grids_in_pixels_and_the_postcard_in_inches() {
    let mut options = DocumentOptions::default();
    let mut fixture = Fixture::new(move |ui| {
        let _ = NewDocumentDialog::new(Bind::bound(&mut options)).show(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
    let art = fixture.by_label("Art & Illustration").expect("the Art & Illustration tab");
    fixture.click(&art).idle(10);
    assert!(shows(&fixture, "1000 × 1000 px @ 300 ppi"), "the grid's card is in pixels");
    assert!(shows(&fixture, "4 × 6 in @ 300 ppi"), "the postcard's card is in inches");
}
