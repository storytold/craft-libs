//! The font picker's flows: style rows under an expanded family, served families and the rested report, and similar
//! fonts.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft_font_manager::craft_font_catalog::{Limits, Os, Source};
use craft_font_manager::picker::{FontFilters, FontPicker, FontPickerSettings, SIMILAR_COUNT, family_rows};
use craft_font_manager::{Catalog, FontEnv};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, Room, trial};
use egui::{Event, Key, Modifiers};

const INTER_REGULAR: &[u8] = include_bytes!("../../../ui-contract/fonts/inter/Inter-Regular.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../../ui-contract/fonts/inter/Inter-SemiBold.ttf");

fn catalog_of(faces: &[&[u8]]) -> Arc<Catalog> {
    let catalog = Arc::new(Catalog::new(FontEnv { os: Os::None, ..FontEnv::default() }).with_font_service(false));
    for bytes in faces {
        catalog.add_bytes(bytes.to_vec(), Source::Bundled, &Limits::default()).expect("a font file");
    }
    catalog
}

/// What the picker reported over the passes.
#[derive(Default)]
struct Seen {
    family: String,
    picks: Vec<(String, Option<String>)>,
    rested: Vec<String>,
}

fn rig(catalog: Arc<Catalog>, served: Vec<String>) -> (Fixture, Rc<RefCell<Seen>>) {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let shown = Rc::clone(&seen);
    let mut settings = FontPickerSettings::default();
    let mut fx = Fixture::new(move |ui| {
        let mut seen = shown.borrow_mut();
        let out =
            FontPicker::new("Font family", Bind::bound(&mut seen.family), &catalog).settings(Bind::bound(&mut settings)).served(&served).show(ui);
        if let Some(family) = out.picked {
            seen.picks.push((family, out.picked_style));
        }
        seen.rested.extend(out.rested);
    });
    fx.in_room(Room::Window);
    fx.settle();
    (fx, seen)
}

fn open(fx: &mut Fixture) {
    fx.key(Key::Tab, Modifiers::NONE);
    fx.key(Key::ArrowDown, Modifiers::ALT);
    fx.settle();
}

fn present(fx: &Fixture, text: &str) -> bool {
    fx.by_label(text).is_some() || fx.texts().iter().any(|painted| painted.text == text)
}

#[trial]
fn a_style_row_under_an_expanded_family_picks_family_and_style_once() {
    let (mut fx, seen) = rig(catalog_of(&[INTER_REGULAR, INTER_SEMIBOLD]), vec![]);
    open(&mut fx);
    assert!(!present(&fx, "SemiBold"), "styles hide until the family opens");
    // Focus stays in the field, whose caret keeps Left and Right: the family opens through its row's expand request.
    let inter = fx.by_label("Inter").expect("the Inter row");
    fx.access(AccessActionKind::Expand, &inter);
    fx.settle();
    assert!(present(&fx, "SemiBold"), "the open family lists its styles");

    let style = fx.by_label("SemiBold").expect("the style row");
    fx.click(&style);
    fx.settle();
    assert_eq!(seen.borrow().picks, [("Inter".to_owned(), Some("SemiBold".to_owned()))], "one commit with family and style");
    assert!(!present(&fx, "SemiBold"), "the list closed");

    open(&mut fx);
    assert!(!present(&fx, "SemiBold"), "the expanded set is gone when the list reopens");
}

#[trial]
fn a_served_family_lists_without_a_sample_and_is_reported_rested_once() {
    let (mut fx, seen) = rig(catalog_of(&[INTER_REGULAR]), vec!["Gotham".to_owned()]);
    open(&mut fx);
    assert!(present(&fx, "Gotham"), "the served family lists");
    assert!(present(&fx, "Not installed"), "with the served mark");

    // The pointer rests on Inter: reported after the rest delay.
    let inter = fx.by_label("Inter").expect("the Inter row");
    fx.frame(vec![Event::PointerMoved(inter.center())]);
    fx.advance(1.0);
    fx.settle();
    assert_eq!(seen.borrow().rested, ["Inter"]);

    // A key moves the lead and reports at once; a family already reported stays reported.
    fx.key(Key::ArrowUp, Modifiers::NONE);
    fx.key(Key::ArrowDown, Modifiers::NONE);
    fx.key(Key::ArrowUp, Modifiers::NONE);
    assert_eq!(seen.borrow().rested, ["Inter", "Gotham"]);
}

#[trial]
fn similar_fonts_put_the_current_family_first_and_leave_the_list_alone_without_a_reference() {
    let catalog = catalog_of(&[epaint_default_fonts::HACK_REGULAR, epaint_default_fonts::UBUNTU_LIGHT, INTER_REGULAR]);
    let mut similar = FontFilters::default();
    similar.similar = true;
    let rows = family_rows(&catalog, "", None, &similar, "Inter", "en");
    let names: Vec<&str> = rows.iter().map(|row| row.family.as_str()).collect();
    assert_eq!(names.first(), Some(&"Inter"), "the current family first, though it is not first by name");
    assert!(rows.len() <= SIMILAR_COUNT);

    // A current family the catalog lacks has no traits: the list stays as unfiltered.
    let unfiltered = family_rows(&catalog, "", None, &FontFilters::default(), "", "en");
    assert_eq!(family_rows(&catalog, "", None, &similar, "Missing", "en"), unfiltered);
}
