//! The font picker's flows: style rows under an expanded family, served families and the rested report, and similar
//! fonts.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft_behavior::{PartKey, Preview};
use craft_font_manager::craft_font_catalog::{Limits, Os, Source};
use craft_font_manager::missing::MissingFontsWords;
use craft_font_manager::picker::{
    FamilyInfos, FontFilters, FontPicker, FontPickerKind, FontPickerSettings, FontPickerWords, MissingAdvice, SIMILAR_COUNT, family_rows,
};
use craft_font_manager::{Catalog, FontEnv};
use craft_ui::buttons::ControlPlace;
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::runtime::env::{Label, with_env};
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{Fixture, Room, last_frame_derived_builds, trial};
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
    actions: Vec<usize>,
    /// The id of the shown picker, as the last pass reported it.
    id: Option<egui::Id>,
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
    let infos = FamilyInfos::of(&catalog);
    let rows = family_rows(&catalog, &infos, "", None, &similar, "Inter", "en");
    let names: Vec<&str> = rows.iter().map(|row| row.family.as_str()).collect();
    assert_eq!(names.first(), Some(&"Inter"), "the current family first, though it is not first by name");
    assert!(rows.len() <= SIMILAR_COUNT);

    // A current family the catalog lacks has no traits: the list stays as unfiltered.
    let unfiltered = family_rows(&catalog, &infos, "", None, &FontFilters::default(), "", "en");
    assert_eq!(family_rows(&catalog, &infos, "", None, &similar, "Missing", "en"), unfiltered);
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
