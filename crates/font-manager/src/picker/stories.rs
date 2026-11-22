//! Stories of `FontPicker`: the closed field, the list open with recents and a typed query, and the list open with
//! the class column and a filter on. They draw from the two faces egui ships in an in-memory catalog.

use std::sync::{Arc, OnceLock};

use craft_behavior::Mixed;
use craft_ui::contract::{Bind, Show as _, Validity};
use craft_ui::runtime::env::{Label, Message};
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::{Key, Modifiers, Ui};

use super::{FontFilters, FontPicker, FontPickerKind, FontPickerSettings, MissingAdvice};
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

/// The story catalog with Inter Regular and SemiBold, a family of two styles.
fn styled_catalog() -> Arc<Catalog> {
    static CATALOG: OnceLock<Arc<Catalog>> = OnceLock::new();
    Arc::clone(CATALOG.get_or_init(|| {
        let catalog = story_catalog();
        for bytes in [
            include_bytes!("../../../ui-contract/fonts/inter/Inter-Regular.ttf").as_slice(),
            include_bytes!("../../../ui-contract/fonts/inter/Inter-SemiBold.ttf").as_slice(),
        ] {
            let _ = catalog.add_bytes(bytes.to_vec(), Source::Bundled, &Limits::default());
        }
        catalog
    }))
}

/// What a story adds to the picker besides its state.
#[derive(Default)]
struct Extras<'a> {
    served: &'a [String],
    banner: Option<&'a str>,
    kind: FontPickerKind,
    actions: &'a [Label],
    shown_as: Option<&'a str>,
    advice: MissingAdvice,
}

fn show_with(ui: &mut Ui, data: &mut StoryData, init: Demo, catalog: &Arc<Catalog>, extras: &Extras<'_>) -> StoryOutput {
    let Demo { family, settings, filters } = data.value("picker", init);
    let mut picker = FontPicker::new("Font family", Bind::bound(family), catalog)
        .settings(Bind::bound(settings))
        .filters(Bind::bound(filters))
        .served(extras.served)
        .kind(extras.kind)
        .actions(extras.actions)
        .advice(extras.advice);
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

/// The preview banner above the filters.
#[story(FontPicker<'static>, covers = [Open], drive = Drive::Custom(open_list))]
fn font_picker_banner_text(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let extras = Extras { banner: Some("The quick brown fox"), ..Extras::default() };
    show_with(ui, data, demo("Ubuntu", FontPickerSettings::default(), FontFilters::default()), &catalog(), &extras)
}

/// Large samples: a face with Arabic letters gets the second sample after the Latin one.
#[story(FontPicker<'static>, covers = [Open], drive = Drive::Custom(open_list))]
fn font_picker_arabic_sample(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = FontPickerSettings { preview: crate::picker::FontPreviewSize::Large, ..FontPickerSettings::default() };
    show_with(ui, data, demo("Inter", settings, FontFilters::default()), &styled_catalog(), &Extras::default())
}

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
