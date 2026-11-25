//! Stories of `FontPicker`: the closed field, the list open with recents and a typed query, the list open with the
//! class column and a filter on, and the list open with host rows above the families. They draw from the two faces
//! egui ships in an in-memory catalog, with Inter where a story needs a family of two styles, Noto Sans Arabic, from
//! the craft fonts set when the build has `CRAFT_FONTS_DIR`, for the Arabic sample, and the repository's Noto Sans CJK
//! SC subset for the Chinese one.

use std::sync::{Arc, OnceLock};

use craft_behavior::Mixed;
use craft_ui::contract::{Bind, Show as _, Validity};
use craft_ui::fonts::{EmbeddedFace, FontRef};
use craft_ui::runtime::env::{Label, Message};
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::{Key, Modifiers, Ui};

use super::{
    FamilySet, FilterLayout, FontFilters, FontPicker, FontPickerKind, FontPickerSettings, FontScript, HostFontGroup, HostFontRow, MissingAdvice,
};
use crate::Catalog;
use crate::craft_font_catalog::{Limits, Source};
use crate::panel::fixtures::story_catalog;

/// The catalog every story shares, so its families are read once.
fn catalog() -> Arc<Catalog> {
    static CATALOG: OnceLock<Arc<Catalog>> = OnceLock::new();
    Arc::clone(CATALOG.get_or_init(story_catalog))
}

const LONG_FAMILY: &str = "Source Sans Pro Semibold Condensed Display Extra Wide Italic Old Style Alternate Numerals";

/// What a story keeps between passes.
#[derive(Clone, Debug, PartialEq)]
struct Demo {
    family: String,
    settings: FontPickerSettings,
    filters: FontFilters,
}

fn demo(family: &str, settings: FontPickerSettings, filters: FontFilters) -> Demo {
    Demo { family: family.to_owned(), settings, filters }
}

/// The craft fonts set's Noto Sans Arabic; empty when the build has no `CRAFT_FONTS_DIR`.
static STORY_FONTS: &[EmbeddedFace] = include!(concat!(env!("OUT_DIR"), "/story_fonts.rs"));

/// A new story catalog with Inter Regular and SemiBold, a family of two styles, and the faces `more`.
fn with_inter(more: &[&'static [u8]]) -> Arc<Catalog> {
    let catalog = story_catalog();
    let inter = [
        include_bytes!("../../../ui-contract/fonts/inter/Inter-Regular.ttf").as_slice(),
        include_bytes!("../../../ui-contract/fonts/inter/Inter-SemiBold.ttf").as_slice(),
    ];
    for bytes in inter.iter().chain(more) {
        let _ = catalog.add_bytes(bytes.to_vec(), Source::Bundled, &Limits::default());
    }
    catalog
}

/// The story catalog with Inter Regular and SemiBold.
fn styled_catalog() -> Arc<Catalog> {
    static CATALOG: OnceLock<Arc<Catalog>> = OnceLock::new();
    Arc::clone(CATALOG.get_or_init(|| with_inter(&[])))
}

/// The styled catalog with Noto Sans Arabic, when the build has the craft fonts set.
fn arabic_catalog() -> Arc<Catalog> {
    static CATALOG: OnceLock<Arc<Catalog>> = OnceLock::new();
    Arc::clone(CATALOG.get_or_init(|| with_inter(&STORY_FONTS.iter().map(|face| face.bytes).collect::<Vec<_>>())))
}

/// The styled catalog with the repository's Noto Sans CJK SC subset.
fn cjk_catalog() -> Arc<Catalog> {
    static CATALOG: OnceLock<Arc<Catalog>> = OnceLock::new();
    Arc::clone(CATALOG.get_or_init(|| with_inter(&[include_bytes!("../../../ui-contract/tests/fonts/NotoSansCJKsc-test.otf").as_slice()])))
}

/// What a story adds to the picker besides its state.
#[derive(Default)]
struct Extras<'a> {
    served: &'a [String],
    banner: Option<&'a str>,
    sets: &'a [FamilySet<'a>],
    scripts: &'a [FontScript],
    layout: FilterLayout,
    kind: FontPickerKind,
    actions: &'a [Label],
    shown_as: Option<&'a str>,
    advice: MissingAdvice,
    host: &'a [HostFontGroup<'a>],
}

