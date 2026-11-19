//! The font picker's flows: style rows under an expanded family, served families and the rested report, and similar
//! fonts.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft_behavior::Preview;
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
    /// The family the open list proposes as a live preview, as the last pass reported it.
    preview: Option<String>,
}

fn rig(catalog: Arc<Catalog>, served: Vec<String>) -> (Fixture, Rc<RefCell<Seen>>) {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let shown = Rc::clone(&seen);
    let mut settings = FontPickerSettings::default();
    let mut fx = Fixture::new(move |ui| {
        let mut seen = shown.borrow_mut();
        let out =
            FontPicker::new("Font family", Bind::bound(&mut seen.family), &catalog).settings(Bind::bound(&mut settings)).served(&served).show(ui);
        match out.preview {
            Preview::Shows(family) => seen.preview = Some(family),
            Preview::Ended => seen.preview = None,
            _ => {}
        }
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

#[trial]
fn the_row_under_the_pointer_is_proposed_as_the_preview_until_the_pointer_leaves_or_the_list_closes() {
    let (mut fx, seen) = rig(catalog_of(&[INTER_REGULAR, epaint_default_fonts::HACK_REGULAR]), vec![]);
    assert_eq!(seen.borrow().preview, None, "a closed list previews nothing");
    open(&mut fx);
    let hack = fx.by_label("Hack").expect("the Hack row");
    fx.frame(vec![Event::PointerMoved(hack.center())]);
    fx.settle();
    assert_eq!(seen.borrow().preview.as_deref(), Some("Hack"), "the hovered row previews its family");
    assert!(seen.borrow().picks.is_empty(), "a hover picks nothing");
    assert_ne!(seen.borrow().family, "Hack", "a preview leaves the value");

    fx.frame(vec![Event::PointerGone]);
    fx.settle();
    assert_eq!(seen.borrow().preview, None, "the preview ends when the pointer leaves the rows");
    let hack = fx.by_label("Hack").expect("the Hack row");
    fx.frame(vec![Event::PointerMoved(hack.center())]);
    fx.settle();
    assert_eq!(seen.borrow().preview.as_deref(), Some("Hack"), "the pointer back on the row previews it again");

    fx.key(Key::Escape, Modifiers::NONE);
    fx.settle();
    assert_eq!(seen.borrow().preview, None, "a list closed without a pick cancels the preview");
}

#[trial]
fn a_preview_over_a_typed_query_keeps_the_query_and_the_list() {
    let (mut fx, seen) = rig(catalog_of(&[INTER_REGULAR, epaint_default_fonts::HACK_REGULAR]), vec![]);
    let field = fx.by_label("Font family").expect("the family field");
    fx.click(&field);
    fx.settle();
    fx.type_text("Ha");
    fx.settle();
    let hack = fx.by_label("Hack").expect("the typed query lists Hack");
    fx.frame(vec![Event::PointerMoved(hack.center())]);
    fx.settle();
    assert_eq!(seen.borrow().preview.as_deref(), Some("Hack"), "the hovered row previews its family");
    assert!(present(&fx, "Ha"), "the field keeps the typed query while the row previews");
    assert!(fx.by_label("Hack").is_some(), "the list stays open on the query");

    fx.frame(vec![Event::PointerGone]);
    fx.settle();
    assert_eq!(seen.borrow().preview, None, "leaving the rows ends the preview");
    assert!(fx.by_label("Hack").is_some(), "the list stays open after the preview ends");
    assert!(present(&fx, "Ha"), "and the query stays");
}

#[trial]
fn a_click_on_the_focused_closed_field_selects_the_name_so_typing_replaces_it() {
    let (mut fx, seen) = rig(catalog_of(&[INTER_REGULAR, epaint_default_fonts::HACK_REGULAR]), vec![]);
    for family in ["Inter", "Hack"] {
        let field = fx.by_label("Font family").expect("the family field");
        fx.click(&field);
        fx.settle();
        fx.type_text(family);
        fx.settle();
        fx.key(Key::Enter, Modifiers::NONE);
        fx.settle();
        assert_eq!(seen.borrow().picks.last().map(|(picked, _)| picked.as_str()), Some(family), "the typed name is picked");
    }
}

#[trial]
fn classifies_japanese_and_cjk_families() {
    use craft_font_manager::picker::FontClass;
    const NONE: [u8; 10] = [0; 10];
    assert_eq!(FontClass::classify("MS Mincho", 0, NONE, false), Some(FontClass::Serif));
    assert_eq!(FontClass::classify("ＭＳ 明朝", 0, NONE, false), Some(FontClass::Serif));
    assert_eq!(FontClass::classify("Yu Gothic", 0, NONE, false), Some(FontClass::SansSerif));
    assert_eq!(FontClass::classify("游ゴシック", 0, NONE, false), Some(FontClass::SansSerif));
    assert_eq!(FontClass::classify("Meiryo", 0, NONE, false), Some(FontClass::SansSerif));
    assert_eq!(FontClass::classify("メイリオ", 0, NONE, false), Some(FontClass::SansSerif));
    assert_eq!(FontClass::classify("HG行書体", 0, NONE, false), Some(FontClass::Script));
    assert_eq!(FontClass::classify("HG正楷書体-PRO", 0, NONE, false), Some(FontClass::Script));
    assert_eq!(FontClass::classify("UD デジタル 教科書体", 0, NONE, false), Some(FontClass::Handwritten));
    assert_eq!(FontClass::classify("HG創英角ﾎﾟｯﾌﾟ体", 0, NONE, false), Some(FontClass::Handwritten));
    assert_eq!(FontClass::classify("MS Gothic 等幅", 0, NONE, false), Some(FontClass::Monospace));
    // A Latin word that merely ends in `song` is not a CJK serif (WindSong is a script family); the foundry stems
    // keep their Song/Songti reading.
    assert_eq!(FontClass::classify("WindSong", 0, NONE, false), None, "an English word ending in song is not a serif");
    assert_eq!(FontClass::classify("STSong", 0, NONE, false), Some(FontClass::Serif));
    assert_eq!(FontClass::classify("STZhongsong", 0, NONE, false), Some(FontClass::Serif));
    assert_eq!(FontClass::classify("FZFangSong-Z02", 0, NONE, false), Some(FontClass::Serif));
    // A generic `pop` fallback sits after the Script markers: `Pop Script` keeps Script, a bare `Pop` is the rounded
    // display face.
    assert_eq!(FontClass::classify("Pop Script", 0, NONE, false), Some(FontClass::Script));
    assert_eq!(FontClass::classify("Pop", 0, NONE, false), Some(FontClass::Handwritten));
    // A `hei` (黑) suffix is a CJK sans, but not against an explicit script marker: `Xhei Script` is a script.
    assert_eq!(FontClass::classify("SimHei", 0, NONE, false), Some(FontClass::SansSerif));
    assert_eq!(FontClass::classify("Xhei Script", 0, NONE, false), Some(FontClass::Script));
}
