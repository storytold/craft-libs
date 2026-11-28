//! The font picker's flows: style rows under an expanded family, served families and the rested report, similar
//! fonts, and the host's rows.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use craft_behavior::{GESTURE_GAP, PartKey, Preview};
use craft_font_manager::craft_font_catalog::{Limits, Os, Source};
use craft_font_manager::missing::MissingFontsWords;
use craft_font_manager::picker::{
    FamilyInfos, FamilySet, FilterLayout, FontClass, FontFilters, FontPicker, FontPickerKind, FontPickerSettings, FontPickerWords, FontPreviewSize,
    FontScript, HostFontGroup, HostFontRow, MissingAdvice, SIMILAR_COUNT, SampleText, family_rows,
};
use craft_font_manager::{Catalog, FontEnv};
use craft_theme::{Contrast, Mode};
use craft_ui::buttons::ControlPlace;
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::runtime::env::{Label, with_env};
use craft_ui::runtime::frame::take_output;
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::pointer::{moved, wheel_lines};
use craft_ui::testing::{Fixture, Room, last_frame_derived_builds, trial};
use egui::{Color32, Event, Key, Modifiers, Shape};

const INTER_REGULAR: &[u8] = include_bytes!("../../../ui-contract/fonts/inter/Inter-Regular.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../../ui-contract/fonts/inter/Inter-SemiBold.ttf");

fn catalog_of(faces: &[&[u8]]) -> Arc<Catalog> {
    let catalog = Arc::new(Catalog::new(FontEnv { os: Os::None, ..FontEnv::default() }).with_font_service(false));
    for bytes in faces {
        catalog.add_bytes(bytes.to_vec(), Source::Bundled, &Limits::default()).expect("a font file");
    }
    catalog.rescan();
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
    actions: Vec<usize>,
    /// The id of the shown picker, as the last pass reported it.
    id: Option<egui::Id>,
    /// The keys of the host rows picked, in order.
    host_picks: Vec<String>,
}

fn rig(catalog: Arc<Catalog>, served: Vec<String>) -> (Fixture, Rc<RefCell<Seen>>) {
    rig_full(catalog, served, FontPickerSettings::default(), None, Vec::new(), None)
}

/// A rig whose picker keeps `settings` and, when given, `filters`.
fn rig_with(catalog: Arc<Catalog>, served: Vec<String>, settings: FontPickerSettings, filters: Option<FontFilters>) -> (Fixture, Rc<RefCell<Seen>>) {
    rig_full(catalog, served, settings, filters, Vec::new(), None)
}

/// A picker with one host group (without a heading) per entry of `host`, showing the host row `current_host` as its
/// value when given.
fn rig_with_host(
    catalog: Arc<Catalog>,
    served: Vec<String>,
    host: Vec<Vec<HostFontRow>>,
    current_host: Option<String>,
) -> (Fixture, Rc<RefCell<Seen>>) {
    rig_full(catalog, served, FontPickerSettings::default(), None, host, current_host)
}