fn show_with(ui: &mut Ui, data: &mut StoryData, init: Demo, catalog: &Arc<Catalog>, extras: &Extras<'_>) -> StoryOutput {
    let Demo { family, settings, filters } = data.value("picker", init);
    let mut picker = FontPicker::new("Font family", Bind::bound(family), catalog)
        .settings(Bind::bound(settings))
        .filters(Bind::bound(filters))
        .served(extras.served)
        .sets(extras.sets)
        .scripts(extras.scripts)
        .filter_layout(extras.layout)
        .kind(extras.kind)
        .actions(extras.actions)
        .advice(extras.advice)
        .host(extras.host);
    if let Some(face) = extras.shown_as {
        picker = picker.shown_as(face);
    }
    if let Some(text) = extras.banner {
        picker = picker.preview_text(text);
    }
    picker.show(ui).into()
}

fn show_picker(ui: &mut Ui, data: &mut StoryData, init: Demo) -> StoryOutput {
    show_with(ui, data, init, &catalog(), &Extras::default())
}

#[story(FontPicker<'static>, covers = [Default], drive = Drive::Type("Hack"))]
fn font_picker(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_picker(ui, data, demo("Ubuntu", FontPickerSettings::default(), FontFilters::default()))
}

/// The plain font box in a ribbon group's place (`ControlPlace::Ribbon`): held at the small field length whichever
/// family it shows, as a word processor's ribbon font box is; a row short of room narrows it no further than a name
/// field the short field length wide and its arrow.
#[story(FontPicker<'static>, covers = [], title = "In a ribbon")]
fn font_picker_in_ribbon(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let extras = Extras { kind: FontPickerKind::Plain, ..Extras::default() };
    craft_ui::runtime::env::with_env(
        ui,
        |overrides| {
            overrides.values.set(craft_ui::buttons::ControlPlace::Ribbon);
        },
        |ui| show_with(ui, data, demo("Ubuntu", FontPickerSettings::default(), FontFilters::default()), &catalog(), &extras),
    )
}

/// Focuses the field and types the start of a name, which opens the list on the matches.
fn type_query(fixture: &mut Fixture) {
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.type_text("u");
    fixture.settle();
}

#[story(FontPicker<'static>, covers = [Open], drive = Drive::Custom(type_query))]
fn font_picker_open_with_query(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = FontPickerSettings { recent: vec!["Ubuntu".to_owned()], ..FontPickerSettings::default() };
    show_picker(ui, data, demo("Ubuntu", settings, FontFilters::default()))
}

/// Focuses the field and opens the list on every family with Alt+Down.
fn open_list(fixture: &mut Fixture) {
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::ALT);
    fixture.settle();
}

/// The library list: Hack starred and recent, so the recent group and its rule head the list; Inter has two styles
/// to open and its name lines up with the one-style families'.
#[story(FontPicker<'static>, covers = [Open], drive = Drive::Custom(open_list))]
fn font_picker_open_with_filters(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = FontPickerSettings { favorites: vec!["Hack".to_owned()], recent: vec!["Hack".to_owned()], ..FontPickerSettings::default() };
    show_with(ui, data, demo("Inter", settings, FontFilters::default()), &styled_catalog(), &Extras::default())
}

/// Opens the list and expands Inter through its row's expand request: focus stays in the field, whose caret keeps
/// Left and Right. A disabled or read-only picker opens no list, and nothing expands.
fn expand_inter(fixture: &mut Fixture) {
    open_list(fixture);
    let Some(inter) = fixture.by_label("Inter") else { return };
    fixture.access(AccessActionKind::Expand, &inter);
    fixture.settle();
}

/// A family of two styles with its style rows showing.
#[story(FontPicker<'static>, covers = [Open], drive = Drive::Custom(expand_inter))]
fn font_picker_expanded_styles(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_with(ui, data, demo("Inter", FontPickerSettings::default(), FontFilters::default()), &styled_catalog(), &Extras::default())
}

