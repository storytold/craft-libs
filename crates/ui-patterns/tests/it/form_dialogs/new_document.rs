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

/// Opens the Save Document Preset pane with the icon on the Name row, types `name` over the proposed one and saves.
fn save_preset(fixture: &mut Fixture, name: &str) {
    let save = fixture.by_label("Save Document Preset").expect("the Save Document Preset button");
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
    let save = fixture.by_label("Save Document Preset").expect("the Save Document Preset button");
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

#[trial]
fn recent_preset_cards_paint_one_small_page_at_its_aspect() {
    use craft_behavior::PixelSize;
    use craft_host::target::Os;
    use craft_ui::runtime::access::Role;
    use craft_ui::testing::{Frames, install_spec, pointer};
    use std::cell::Cell;

    let wide = PixelSize::new(1200, 900);
    let tall = PixelSize::new(600, 1200);
    for scale in [1.0, 1.25] {
        let mut frames = Frames::uninstalled();
        let mut install = install_spec();
        install.shell_info = install.shell_info.os(Os::Windows);
        craft_ui::install(&frames.ctx, install).expect("a fresh Windows-style desktop fixture");
        frames.screen(Vec2::new(1440.0, 900.0));
        frames.pixels_per_point(scale);
        let mut options = DocumentOptions::default();
        let mut recent = DocumentOptions::default();
        recent.size = tall;
        let mut recents = vec![recent];
        let selected_size = Cell::new(options.size);
        let mut show = |ui: &mut egui::Ui| {
            let _ = NewDocumentDialog::new(Bind::bound(&mut options)).clipboard(wide).recent(Bind::bound(&mut recents)).show(ui);
            selected_size.set(options.size);
        };
        for _ in 0..30 {
            frames.frame(Vec::new(), &mut show);
        }
        assert_eq!(frames.ctx.pixels_per_point(), scale);
        assert_eq!(selected_size.get(), wide, "Recent starts with the Clipboard preset");
        let tree = frames.tree();
        let card = |name: &str| {
            tree.with_role(Role::LIST_ITEM)
                .into_iter()
                .find(|key| tree.name_of(*key).is_some_and(|label| label.starts_with(name)))
                .expect("the preset card")
        };
        let clipboard = card("Clipboard");
        let custom = card("Custom");
        assert_eq!(tree.selected_of(clipboard), Some(true));
        let shapes = frames.frame_shapes(&mut show);
        for (name, key, size) in [("Clipboard", clipboard, wide), ("Custom", custom, tall)] {
            let card = tree.bounds_of(key).expect("visible tile bounds");
            let caption = frames.texts().iter().find(|line| line.text == name).expect("the preset title is painted");
            let outlines: Vec<_> = shapes
                .iter()
                .filter_map(|shape| match shape {
                    egui::Shape::Rect(rect)
                        if rect.stroke.width > 0.0
                            && (rect.rect.center().x - card.center().x).abs() <= 1.0
                            && card.contains_rect(rect.rect)
                            && rect.rect.bottom() <= caption.rect.top() =>
                    {
                        Some(rect.rect)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(outlines.len(), 1, "one page outline and no extra frame in {name} at {scale}: {outlines:?}");
            let page = outlines[0];
            assert!((page.width().max(page.height()) - 30.0).abs() <= 1.0, "the compact page outline in {name}: {page:?}");
            assert!((page.width() / page.height() - size.width as f32 / size.height as f32).abs() < 0.05, "the page has the preset's aspect");
        }

        let at = tree.bounds_of(custom).expect("the custom recent is visible").center();
        pointer::click(&mut frames, at, &mut show);
        assert_eq!(selected_size.get(), tall, "clicking the tall recent still loads its actual dimensions");
        let tree = frames.tree();
        assert_eq!(tree.selected_of(custom), Some(true));
        assert_eq!(tree.selected_of(clipboard), Some(false));
    }
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

#[trial]
fn the_new_document_layout_places_presets_and_details_below_the_tabs() {
    let mut options = DocumentOptions::default();
    let mut fixture = Fixture::new(move |ui| {
        let _ = NewDocumentDialog::new(Bind::bound(&mut options)).show(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
    let recent = fixture.by_label("Recent").expect("the Recent tab").rect;
    let mode = fixture.by_label("Color Mode").expect("the Color Mode field").rect;
    let depth = fixture.by_label("Depth").expect("the Depth field").rect;
    let save = fixture.by_label("Save Document Preset").expect("the save action").rect;
    let create = fixture.by_label("Create").expect("the create action").rect;
    assert!(shows(&fixture, "PRESET DETAILS"), "the details header is a caption");
    assert!(fixture.texts().iter().any(|t| t.text.starts_with("BLANK DOCUMENT PRESETS")), "the presets header is a caption");
    assert!(recent.bottom() <= mode.top(), "tabs go above the details");
    assert!((mode.top() - depth.top()).abs() < 1.0, "color mode and depth share a row");
    assert!(mode.right() <= depth.left(), "depth follows color mode");
    assert!(save.bottom() <= create.top(), "Save Preset is above the footer");
}

/// The fields run Name, size, Color Mode, Background Contents, then the advanced Color Profile and Pixel Aspect Ratio; the
/// save icon (no text) sits at the end of the Name row.
#[trial]
fn the_new_document_fields_run_background_before_the_advanced_ones_and_save_sits_on_the_name_row() {
    let mut options = DocumentOptions::default();
    let mut fixture = Fixture::new(move |ui| {
        let _ = NewDocumentDialog::new(Bind::bound(&mut options)).show(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
    let top = |fixture: &Fixture, label: &str| fixture.by_label(label).unwrap_or_else(|| panic!("the {label} field")).rect.top();
    let (mode, background) = (top(&fixture, "Color Mode"), top(&fixture, "Background Contents"));
    let (profile, aspect) = (top(&fixture, "Color Profile"), top(&fixture, "Pixel Aspect Ratio"));
    assert!(mode < background && background < profile && profile < aspect, "Background Contents comes before the advanced fields");

    let name = fixture.by_label("Name").expect("the Name field").rect;
    let save = fixture.by_label("Save Document Preset").expect("the save action").rect;
    assert!(save.top() < name.bottom() && name.top() < save.bottom(), "the save icon shares the Name row");
    assert!(name.right() <= save.left(), "the save icon is at the end of the Name field");
    assert!(!fixture.texts().iter().any(|run| run.text.starts_with("Save ")), "the save control is an icon, with no text");
}

/// The caption role sets its text in capitals when it paints; the label stays as written.
#[trial]
fn section_captions_paint_in_capitals_and_keep_their_text() {
    let mut options = DocumentOptions::default();
    let mut fixture = Fixture::new(move |ui| {
        let _ = NewDocumentDialog::new(Bind::bound(&mut options)).show(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
    assert!(shows(&fixture, "PRESET DETAILS"), "the caption paints in capitals");
    assert!(fixture.by_label("Preset Details").is_some(), "the accessible name keeps the text as written");
}

/// The preset cards fill the left column's width, and each card's title and size line are painted whole and centred
/// under the page outline.
#[trial]
fn the_preset_cards_fill_the_column_and_show_their_size_lines_whole() {
    use craft_ui::contract::Gap;
    use craft_ui::runtime::access::Role;
    use std::cell::Cell;

    let mut options = DocumentOptions::default();
    let column_gap = Rc::new(Cell::new(0.0f32));
    let shared = Rc::clone(&column_gap);
    let mut fixture = Fixture::new(move |ui| {
        shared.set(Gap::Row.points(ui));
        let _ = NewDocumentDialog::new(Bind::bound(&mut options)).show(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
    let photo = fixture.by_label("Photo").expect("the Photo tab");
    fixture.click(&photo).idle(10);
    let cards = fixture.by_role(Role::LIST_ITEM);
    assert!(cards.len() >= 3, "the tab lists presets");
    let first_row: Vec<_> = cards.iter().filter(|card| (card.rect.top() - cards[0].rect.top()).abs() < 1.0).collect();
    assert!(first_row.len() >= 2, "a row holds several cards");
    let details = fixture.by_label("Preset Details").expect("the details column").rect;
    let column_end = details.left() - column_gap.get();
    let last = first_row.last().expect("a row has cards").rect;
    assert!((column_end - last.right()).abs() <= 1.0, "the row's last card ends at the left column's end ({column_end}): {last:?}");
    for card in &first_row {
        let lines: Vec<_> = fixture.texts().iter().filter(|text| card.rect.contains_rect(text.rect)).collect();
        assert!(lines.iter().any(|text| text.text.contains('\u{d7}')), "the card shows its size line: {:?}", card.rect);
        for line in lines {
            assert!(!line.elided, "{:?} is painted whole", line.text);
            assert!((line.rect.center().x - card.rect.center().x).abs() <= 1.0, "{:?} is centred in its card", line.text);
        }
    }
}