fn rig_full(
    catalog: Arc<Catalog>,
    served: Vec<String>,
    settings: FontPickerSettings,
    filters: Option<FontFilters>,
    host: Vec<Vec<HostFontRow>>,
    current_host: Option<String>,
) -> (Fixture, Rc<RefCell<Seen>>) {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let shown = Rc::clone(&seen);
    let (mut settings, mut filters) = (settings, filters);
    let mut fx = Fixture::new(move |ui| {
        let mut seen = shown.borrow_mut();
        let groups: Vec<HostFontGroup<'_>> = host.iter().map(|rows| HostFontGroup::new(None, rows)).collect();
        let mut picker = FontPicker::new("Font family", Bind::bound(&mut seen.family), &catalog)
            .settings(Bind::bound(&mut settings))
            .served(&served)
            .host(&groups);
        if let Some(filters) = filters.as_mut() {
            picker = picker.filters(Bind::bound(filters));
        }
        if let Some(key) = &current_host {
            picker = picker.current_host(key.clone());
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
        seen.host_picks.extend(out.host_picked);
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

/// A selection of several families leaves the family field empty, on screen and to assistive technology, as the
/// reference applications do: no family name and no word stands in for it.
#[trial]
fn a_mixed_family_shows_an_empty_field() {
    let catalog = catalog_of(&[INTER_REGULAR]);
    let mut family = craft_behavior::Mixed::<String>::Mixed;
    let mut fx = Fixture::new(move |ui| {
        FontPicker::new("Font family", Bind::bound_mixed(&mut family), &catalog).show(ui);
    });
    fx.in_room(Room::Window);
    fx.settle();
    let field = fx.by_role(craft_ui::runtime::access::Role::COMBO_BOX).into_iter().next().expect("the family field");
    assert!(fx.tree().value_of(field.node).is_none_or(str::is_empty), "the mixed family has no accessible value");
    let words: Vec<String> = fx
        .texts()
        .iter()
        .filter(|painted| field.rect.contains_rect(painted.rect) && painted.text.chars().any(char::is_alphabetic))
        .map(|painted| painted.text.clone())
        .collect();
    assert!(words.is_empty(), "the field paints no text: {words:?}");
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
    let infos = FamilyInfos::of(&catalog);
    let rows = family_rows(&catalog, &infos, "", None, &similar, &[], "en");
    let names: Vec<&str> = rows.iter().map(|row| row.family.as_str()).collect();
    assert_eq!(names.first(), Some(&"Inter"), "the reference family first, though it is not first by name");
    assert!(rows.len() <= SIMILAR_COUNT);

    // A reference family the catalog lacks has no traits: the list stays as unfiltered.
    let unfiltered = family_rows(&catalog, &infos, "", None, &FontFilters::default(), &[], "en");
    similar.similar = Some("Missing".to_owned());
    assert_eq!(family_rows(&catalog, &infos, "", None, &similar, &[], "en"), unfiltered);
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
fn a_host_row_pick_reports_its_key_and_leaves_the_family() {
    let host = || {
        vec![vec![
            HostFontRow::face("document", "Document Sans", None),
            HostFontRow::notice("Subset fonts can't take new text"),
            HostFontRow::action("load", "Load font file"),
        ]]
    };
    let (mut fx, seen) = rig_with_host(catalog_of(&[INTER_REGULAR]), vec![], host(), None);
    open(&mut fx);
    let load = fx.by_label("Load font file").expect("the action row");
    fx.click(&load);
    fx.settle();
    assert_eq!(seen.borrow().host_picks, ["load"], "the action row reports its key");
    assert_eq!(seen.borrow().family, "", "a host pick writes no family");
    assert!(seen.borrow().picks.is_empty(), "a host pick is no family pick and no recent family");
    assert!(fx.by_label("Load font file").is_none(), "the list closed");

    open(&mut fx);
    let face = fx.by_label("Document Sans").expect("the face row");
    fx.click(&face);
    fx.settle();
    assert_eq!(seen.borrow().host_picks, ["load", "document"], "the face row reports its key");
    assert_eq!(seen.borrow().family, "", "a host face writes no family");
    assert!(seen.borrow().picks.is_empty(), "a host face is no family pick");

    // The closed field shows the host face; ArrowDown steps over the notice to the action row.
    let (mut fx, seen) = rig_with_host(catalog_of(&[INTER_REGULAR]), vec![], host(), Some("document".to_owned()));
    fx.key(Key::Tab, Modifiers::NONE);
    fx.key(Key::ArrowDown, Modifiers::NONE);
    fx.settle();
    assert_eq!(seen.borrow().host_picks, ["load"], "the arrow skips the notice");
    assert!(seen.borrow().picks.is_empty(), "and picks no family");
    assert_eq!(seen.borrow().family, "");
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
    family_rows(catalog, &FamilyInfos::of(catalog), "", None, filters, sets, "en").into_iter().map(|row| row.family).collect()
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

    let mut settings = FontPickerSettings::default();
    settings.favorites = vec![all[0].clone(), all[2].clone()];
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

/// Thai coverage comes from the face's `cmap` and Japanese from its `OS/2` code pages and Unicode ranges, which the
/// scan's records do not hold: a Thai face and a CJK face each pass their own script's filter and not the other's.
#[trial]
fn a_thai_and_a_cjk_face_each_pass_their_own_scripts_filter() {
    const THAI: &[u8] = include_bytes!("../../../font-subset/tests/data/sarabun/Sarabun-Regular.ttf");
    const CJK: &[u8] = include_bytes!("../../../ui-contract/tests/fonts/NotoSansCJKsc-test.otf");
    let catalog = catalog_of(&[INTER_REGULAR, THAI, CJK]);
    let with = |script: FontScript| {
        let mut filters = FontFilters::default();
        filters.script = Some(script);
        listed(&catalog, &filters, &[])
    };
    let (thai, japanese) = (with(FontScript::Thai), with(FontScript::Japanese));
    assert!(thai.iter().any(|family| family.contains("Sarabun")) && !thai.iter().any(|family| family.contains("CJK")), "Thai: {thai:?}");
    assert!(
        japanese.iter().any(|family| family.contains("CJK")) && !japanese.iter().any(|family| family.contains("Sarabun")),
        "Japanese: {japanese:?}"
    );
    assert!(!thai.iter().chain(&japanese).any(|family| family.contains("Inter")), "Inter covers neither");
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
        // Evicted memory drops in the next component's run, so the window keeps showing one.
        craft_ui::display::Text::new("Size").show(ui);
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
    // Past the eviction grace, which ends the picker's shown life.
    fx.advance(1.0);
    fx.idle(3);
    let ended = take_output(fx.ctx()).ended_previews;
    assert_eq!(ended.iter().filter(|ended| Some(**ended) == id.get()).count(), 1, "the picker that stopped showing is named once: {ended:?}");
}

#[trial]
fn samples_on_white_paint_dark_ink_on_a_lighter_paper_in_a_dark_theme() {
    let white_papers = |on_white: bool| {
        let mut settings = FontPickerSettings::default();
        settings.samples_on_white = on_white;
        let (mut fx, _) = rig_with(three_faces(), vec![], settings, None);
        fx.theme(Mode::Dark, Contrast::Standard);
        fx.settle();
        open(&mut fx);
        fx.frame_shapes().into_iter().filter(|shape| matches!(shape, Shape::Rect(rect) if rect.fill == Color32::WHITE)).count()
    };
    assert!(white_papers(true) >= 3, "each family's sample sits on a white paper, in a dark theme");
    assert_eq!(white_papers(false), 0, "without the setting no sample paper is white");
}

/// A picker as the character controls host it: a given value, the filters (so the class column shows beside the
/// families) and a preview text. The field's list keys go to the family list: Enter on a typed part of a name picks the
/// best match, and Down moves the family row, which previews, before Enter picks it. While no family row shows, Enter is
/// the field's: it commits a typed family name and leaves the class column as it was.
#[trial]
fn with_the_class_column_shown_the_field_keys_move_and_pick_in_the_family_list() {
    #[derive(Default)]
    struct Reported {
        picks: Vec<String>,
        previews: Vec<String>,
    }
    let reported = Rc::new(RefCell::new(Reported::default()));
    let seen = Rc::clone(&reported);
    let catalog = catalog_of(&[INTER_REGULAR, epaint_default_fonts::HACK_REGULAR, epaint_default_fonts::UBUNTU_LIGHT]);
    let mut settings = FontPickerSettings::default();
    let filters = Rc::new(RefCell::new(FontFilters::default()));
    let held = Rc::clone(&filters);
    let mut fx = Fixture::new(move |ui| {
        let mut filters = held.borrow_mut();
        let out = FontPicker::new("Font family", Bind::given("Ubuntu".to_owned()), &catalog)
            .settings(Bind::bound(&mut settings))
            .filters(Bind::bound(&mut *filters))
            .preview_text("Sample")
            .show(ui);
        let mut seen = seen.borrow_mut();
        if let Preview::Shows(family) = out.preview {
            seen.previews.push(family);
        }
        seen.picks.extend(out.picked);
    });
    fx.in_room(Room::Window);
    fx.settle();
    let words = FontPickerWords::default();
    let type_in = |fx: &mut Fixture, text: &str| {
        let field = fx.by_label("Font family").expect("the family field");
        fx.click(&field);
        fx.settle();
        fx.type_text(text);
        fx.settle();
    };

    type_in(&mut fx, "Ha");
    assert!(present(&fx, words.all_classes.as_str()), "the class column shows");
    fx.key(Key::Enter, Modifiers::NONE);
    fx.settle();
    assert_eq!(reported.borrow().picks, ["Hack"], "Enter on a partial name picks the best match");

    // "n" matches Inter and Ubuntu.
    type_in(&mut fx, "n");
    reported.borrow_mut().previews.clear();
    fx.key(Key::ArrowDown, Modifiers::NONE);
    fx.settle();
    let keyed = reported.borrow().previews.last().cloned().expect("Down moves the family row, which previews its family");
    assert!(["Inter", "Ubuntu"].contains(&keyed.as_str()), "the row is a family that matches: {keyed}");
    fx.key(Key::Enter, Modifiers::NONE);
    fx.settle();
    assert_eq!(reported.borrow().picks.last(), Some(&keyed), "Enter picks the family row the keys reached");

    // The host's filters list monospaced families only, Hack alone, so the exact name "Inter" matches no row.
    filters.borrow_mut().class = Some(FontClass::Monospace);
    type_in(&mut fx, "Inter");
    assert!(present(&fx, words.all_classes.as_str()), "the class column shows");
    assert!(fx.by_label("Favorite Inter").is_none() && fx.by_label("Favorite Hack").is_none(), "no family row shows");
    fx.key(Key::Enter, Modifiers::NONE);
    fx.settle();
    assert_eq!(reported.borrow().picks.last().map(String::as_str), Some("Inter"), "the field commits the typed name");
    assert_eq!(filters.borrow().class, Some(FontClass::Monospace), "the class column is unchanged");
}

/// A `Plain` picker over `family` with `recent` families, `actions` rows and the face `shown_as`.
fn plain_rig(
    catalog: Arc<Catalog>,
    family: &str,
    recent: &[&str],
    actions: Vec<Label>,
    shown_as: Option<&'static str>,
) -> (Fixture, Rc<RefCell<Seen>>) {
    let seen = Rc::new(RefCell::new(Seen { family: family.to_owned(), ..Seen::default() }));
    let shown = Rc::clone(&seen);
    let mut settings = FontPickerSettings::default();
    settings.recent = recent.iter().map(|family| (*family).to_owned()).collect();
    let mut fx = Fixture::new(move |ui| {
        let mut seen = shown.borrow_mut();
        let mut picker = FontPicker::new("Font family", Bind::bound(&mut seen.family), &catalog)
            .settings(Bind::bound(&mut settings))
            .kind(FontPickerKind::Plain)
            .actions(&actions)
            .advice(if actions.is_empty() { MissingAdvice::None } else { MissingAdvice::AddFont });
        if let Some(face) = shown_as {
            picker = picker.shown_as(face);
        }
        let out = picker.show(ui);
        seen.id = Some(out.edit.response.id);
        if let Some(family) = out.picked {
            seen.picks.push((family, out.picked_style));
        }
        seen.actions.extend(out.action);
    });
    fx.in_room(Room::Window);
    fx.settle();
    (fx, seen)
}

#[trial]
fn a_plain_box_commits_an_unknown_typed_name_lists_recents_first_and_again_after_a_cleared_query() {
    let catalog = catalog_of(&[INTER_REGULAR, epaint_default_fonts::HACK_REGULAR]);
    let (mut fx, seen) = plain_rig(catalog, "Inter", &["Hack"], vec![], None);
    let words = FontPickerWords::default();
    let (recently, all) = (words.recently_used.as_str(), words.all_fonts.as_str());
    open(&mut fx);
    assert!(present(&fx, recently), "the recents have their heading");
    assert!(present(&fx, all), "the families have theirs");
    fx.key(Key::Escape, Modifiers::NONE);
    fx.settle();

    let field = fx.by_label("Font family").expect("the family field");
    // Unfocus after Escape so the click below is a focusing click that selects all text in Plain mode.
    fx.click_at(field.rect.right_center() + egui::vec2(20.0, 0.0));
    fx.settle();
    fx.click(&field);
    fx.settle();
    fx.type_text("x");
    fx.settle();
    assert!(!present(&fx, recently), "a query lists its matches only");
    fx.key(Key::Backspace, Modifiers::NONE);
    fx.settle();
    assert!(present(&fx, recently), "a cleared query lists the recents again");

    fx.type_text("Zapfino");
    fx.settle();
    fx.key(Key::Enter, Modifiers::NONE);
    fx.settle();
    assert_eq!(seen.borrow().family, "Zapfino", "a name that is no family is applied as typed");
    assert_eq!(seen.borrow().picks.last().map(|(picked, _)| picked.as_str()), Some("Zapfino"));
}

#[trial]
fn clicking_the_font_field_selects_all_without_opening_dropdown_and_chevron_toggles() {
    let catalog = catalog_of(&[INTER_REGULAR, epaint_default_fonts::HACK_REGULAR]);
    let (mut fx, _seen) = plain_rig(catalog, "Inter", &["Hack"], vec![], None);
    let words = FontPickerWords::default();
    let (recently, all) = (words.recently_used.as_str(), words.all_fonts.as_str());

    // 1. Clicking the text field focuses and selects all text without opening the dropdown popup
    let field = fx.by_label("Font family").expect("the family field");
    fx.click(&field);
    fx.settle();
    assert!(!present(&fx, recently), "recently used fonts do not open on field click");
    assert!(!present(&fx, all), "all fonts do not open on field click");

    // 2. Typing in the field opens the dropdown filtered on matches
    fx.type_text("Ha");
    fx.settle();
    assert!(present(&fx, "Hack"), "typing in the field opens the dropdown with matches");

    // Pressing Enter commits the typed family and closes the dropdown
    fx.key(Key::Enter, Modifiers::NONE);
    fx.settle();
    fx.frame(vec![]);
    fx.settle();
    assert!(!present(&fx, all), "committing typed query closes the dropdown");

    // 3. Clicking the dropdown arrow (chevron) toggles the dropdown list open
    let chevron = fx.by_label(words.list.as_str()).expect("the chevron button");
    fx.click(&chevron);
    fx.settle();
    assert!(present(&fx, recently), "recently used fonts show up on chevron click");
    assert!(present(&fx, all), "all fonts show up on chevron click");

    // 4. Clicking inside the field while already open keeps dropdown open
    let field = fx.by_label("Font family").expect("the family field");
    fx.click(&field);
    fx.settle();
    assert!(present(&fx, all), "dropdown remains open on click inside field");

    // 5. Clicking the chevron button again toggles the dropdown closed
    let chevron = fx.by_label(words.list.as_str()).expect("the chevron button");
    fx.click(&chevron);
    fx.settle();
    fx.frame(vec![]);
    fx.settle();
    assert!(!present(&fx, all), "dropdown closes on chevron click toggle");
}

#[trial]
fn clicking_the_focused_plain_box_places_caret_instead_of_selecting_all() {
    let catalog = catalog_of(&[INTER_REGULAR, epaint_default_fonts::HACK_REGULAR]);
    let (mut fx, _seen) = plain_rig(catalog, "Inter", &["Hack"], vec![], None);
    let field = fx.by_label("Font family").expect("the family field");

    // 1. The first click gives the field focus and selects all text
    fx.click(&field);
    fx.settle();

    // 2. Clicking again after double-click interval places the caret before the "I" without re-selecting all
    fx.advance(1.0);
    fx.click_at(field.rect.left_center() + egui::vec2(6.0, 0.0));
    fx.settle();

    // 3. Typing "Ha" inserts before "Inter" producing "HaInter" rather than replacing the text with "Ha"
    fx.type_text("Ha");
    fx.settle();
    assert!(present(&fx, "HaInter"), "typing inserts at placed caret before the existing family name");
    assert!(!present(&fx, "Hack"), "Hack is not suggested because draft is HaInter, not Ha");
}

#[trial]
fn an_action_row_is_reported_not_committed_and_a_missing_family_shows_the_note_naming_the_face() {
    let words = MissingFontsWords::default();
    let note = format!("{} {}", words.shown_as.fill(&[("family", "Gotham"), ("face", "Inter")]), words.add_font.as_str());
    let note = note.as_str();
    let (mut fx, seen) = plain_rig(catalog_of(&[INTER_REGULAR]), "Gotham", &[], vec![Label::from("Add Font…")], Some("Inter"));
    // The note is the tooltip of the field's warning mark, so it shows once the pointer rests on the mark.
    let picker = seen.borrow().id.expect("the picker");
    let mark = fx.part_rect(picker, PartKey::new("missing")).expect("the field carries a mark while the family is not installed");
    assert!(!present(&fx, note), "the note waits for a rest on the mark");
    fx.frame(vec![moved(mark.center())]);
    fx.advance(DELAY);
    fx.settle();
    assert!(present(&fx, note), "a rest on the mark shows the note");
    open(&mut fx);
    let action = fx.by_label("Add Font…").expect("the action row");
    fx.click(&action);
    fx.settle();
    assert_eq!(seen.borrow().actions, [0], "the action is reported by its index");
    assert_eq!(seen.borrow().family, "Gotham", "an action leaves the family");
    assert!(seen.borrow().picks.is_empty(), "and picks nothing");

    let (fx, seen) = plain_rig(catalog_of(&[INTER_REGULAR]), "Inter", &[], vec![Label::from("Add Font…")], Some("Inter"));
    let picker = seen.borrow().id.expect("the picker");
    assert!(fx.part_rect(picker, PartKey::new("missing")).is_none(), "an installed family has no mark");
}

/// The width of a plain picker over `family`, in a ribbon group's place or not.
fn picker_width(family: &'static str, ribbon: bool) -> f32 {
    let catalog = catalog_of(&[INTER_REGULAR]);
    let mut fx = Fixture::new(move |ui| {
        let mut family = family.to_owned();
        let mut show = |ui: &mut egui::Ui| {
            let _ = FontPicker::new("Font family", Bind::bound(&mut family), &catalog).kind(FontPickerKind::Plain).show(ui);
        };
        if ribbon {
            with_env(
                ui,
                |overrides| {
                    overrides.values.set(ControlPlace::Ribbon);
                },
                show,
            );
        } else {
            show(ui);
        }
    });
    fx.in_room(Room::Window);
    fx.settle();
    fx.by_label("Font family").expect("the picker").rect.width()
}

/// In a ribbon the font box is held at the small field length, narrower than in a form, and keeps its width whichever
/// family it shows.
#[trial]
fn a_ribbons_font_box_is_short_and_keeps_its_width() {
    let (ribbon, standard) = (picker_width("Inter", true), picker_width("Inter", false));
    assert!(ribbon < standard - 0.5, "the ribbon's font box is narrower: {ribbon} against {standard}");
    let length = craft_ui::testing::style::tokens(craft_theme::Mode::Light, craft_theme::Contrast::Standard).size.length.field.sm;
    assert!((ribbon - length).abs() < 0.5, "the ribbon's font box is the small field length {length}: {ribbon}");
    let long = picker_width("Source Sans Pro Semibold Condensed Extra", true);
    assert!((long - ribbon).abs() < 0.5, "a long family name leaves its width: {long} against {ribbon}");
}

/// In a toolbar (`InBar`) or when `compact` is set, the font box is held at the small field length,
/// narrower than in a form, and keeps its width whichever family it shows.
#[trial]
fn a_bars_font_box_is_short_and_keeps_its_width() {
    let bar_width = |family: &'static str, in_bar: bool, compact: bool| {
        let catalog = catalog_of(&[INTER_REGULAR]);
        let mut fx = Fixture::new(move |ui| {
            let mut family = family.to_owned();
            let mut show = |ui: &mut egui::Ui| {
                let mut picker = FontPicker::new("Font family", Bind::bound(&mut family), &catalog).kind(FontPickerKind::Plain);
                if compact {
                    picker = picker.compact(true);
                }
                let _ = picker.show(ui);
            };
            if in_bar {
                with_env(
                    ui,
                    |overrides| {
                        overrides.values.set(craft_ui::contract::InBar(true));
                    },
                    show,
                );
            } else {
                show(ui);
            }
        });
        fx.in_room(Room::Window);
        fx.settle();
        fx.by_label("Font family").expect("the picker").rect.width()
    };
    let standard = bar_width("Inter", false, false);
    let bar = bar_width("Inter", true, false);
    let opt_in = bar_width("Inter", false, true);
    assert!(bar < standard - 0.5, "the bar's font box is narrower: {bar} against {standard}");
    assert!((bar - opt_in).abs() < 0.5, "the opt-in compact picker matches the bar picker: {opt_in} against {bar}");
    let length = craft_ui::testing::style::tokens(craft_theme::Mode::Light, craft_theme::Contrast::Standard).size.length.field.sm;
    assert!((bar - length).abs() < 0.5, "the bar's font box is the small field length {length}: {bar}");
    let long = bar_width("Source Sans Pro Semibold Condensed Extra", true, false);
    assert!((long - bar).abs() < 0.5, "a long family name leaves its width in a bar: {long} against {bar}");
}

/// The width of a plain picker over "Inter" in a ribbon group's place, in a row beside a block that leaves it 60 pt of
/// the window, so the row shrinks it to its least.
fn squeezed_ribbon_picker_width() -> f32 {
    use craft_ui::contract::ShowContainer as _;
    let catalog = catalog_of(&[INTER_REGULAR]);
    let mut fx = Fixture::new(move |ui| {
        let mut family = "Inter".to_owned();
        with_env(
            ui,
            |overrides| {
                overrides.values.set(ControlPlace::Ribbon);
            },
            |ui| {
                let rest = ui.available_width() - 60.0;
                craft_ui::layout_components::Row::new().show(ui, |ui, _| {
                    let _ = FontPicker::new("Font family", Bind::bound(&mut family), &catalog).kind(FontPickerKind::Plain).show(ui);
                    craft_ui::testing::layout::Block::new(egui::Vec2::new(rest, 20.0)).show(ui);
                });
            },
        );
    });
    fx.in_room(Room::Window);
    fx.settle();
    fx.by_label("Font family").expect("the picker").rect.width()
}

/// A ribbon row that lacks room narrows its font box no further than its least: a name field the short field length
/// (`size.length.short`) wide, about the old ribbon's font name, plus its menu arrow; never to an arrow alone.
#[trial]
fn a_squeezed_ribbon_font_box_keeps_a_short_fields_name() {
    let squeezed = squeezed_ribbon_picker_width();
    let short = craft_ui::testing::style::tokens(craft_theme::Mode::Light, craft_theme::Contrast::Standard).size.length.short;
    assert!(squeezed > short, "the squeezed font box keeps a name field {short} wide and its arrow: {squeezed}");
    assert!(squeezed <= picker_width("Inter", true) + 0.5, "and is no wider than where the room allows");
}

/// The family data builds of the last frame.
fn family_builds(fx: &Fixture) -> u32 {
    last_frame_derived_builds(fx.ctx()).iter().map(|(_, builds)| builds).sum()
}

#[trial]
fn the_family_data_builds_when_the_catalog_changes_and_not_while_it_is_idle() {
    let catalog = catalog_of(&[INTER_REGULAR]);
    let (mut fx, _) = rig(Arc::clone(&catalog), vec![]);
    open(&mut fx);
    fx.frame(vec![]);
    assert_eq!(family_builds(&fx), 0, "an idle open list builds nothing");
    catalog.add_bytes(INTER_SEMIBOLD.to_vec(), Source::Bundled, &Limits::default()).expect("a font file");
    fx.frame(vec![]);
    assert_eq!(family_builds(&fx), 1, "a changed catalog builds the data once, whatever reads it");
}

#[trial]
fn a_picker_handed_another_catalog_at_the_same_generation_builds_its_family_data_once() {
    let (one, two) = (catalog_of(&[INTER_REGULAR]), catalog_of(&[INTER_REGULAR]));
    assert_eq!(one.generation(), two.generation(), "the two catalogs count alike");
    assert_ne!(one.id(), two.id());
    let shown = Rc::new(RefCell::new(Arc::clone(&one)));
    let held = Rc::clone(&shown);
    let mut family = String::new();
    let mut fx = Fixture::new(move |ui| {
        let catalog = Arc::clone(&held.borrow());
        FontPicker::new("Font family", Bind::bound(&mut family), &catalog).show(ui);
    });
    fx.in_room(Room::Window);
    fx.settle();
    open(&mut fx);
    fx.frame(vec![]);
    assert_eq!(family_builds(&fx), 0, "the first catalog's data is kept");
    *shown.borrow_mut() = Arc::clone(&two);
    fx.frame(vec![]);
    assert_eq!(family_builds(&fx), 1, "the second catalog builds the data once");
}

/// Every non-transparent vertex colour of textured meshes whose centre falls inside `rect`.
fn mesh_tints(shapes: &[egui::Shape], rect: egui::Rect, out: &mut Vec<egui::Color32>) {
    for shape in shapes {
        match shape {
            egui::Shape::Vec(inner) => mesh_tints(inner, rect, out),
            egui::Shape::Mesh(mesh) if rect.contains(mesh.calc_bounds().center()) => {
                out.extend(mesh.vertices.iter().map(|vertex| vertex.color).filter(|color| color.a() > 0));
            }
            _ => {}
        }
    }
}

/// The painted tint colours of the missing-font warning icon in `mode`/`contrast`, enabled or disabled, together with
/// the theme's `status.warn.fg`, `text.normal` and `text.disabled` colours.
fn missing_warning_colors(
    mode: craft_theme::Mode,
    contrast: craft_theme::Contrast,
    enabled: bool,
) -> (Vec<egui::Color32>, egui::Color32, egui::Color32, egui::Color32) {
    use craft_ui::contract::Props as _;
    use craft_ui::runtime::env::Env;
    use egui::Color32;
    use std::cell::Cell;

    let catalog = catalog_of(&[INTER_REGULAR]);
    let seen = Rc::new(Cell::new((None, Color32::TRANSPARENT, Color32::TRANSPARENT, Color32::TRANSPARENT)));
    let held = Rc::clone(&seen);
    let mut family = "Gotham".to_owned();
    let mut fx = Fixture::new(move |ui| {
        let color = |[r, g, b, a]: [u8; 4]| Color32::from_rgba_unmultiplied(r, g, b, a);
        let env = Env::of(ui);
        let tokens = &env.tokens.tokens().color;
        let out = FontPicker::new("Font family", Bind::bound(&mut family), &catalog)
            .kind(FontPickerKind::Plain)
            .shown_as("Inter")
            .enabled(enabled)
            .show(ui);
        held.set((Some(out.edit.response.id), color(tokens.status.warn.fg.0), color(tokens.text.normal.0), color(tokens.text.disabled.0)));
    });
    fx.theme(mode, contrast);
    fx.in_room(Room::Window);
    fx.settle();
    let (id, warn_fg, text_normal, text_disabled) = seen.get();
    let picker = id.expect("the picker");
    let mark = fx.part_rect(picker, PartKey::new("missing")).expect("the field carries a mark while the family is not installed");
    let mut tints = Vec::new();
    mesh_tints(&fx.frame_shapes(), mark, &mut tints);
    tints.dedup();
    (tints, warn_fg, text_normal, text_disabled)
}

#[trial]
fn the_missing_font_warning_icon_paints_in_status_warn_fg_and_dims_when_disabled() {
    use craft_theme::{Contrast, Mode};

    for (mode, contrast) in [(Mode::Light, Contrast::Standard), (Mode::Dark, Contrast::Standard), (Mode::Dark, Contrast::High)] {
        let (tints, warn_fg, text_normal, _) = missing_warning_colors(mode, contrast, true);
        assert_ne!(warn_fg, text_normal, "{mode:?} {contrast:?}: warning ink differs from neutral text ink");
        assert!(!tints.is_empty(), "{mode:?} {contrast:?}: the missing-font warning icon paints");
        assert!(
            tints.iter().all(|tint| *tint == warn_fg),
            "{mode:?} {contrast:?}: the warning icon paints in status.warn.fg {warn_fg:?}, not {tints:?}"
        );
    }

    let (tints, warn_fg, _, text_disabled) = missing_warning_colors(Mode::Dark, Contrast::Standard, false);
    assert_ne!(text_disabled, warn_fg, "disabled ink differs from warning ink");
    assert!(!tints.is_empty(), "a disabled picker still paints its warning icon");
    assert!(
        tints.iter().all(|tint| *tint == text_disabled),
        "a disabled picker paints its warning icon in text.disabled {text_disabled:?}, not {tints:?}"
    );
}

/// Texture ids of textured meshes whose centre falls inside `rect`.
fn mesh_textures(shapes: &[egui::Shape], rect: egui::Rect, out: &mut Vec<egui::TextureId>) {
    for shape in shapes {
        match shape {
            egui::Shape::Vec(inner) => mesh_textures(inner, rect, out),
            egui::Shape::Mesh(mesh) if rect.contains(mesh.calc_bounds().center()) => {
                out.push(mesh.texture_id);
            }
            _ => {}
        }
    }
}

#[trial]
fn the_missing_font_warning_icon_is_a_filled_triangle() {
    use std::cell::Cell;

    let catalog = catalog_of(&[INTER_REGULAR]);
    let seen = Rc::new(Cell::new(None));
    let held = Rc::clone(&seen);
    let mut family = "Gotham".to_owned();
    let mut fx = Fixture::new(move |ui| {
        let out = FontPicker::new("Font family", Bind::bound(&mut family), &catalog).kind(FontPickerKind::Plain).shown_as("Inter").show(ui);
        held.set(Some(out.edit.response.id));
    });
    fx.pixels_per_point(2.0);
    fx.record_paint();
    fx.in_room(Room::Window);
    fx.settle();
    let picker = seen.get().expect("the picker");
    let mark = fx.part_rect(picker, PartKey::new("missing")).expect("the field carries a mark while the family is not installed");
    let mut paint = fx.take_paint().expect("recorded paint output");
    let mut textures = Vec::new();
    for clipped in &paint.shapes {
        mesh_textures(std::slice::from_ref(&clipped.shape), mark, &mut textures);
    }
    let texture = *textures.first().expect("the missing-font warning icon paints a texture");
    let image = paint
        .textures_delta
        .set
        .get(&texture)
        .and_then(|deltas| deltas.last())
        .map(|delta| match &delta.image {
            egui::ImageData::Color(color) => color,
        })
        .expect("the warning icon texture was uploaded");
    let [w, h] = image.size;
    let alpha = |fx: f32, fy: f32| {
        let x = ((w as f32 * fx) as usize).min(w - 1);
        let y = ((h as f32 * fy) as usize).min(h - 1);
        image.pixels[y * w + x].a()
    };
    let (left, right, center) = (alpha(0.34, 0.69), alpha(0.66, 0.69), alpha(0.50, 0.48));
    paint.textures_delta.clear();
    assert!(left > 128 && right > 128, "the triangle interior is filled, not an outline (left={left}, right={right}, size={w}x{h})");
    assert!(center < 128, "the exclamation stem stays knocked out inside the filled triangle (center={center})");
}

/// The library picker's family and settings, as the last pass left them.
struct Library {
    family: String,
    settings: FontPickerSettings,
}

/// A library list with Hack as a recent family: the shown picker and the catalog of Inter (two styles) and Hack (one).
fn library_rig() -> (Fixture, Rc<RefCell<Library>>) {
    let catalog = catalog_of(&[INTER_REGULAR, INTER_SEMIBOLD, epaint_default_fonts::HACK_REGULAR]);
    let mut settings = FontPickerSettings::default();
    settings.recent = vec!["Hack".to_owned()];
    let state = Rc::new(RefCell::new(Library { family: "Inter".to_owned(), settings }));
    let shown = Rc::clone(&state);
    let mut filters = FontFilters::default();
    let mut fx = Fixture::new(move |ui| {
        let mut state = shown.borrow_mut();
        let Library { family, settings } = &mut *state;
        FontPicker::new("Font family", Bind::bound(family), &catalog).settings(Bind::bound(settings)).filters(Bind::bound(&mut filters)).show(ui);
    });
    fx.in_room(Room::Window);
    fx.settle();
    open(&mut fx);
    (fx, state)
}

/// "أبجد هوز", which neither Inter nor Hack draws.
const ARABIC_TEXT: &str = "\u{623}\u{628}\u{62C}\u{62F} \u{647}\u{648}\u{632}";

/// An open library list over Inter and Hack, with Gotham served, `family` current and `text` as the preview text.
fn sample_rig(family: &'static str, text: &'static str) -> Fixture {
    let catalog = catalog_of(&[INTER_REGULAR, epaint_default_fonts::HACK_REGULAR]);
    let served = vec!["Gotham".to_owned()];
    let mut family = family.to_owned();
    let mut settings = FontPickerSettings::default();
    let mut fx = Fixture::new(move |ui| {
        FontPicker::new("Font family", Bind::bound(&mut family), &catalog)
            .settings(Bind::bound(&mut settings))
            .served(&served)
            .preview_text(text)
            .show(ui);
    });
    fx.in_room(Room::Window);
    fx.settle();
    open(&mut fx);
    fx
}

/// Each row has one sample, the picker's sample text, in its own face; a face that lacks the text's letters shows its
/// name instead, in its row and in the banner; and a family without a face shows no sample at all, not the text in the interface face.
#[trial]
fn each_row_samples_the_preview_text_and_a_face_without_it_shows_its_name() {
    let fx = sample_rig("Inter", "Hello");
    for family in ["Inter", "Hack"] {
        assert!(fx.by_label(&format!("{family}: Hello")).is_some(), "{family}'s sample is the preview text");
        assert!(fx.by_label(&format!("{family}: Sample")).is_none(), "{family} has no sample word");
    }

    let fx = sample_rig("Gotham", ARABIC_TEXT);
    assert!(fx.by_label("Font preview: Gotham").is_some(), "the banner shows the served family");
    for family in ["Inter", "Hack"] {
        assert!(fx.by_label(&format!("{family}: {family}")).is_some(), "{family} lacks Arabic: its sample is its name");
    }
    assert!(
        fx.texts().iter().all(|painted| painted.text != ARABIC_TEXT),
        "no face draws the text: neither the rows nor the banner of a family without a face paint it"
    );

    let fx = sample_rig("Inter", ARABIC_TEXT);
    assert!(fx.by_label("Font preview: Inter").is_some(), "the banner shows Inter");
    assert!(
        fx.texts().iter().all(|painted| painted.text != ARABIC_TEXT),
        "the banner of a face without the text's letters shows what its row shows, not the text"
    );
}

/// The sample texts the host offers in the chips rigs: the selection's text first, then a fixed word.
const SAMPLE_TEXTS: [(&str, &str, &str); 2] = [("selection", "Selected Text", "Hello"), ("word", "Word", "Abc")];

/// An open library list in the `Chips` layout over Inter, keeping `settings`, with the host's sample texts when
/// `offered` and "Hello" as the preview text.
fn sample_texts_rig(settings: Rc<RefCell<FontPickerSettings>>, offered: bool) -> Fixture {
    let catalog = catalog_of(&[INTER_REGULAR]);
    let mut family = "Inter".to_owned();
    let mut filters = FontFilters::default();
    let mut fx = Fixture::new(move |ui| {
        let texts: Vec<SampleText<'_>> =
            if offered { SAMPLE_TEXTS.iter().map(|(key, label, text)| SampleText::new(key, *label, text)).collect() } else { Vec::new() };
        let mut settings = settings.borrow_mut();
        FontPicker::new("Font family", Bind::bound(&mut family), &catalog)
            .settings(Bind::bound(&mut settings))
            .filters(Bind::bound(&mut filters))
            .filter_layout(FilterLayout::Chips)
            .preview_text("Hello")
            .sample_texts(&texts)
            .show(ui);
    });
    fx.in_room(Room::Window);
    fx.settle();
    open(&mut fx);
    fx
}

/// The host's sample texts make a Sample Text dropdown beside the preview size: the rows and the banner show the
/// chosen text (the first while none is chosen), a choice from the dropdown is kept in the settings and every row
/// follows it, and a stored choice the host no longer offers falls back to the first. Without sample texts there is
/// no dropdown and the preview text shows.
#[trial]
fn the_sample_text_dropdown_picks_what_every_sample_shows() {
    let settings = Rc::new(RefCell::new(FontPickerSettings::default()));
    let mut fx = sample_texts_rig(Rc::clone(&settings), true);
    assert!(fx.by_label("Inter: Hello").is_some(), "with no choice stored the rows show the first sample text");
    let dropdown = fx.by_label("Sample Text").expect("the sample text dropdown");
    fx.click(&dropdown);
    fx.settle();
    let word = fx.by_label("Word").expect("the dropdown lists the host's sample texts");
    fx.click(&word);
    fx.settle();
    assert_eq!(settings.borrow().sample_text.as_deref(), Some("word"), "the choice is kept in the settings");
    assert!(fx.by_label("Inter: Abc").is_some(), "the row follows the chosen sample text");
    assert!(fx.by_label("Inter: Hello").is_none(), "no row keeps the earlier text");
    assert!(fx.texts().iter().any(|painted| painted.text == "Abc"), "the banner shows the chosen text");

    let stored = |key: &str| {
        let mut settings = FontPickerSettings::default();
        settings.sample_text = Some(key.to_owned());
        Rc::new(RefCell::new(settings))
    };
    let fx = sample_texts_rig(stored("gone"), true);
    assert!(fx.by_label("Inter: Hello").is_some(), "a choice the host no longer offers falls back to the first sample text");

    let fx = sample_texts_rig(stored("word"), false);
    assert!(fx.by_label("Sample Text").is_none(), "without the host's sample texts there is no dropdown");
    assert!(fx.by_label("Inter: Hello").is_some(), "without sample texts the rows show the preview text");
}

/// The repository's Noto Sans CJK SC subset (CFF): its line (hhea 1160/-288) is taller than Inter's.
const CJK_SUBSET: &[u8] = include_bytes!("../../../ui-contract/tests/fonts/NotoSansCJKsc-test.otf");

/// "中文", which the subset draws and Inter lacks.
const CJK_TEXT: &str = "\u{4E2D}\u{6587}";

/// An open library list at Large over Inter and the CJK subset, with `family` current and the preview text `text`
/// holds at each pass.
fn cjk_rig(family: &'static str, text: Rc<Cell<Option<&'static str>>>) -> Fixture {
    let catalog = catalog_of(&[INTER_REGULAR, CJK_SUBSET]);
    let mut family = family.to_owned();
    let mut settings = FontPickerSettings::default();
    settings.preview = FontPreviewSize::Large;
    let mut fx = Fixture::new(move |ui| {
        let mut picker = FontPicker::new("Font family", Bind::bound(&mut family), &catalog).settings(Bind::bound(&mut settings));
        if let Some(text) = text.get() {
            picker = picker.preview_text(text);
        }
        picker.show(ui);
    });
    fx.in_room(Room::Window);
    fx.settle();
    open(&mut fx);
    fx
}

/// The host's text can arrive while the list is open (a selection made after the menu opened): every row follows it,
/// in a CJK face too, and no row keeps the sample word.
#[trial]
fn rows_sample_the_host_text_that_arrives_after_the_list_opened() {
    let text = Rc::new(Cell::new(None));
    let mut fx = cjk_rig("Inter", Rc::clone(&text));
    assert!(fx.by_label("Inter: Sample").is_some(), "without a host text the rows show the sample word");
    text.set(Some(CJK_TEXT));
    fx.settle();
    assert!(fx.by_label(&format!("Noto Sans CJK SC: {CJK_TEXT}")).is_some(), "the CJK face's row shows the host's text");
    assert!(fx.by_label("Inter: Inter").is_some(), "Inter lacks the text's letters: its row shows its name");
    for family in ["Inter", "Noto Sans CJK SC"] {
        assert!(fx.by_label(&format!("{family}: Sample")).is_none(), "{family}'s row no longer shows the sample word");
    }
}

/// The baseline of a painted text, on screen.
fn baseline_of(text: &egui::epaint::TextShape) -> f32 {
    let row = text.galley.rows.first().expect("a row");
    text.pos.y + row.pos.y + row.row.glyphs.first().map_or(row.row.size.y, |glyph| glyph.pos.y)
}

/// Faces whose lines differ (Inter's and the CJK subset's, which is taller) sit on one baseline in their rows, and each
/// sample's ink, the rows' and the banner's, stays inside its band: none is clipped.
#[trial]
fn every_sample_sits_on_one_baseline_inside_its_band() {
    let mut fx = cjk_rig("Noto Sans CJK SC", Rc::new(Cell::new(Some(CJK_TEXT))));
    let shapes = fx.frame_clipped_shapes();
    let painted = |text: &str, band: egui::Rect| {
        shapes
            .iter()
            .filter_map(|(_, shape)| match shape {
                egui::Shape::Text(painted) if painted.galley.job.text == text => Some(painted),
                _ => None,
            })
            .find(|painted| band.contains(painted.visual_bounding_rect().center()))
            .unwrap_or_else(|| panic!("{text} painted in {band:?}"))
    };
    let mut offsets = Vec::new();
    for (family, text) in [("Inter", "Inter"), ("Noto Sans CJK SC", CJK_TEXT)] {
        let band = fx.by_label(&format!("{family}: {text}")).unwrap_or_else(|| panic!("{family}'s sample")).rect;
        let sample = painted(text, band);
        let ink = sample.visual_bounding_rect();
        assert!(band.expand(0.01).contains_rect(ink), "{family}'s ink {ink:?} stays inside its band {band:?}");
        offsets.push(baseline_of(sample) - band.top());
    }
    assert!((offsets[0] - offsets[1]).abs() < 0.5, "both samples sit on one baseline in their bands: {offsets:?}");
    let banner = fx.by_label("Font preview: Noto Sans CJK SC").expect("the banner").rect;
    let ink = painted(CJK_TEXT, banner).visual_bounding_rect();
    assert!(banner.contains_rect(ink), "the banner's ink {ink:?} stays inside the banner {banner:?}");
    let name = fx.texts().iter().find(|text| text.text == "Noto Sans CJK SC" && banner.contains_rect(text.rect)).expect("the banner's name").rect;
    assert!(ink.top() >= name.bottom(), "the banner's sample stays below its name: {ink:?} {name:?}");
}

/// Production's and Photoshop's row: the star first, then the names in one column whether or not a family has styles
/// to open (a recent row included), the sample after the name; and a rule, not blank room, ends the recent group.
#[trial]
fn family_names_line_up_after_the_star_and_before_the_sample() {
    let (fx, _) = library_rig();
    let banner = fx.by_label("Font preview: Inter").expect("the banner names the current family").rect;
    let names: Vec<egui::Rect> = fx
        .texts()
        .iter()
        .filter(|painted| (painted.text == "Inter" || painted.text == "Hack") && painted.rect.top() >= banner.bottom())
        .map(|painted| painted.rect)
        .collect();
    assert_eq!(names.len(), 3, "Hack in the recent group and in the list, Inter once: {names:?}");
    assert!(names.iter().all(|rect| (rect.left() - names[0].left()).abs() < 0.5), "the names start at one x: {names:?}");
    let star = fx.by_label("Favorite Inter").expect("Inter's star").rect;
    let inter = names.iter().find(|rect| (rect.center().y - star.center().y).abs() < star.height()).expect("Inter's name beside its star");
    assert!(star.right() <= inter.left(), "the star comes first: {star:?} {inter:?}");
    let sample =
        fx.texts().iter().all(|painted| painted.text != "Sample" || painted.rect.top() < banner.bottom() || painted.rect.left() >= inter.right());
    assert!(sample, "no sample before a name");
    // The recent group's row is the top one; the rule lies between it and the next name, lower than the room between.
    let recent = names.iter().map(|rect| rect.bottom()).fold(f32::INFINITY, f32::min);
    let next = names.iter().map(|rect| rect.top()).filter(|top| *top > recent).fold(f32::INFINITY, f32::min);
    let rule = fx
        .shape_bounds()
        .iter()
        .any(|shape| shape.top() >= recent && shape.bottom() <= next && shape.height() < (next - recent) && shape.width() > star.width() * 4.0);
    assert!(rule, "a rule under the recent group");
}

/// A click on a row's star stars the family without picking it or closing the list.
#[trial]
fn a_star_flips_the_favourite_without_picking_or_closing() {
    let (mut fx, state) = library_rig();
    let star = fx.by_label("Favorite Hack").expect("Hack's star");
    fx.click(&star);
    fx.settle();
    assert!(fx.by_label("Favorite Inter").is_some(), "the list stays open");
    let state = state.borrow();
    assert_eq!(state.settings.favorites, ["Hack"], "Hack is starred");
    assert_eq!(state.family, "Inter", "the star picks nothing");
}

/// The class column is a list of options: a class narrows the list, All Classes brings every family back.
#[trial]
fn a_class_narrows_the_list_and_all_classes_restores_it() {
    let (mut fx, _) = library_rig();
    let monospace = fx.by_label("Monospace").expect("the Monospace class");
    fx.click(&monospace);
    fx.settle();
    assert!(fx.by_label("Favorite Hack").is_some(), "Hack is monospaced");
    assert!(fx.by_label("Favorite Inter").is_none(), "Inter is not");
    let all = fx.by_label("All Classes").expect("All Classes");
    fx.click(&all);
    fx.settle();
    assert!(fx.by_label("Favorite Inter").is_some(), "every family is back");
}

#[trial]
fn a_click_on_a_rows_sample_picks_its_family() {
    let (mut fx, seen) = rig(catalog_of(&[INTER_REGULAR]), vec![]);
    open(&mut fx);
    let inter = fx.by_label("Inter").expect("the Inter row");
    let sample = fx
        .texts()
        .iter()
        .find(|painted| painted.text == "Sample" && inter.rect.contains(painted.rect.center()))
        .map(|painted| painted.rect.center())
        .expect("Inter's row shows its sample");
    fx.click_at(sample);
    fx.settle();
    assert_eq!(seen.borrow().picks, [("Inter".to_owned(), None)], "the click picks the row");
}

/// The open list loads its rows' faces through the host's jobs, never in a pass, and a list opened again over the
/// same rows loads none of them again.
#[trial]
fn the_list_loads_its_sample_faces_off_the_pass_and_once() {
    let jobs = Arc::new(craft_host::ManualJobs::new());
    let host = craft_host::Host::builder(craft_host::AppInfo { id: "trial", name: "Trial", version: "0" }).jobs(jobs.clone()).build();
    let (mut fx, _) = rig(catalog_of(&[INTER_REGULAR, epaint_default_fonts::HACK_REGULAR]), vec![]);
    fx.host(Arc::new(host));
    open(&mut fx);
    assert!(present(&fx, "Inter"), "the rows show while their faces load");
    assert!(jobs.waiting() > 0, "the rows' faces wait for the host's jobs");
    jobs.run_all();
    fx.settle();
    fx.key(Key::Escape, Modifiers::NONE);
    fx.settle();
    open(&mut fx);
    fx.frame(vec![]);
    assert_eq!(jobs.waiting(), 0, "the list opened again loads no face");
}

#[trial]
fn the_closed_field_skips_building_stops_while_idle_and_steps_on_input() {
    let all = listed(&three_faces(), &FontFilters::default(), &[]);
    assert_eq!(all.len(), 3);

    let (mut fx, seen) = rig(three_faces(), vec![]);
    seen.borrow_mut().family = all[0].clone();
    fx.frame(vec![]);
    let before = craft_font_manager::picker::stop_builds(fx.ctx());
    for _ in 0..5 {
        fx.frame(vec![]);
    }
    assert_eq!(craft_font_manager::picker::stop_builds(fx.ctx()), before, "idle closed frames do not rebuild stops");

    aim_at_field(&mut fx);
    assert_eq!(craft_font_manager::picker::stop_builds(fx.ctx()), before, "focusing and hovering without stepping do not rebuild stops");

    fx.key(Key::ArrowDown, Modifiers::NONE);
    assert_eq!(seen.borrow().family, all[1], "ArrowDown steps to the next family");
    let after_key = craft_font_manager::picker::stop_builds(fx.ctx());
    assert_eq!(after_key, before + 1, "one ArrowDown builds stops once");

    fx.frame(vec![wheel_lines(-1.0)]);
    assert_eq!(seen.borrow().family, all[2], "wheel down steps to the following family");
    let after_wheel = craft_font_manager::picker::stop_builds(fx.ctx());
    assert_eq!(after_wheel, after_key + 1, "one wheel tick builds stops once");

    fx.advance(GESTURE_GAP * 2.0).idle(3);
    assert_eq!(seen.borrow().picks.last().map(|(f, _)| f.as_str()), Some(all[2].as_str()), "wheel end commits the stepped family");
    assert_eq!(craft_font_manager::picker::stop_builds(fx.ctx()), after_wheel, "wheel end and idle frames do not rebuild stops");
}