/// Families the host can fetch, listed with the "Not installed" mark.
#[story(FontPicker<'static>, covers = [Open], drive = Drive::Custom(open_list))]
fn font_picker_served(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let served = ["Gotham".to_owned(), "Proxima Nova".to_owned()];
    show_with(
        ui,
        data,
        demo("Hack", FontPickerSettings::default(), FontFilters::default()),
        &catalog(),
        &Extras { served: &served, ..Extras::default() },
    )
}

/// A host group above the families: a face with its sample, an action and a notice, under the group's heading.
#[story(FontPicker<'static>, covers = [], drive = Drive::Custom(open_list))]
fn font_picker_host_groups(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = [
        HostFontRow::face("document-sans", "Document Sans", Some(FontRef::proportional())),
        HostFontRow::face("document-mono", "Document Mono", Some(FontRef::monospace())),
        HostFontRow::action("load", "Load font file"),
        HostFontRow::notice("Subset fonts can't take new text"),
    ];
    let host = [HostFontGroup::new(Some(Label::from("Document fonts")), &rows)];
    let extras = Extras { host: &host, ..Extras::default() };
    show_with(ui, data, demo("Ubuntu", FontPickerSettings::default(), FontFilters::default()), &catalog(), &extras)
}

/// The preview banner above the filters.
#[story(FontPicker<'static>, covers = [Open], drive = Drive::Custom(open_list))]
fn font_picker_banner_text(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let extras = Extras { banner: Some("The quick brown fox"), ..Extras::default() };
    show_with(ui, data, demo("Ubuntu", FontPickerSettings::default(), FontFilters::default()), &catalog(), &extras)
}

/// An Arabic sample text, at Large: every row's sample shows it in the family's face, Noto Sans Arabic (when the build
/// has the craft fonts set) draws it, and a face without Arabic letters shows its name.
#[story(FontPicker<'static>, covers = [Open], drive = Drive::Custom(open_list))]
fn font_picker_arabic_sample_text(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = FontPickerSettings { preview: crate::picker::FontPreviewSize::Large, ..FontPickerSettings::default() };
    let extras = Extras { banner: Some(ARABIC_SAMPLE_TEXT), ..Extras::default() };
    show_with(ui, data, demo("Inter", settings, FontFilters::default()), &arabic_catalog(), &extras)
}

/// The `Chips` filter area: the script row, a host set that is on and one that is disabled with its reason, the
/// classes, Variable and the preview size row.
#[story(FontPicker<'static>, covers = [Open], drive = Drive::Custom(open_list))]
fn font_picker_filter_chips(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let document = ["ubuntu".to_owned(), "hack".to_owned()];
    let added: [String; 0] = [];
    let sets = [
        FamilySet::new("document", "Document", &document),
        FamilySet::new("added", "Recently Added", &added).ordered().enabled(false).hint("No fonts were added recently"),
    ];
    let scripts = [FontScript::Latin, FontScript::Japanese, FontScript::Thai];
    let filters = FontFilters { sets: vec!["document".to_owned()], ..FontFilters::default() };
    let extras = Extras { sets: &sets, scripts: &scripts, layout: FilterLayout::Chips, ..Extras::default() };
    show_with(ui, data, demo("Hack", FontPickerSettings::default(), filters), &catalog(), &extras)
}

/// The "≈ family ✕" chip while Show Similar Fonts is on, and the Show Similar part on each row.
#[story(FontPicker<'static>, covers = [Open], drive = Drive::Custom(open_list))]
fn font_picker_filter_chips_similar(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let filters = FontFilters { similar: Some("Inter".to_owned()), ..FontFilters::default() };
    let extras = Extras { layout: FilterLayout::Chips, ..Extras::default() };
    show_with(ui, data, demo("Inter", FontPickerSettings::default(), filters), &styled_catalog(), &extras)
}

