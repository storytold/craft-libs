//! The font picker's flows: style rows under an expanded family, served families and the rested report, and similar
//! fonts.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use craft_behavior::{GESTURE_GAP, Preview};
use craft_font_manager::craft_font_catalog::{Limits, Os, Source};
use craft_font_manager::picker::{FamilySet, FontFilters, FontPicker, FontPickerSettings, FontScript, SIMILAR_COUNT, family_rows};
use craft_font_manager::{Catalog, FontEnv};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::runtime::frame::take_output;
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::pointer::{moved, wheel_lines};
use craft_ui::testing::{Fixture, Room, trial};
use craft_theme::{Contrast, Mode};
use egui::{Color32, Event, Key, Modifiers, Shape};

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
    rig_with(catalog, served, FontPickerSettings::default(), None)
}

/// A rig whose picker keeps `settings` and, when given, `filters`.
fn rig_with(catalog: Arc<Catalog>, served: Vec<String>, settings: FontPickerSettings, filters: Option<FontFilters>) -> (Fixture, Rc<RefCell<Seen>>) {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let shown = Rc::clone(&seen);
    let (mut settings, mut filters) = (settings, filters);
    let mut fx = Fixture::new(move |ui| {
        let mut seen = shown.borrow_mut();
        let mut picker =
            FontPicker::new("Font family", Bind::bound(&mut seen.family), &catalog).settings(Bind::bound(&mut settings)).served(&served);
        if let Some(filters) = filters.as_mut() {
            picker = picker.filters(Bind::bound(filters));
        }
        let out = picker.show(ui);
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
fn similar_fonts_put_the_reference_family_first_and_leave_the_list_alone_without_a_reference() {
    let catalog = catalog_of(&[epaint_default_fonts::HACK_REGULAR, epaint_default_fonts::UBUNTU_LIGHT, INTER_REGULAR]);
    let mut similar = FontFilters::default();
    similar.similar = Some("Inter".to_owned());
    let rows = family_rows(&catalog, "", None, &similar, &[], "en");
    let names: Vec<&str> = rows.iter().map(|row| row.family.as_str()).collect();
    assert_eq!(names.first(), Some(&"Inter"), "the reference family first, though it is not first by name");
    assert!(rows.len() <= SIMILAR_COUNT);

    // A reference family the catalog lacks has no traits: the list stays as unfiltered.
    let unfiltered = family_rows(&catalog, "", None, &FontFilters::default(), &[], "en");
    similar.similar = Some("Missing".to_owned());
    assert_eq!(family_rows(&catalog, "", None, &similar, &[], "en"), unfiltered);
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

fn three_faces() -> Arc<Catalog> {
    catalog_of(&[epaint_default_fonts::HACK_REGULAR, epaint_default_fonts::UBUNTU_LIGHT, INTER_REGULAR])
}

/// The families of `catalog` in list order under `filters` and `sets`.
fn listed(catalog: &Arc<Catalog>, filters: &FontFilters, sets: &[FamilySet<'_>]) -> Vec<String> {
    family_rows(catalog, "", None, filters, sets, "en").into_iter().map(|row| row.family).collect()
}

/// Focuses the field and rests the pointer on it, so the wheel turns over the closed field.
fn aim_at_field(fx: &mut Fixture) {
    let field = fx.by_label("Font family").expect("the family field");
    fx.key(Key::Tab, Modifiers::NONE);
    fx.frame(vec![moved(field.center())]);
}

#[trial]
fn the_closed_fields_wheel_steps_one_family_per_line_through_the_filtered_order_and_commits_one_gesture() {
    let all = listed(&three_faces(), &FontFilters::default(), &[]);
    assert_eq!(all.len(), 3);

    let (mut fx, seen) = rig(three_faces(), vec![]);
    seen.borrow_mut().family = all[0].clone();
    aim_at_field(&mut fx);
    fx.frame(vec![wheel_lines(-1.0), wheel_lines(1.0)]);
    fx.advance(GESTURE_GAP * 2.0).idle(3);
    assert_eq!(seen.borrow().family, all[0], "opposite notches in one pass step nothing");
    assert!(seen.borrow().picks.is_empty());

    fx.frame(vec![wheel_lines(-2.0)]);
    assert_eq!(seen.borrow().family, all[2], "two lines step two families");
    assert!(seen.borrow().picks.is_empty(), "nothing is picked while the wheel turns");
    fx.advance(GESTURE_GAP * 2.0).idle(3);
    assert_eq!(seen.borrow().picks, [(all[2].clone(), None)], "the run's end picks once");

    let settings = FontPickerSettings { favorites: vec![all[0].clone(), all[2].clone()], ..FontPickerSettings::default() };
    let mut favorites = FontFilters::default();
    favorites.favorites = true;
    let (mut fx, seen) = rig_with(three_faces(), vec![], settings, Some(favorites));
    seen.borrow_mut().family = all[0].clone();
    aim_at_field(&mut fx);
    fx.frame(vec![wheel_lines(-1.0)]);
    assert_eq!(seen.borrow().family, all[2], "a line steps over the family the favorites filter hides");
    fx.advance(GESTURE_GAP * 2.0).idle(3);
    assert_eq!(seen.borrow().picks, [(all[2].clone(), None)]);
}

#[trial]
fn a_script_filter_keeps_only_the_families_whose_face_covers_the_script() {
    let catalog = three_faces();
    let unfiltered = listed(&catalog, &FontFilters::default(), &[]);
    let with = |script: FontScript| {
        let mut filters = FontFilters::default();
        filters.script = Some(script);
        listed(&catalog, &filters, &[])
    };
    assert_eq!(with(FontScript::Latin), unfiltered, "every shipped face covers Latin");
    assert!(with(FontScript::Thai).is_empty(), "none covers Thai");
    assert!(with(FontScript::Japanese).is_empty(), "none covers Japanese");
}

#[trial]
fn a_host_set_keeps_only_its_families_and_an_ordered_one_lists_them_in_its_order() {
    let catalog = three_faces();
    let all = listed(&catalog, &FontFilters::default(), &[]);
    // The host keeps canonical lowercase names.
    let members = vec![all[2].to_ascii_lowercase(), all[0].to_ascii_lowercase()];
    let mut on = FontFilters::default();
    on.sets = vec!["document".to_owned()];

    let plain = [FamilySet::new("document", "Document", &members)];
    assert_eq!(listed(&catalog, &on, &plain), [all[0].clone(), all[2].clone()], "its families, in the name order");

    let ordered = [FamilySet::new("document", "Document", &members).ordered()];
    assert_eq!(listed(&catalog, &on, &ordered), [all[2].clone(), all[0].clone()], "its families, in its own order");

    let disabled = [FamilySet::new("document", "Document", &members).ordered().enabled(false)];
    assert_eq!(listed(&catalog, &on, &disabled), all, "a disabled set's key filters nothing");
    assert_eq!(listed(&catalog, &on, &[]), all, "a key naming no set filters nothing");
}

#[trial]
fn a_picker_that_stops_showing_with_a_preview_is_reported_once_in_ended_previews() {
    let catalog = catalog_of(&[INTER_REGULAR, epaint_default_fonts::HACK_REGULAR]);
    let shown = Rc::new(Cell::new(true));
    let id = Rc::new(Cell::new(None));
    let (visible, at) = (Rc::clone(&shown), Rc::clone(&id));
    let mut family = String::new();
    let mut fx = Fixture::new(move |ui| {
        if visible.get() {
            let out = FontPicker::new("Font family", Bind::bound(&mut family), &catalog).show(ui);
            at.set(Some(out.edit.response.id));
        }
    });
    fx.in_room(Room::Window);
    fx.settle();
    let hover_hack = |fx: &mut Fixture| {
        open(fx);
        let hack = fx.by_label("Hack").expect("the Hack row");
        fx.frame(vec![Event::PointerMoved(hack.center())]);
        fx.settle();
    };

    hover_hack(&mut fx);
    fx.key(Key::Escape, Modifiers::NONE);
    fx.settle();
    assert!(take_output(fx.ctx()).ended_previews.is_empty(), "a picker that ended its own preview names none");

    hover_hack(&mut fx);
    shown.set(false);
    fx.settle();
    fx.idle(3);
    let ended = take_output(fx.ctx()).ended_previews;
    assert_eq!(ended.iter().filter(|ended| Some(**ended) == id.get()).count(), 1, "the picker that stopped showing is named once: {ended:?}");
}

#[trial]
fn samples_on_white_paint_dark_ink_on_a_lighter_paper_in_a_dark_theme() {
    let white_papers = |on_white: bool| {
        let settings = FontPickerSettings { samples_on_white: on_white, ..FontPickerSettings::default() };
        let (mut fx, _) = rig_with(three_faces(), vec![], settings, None);
        fx.theme(Mode::Dark, Contrast::Standard);
        fx.settle();
        open(&mut fx);
        fx.frame_shapes().into_iter().filter(|shape| matches!(shape, Shape::Rect(rect) if rect.fill == Color32::WHITE)).count()
    };
    assert!(white_papers(true) >= 3, "each family's sample sits on a white paper, in a dark theme");
    assert_eq!(white_papers(false), 0, "without the setting no sample paper is white");
}