/// A Chinese sample text, at Large: Noto Sans CJK SC (a subset in the repository) draws it in its row and the banner,
/// on the baseline of the Latin rows and inside its band though its line is taller than Inter's, and Inter shows its
/// name.
#[story(FontPicker<'static>, covers = [Open], drive = Drive::Custom(open_list))]
fn font_picker_cjk_sample_text(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = FontPickerSettings { preview: crate::picker::FontPreviewSize::Large, ..FontPickerSettings::default() };
    let extras = Extras { banner: Some(CJK_SAMPLE_TEXT), ..Extras::default() };
    show_with(ui, data, demo("Noto Sans CJK SC", settings, FontFilters::default()), &cjk_catalog(), &extras)
}

/// "中文", which the repository's Noto Sans CJK SC subset draws.
const CJK_SAMPLE_TEXT: &str = "\u{4E2D}\u{6587}";

/// "أبجد هوز", the first words of the abjad order.
const ARABIC_SAMPLE_TEXT: &str = "\u{623}\u{628}\u{62C}\u{62F} \u{647}\u{648}\u{632}";

/// Settings whose recents are the families `recent`.
fn recents(recent: &[&str]) -> FontPickerSettings {
    FontPickerSettings { recent: recent.iter().map(|family| (*family).to_owned()).collect(), ..FontPickerSettings::default() }
}

/// Word's plain font box, closed.
#[story(FontPicker<'static>, covers = [], drive = Drive::Never)]
fn font_picker_plain(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let extras = Extras { kind: FontPickerKind::Plain, ..Extras::default() };
    show_with(ui, data, demo("Ubuntu", recents(&["Hack"]), FontFilters::default()), &catalog(), &extras)
}

/// The plain list open: recent fonts, all fonts and an action row.
#[story(FontPicker<'static>, covers = [Open], drive = Drive::Custom(open_list))]
fn font_picker_plain_open_with_recents(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let actions = [Label::from("Add Font…")];
    let extras = Extras { kind: FontPickerKind::Plain, actions: &actions, ..Extras::default() };
    show_with(ui, data, demo("Ubuntu", recents(&["Hack"]), FontFilters::default()), &catalog(), &extras)
}

/// The plain list on a typed query: matches only, no headings and no action rows.
#[story(FontPicker<'static>, covers = [Open], drive = Drive::Custom(type_query))]
fn font_picker_plain_typed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let actions = [Label::from("Add Font…")];
    let extras = Extras { kind: FontPickerKind::Plain, actions: &actions, ..Extras::default() };
    show_with(ui, data, demo("Ubuntu", recents(&["Ubuntu"]), FontFilters::default()), &catalog(), &extras)
}

/// A font the catalog lacks, shown in another face: the field carries the missing mark and its note.
#[story(FontPicker<'static>, covers = [], drive = Drive::Never)]
fn font_picker_plain_missing(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let actions = [Label::from("Add Font…")];
    let extras =
        Extras { kind: FontPickerKind::Plain, actions: &actions, shown_as: Some("Inter"), advice: MissingAdvice::AddFont, ..Extras::default() };
    show_with(ui, data, demo("Gotham", FontPickerSettings::default(), FontFilters::default()), &catalog(), &extras)
}

/// The family of a selection whose members differ.
#[story(FontPicker<'static>, covers = [Mixed], drive = Drive::Never)]
fn font_picker_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let catalog = catalog();
    FontPicker::new("Font family", data.bind_mixed("family", Mixed::<String>::Mixed), &catalog).show(ui).into()
}

/// A family the catalog does not hold, which the check refuses.
#[story(FontPicker<'static>, covers = [Invalid], drive = Drive::Never)]
fn font_picker_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let catalog = catalog();
    // The story's catalog is built once for the process, so the check reads it rather than taking it as an input.
    let family = data.bind("family", "Gotham".to_owned()).check(|family| {
        if self::catalog().styles(family).is_empty() { Validity::Refuse(Message::new("No such font is installed")) } else { Validity::Ok }
    });
    FontPicker::new("Font family", family, &catalog).show(ui).into()
}

/// A family name far longer than the field.
#[story(FontPicker<'static>, covers = [LongText], drive = Drive::Never)]
fn font_picker_long(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let catalog = catalog();
    FontPicker::new("Font family", data.bind("family", LONG_FAMILY.to_owned()), &catalog).show(ui).into()
}
