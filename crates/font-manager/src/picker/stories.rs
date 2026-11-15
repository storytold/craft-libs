//! Stories of `FontPicker`: the closed field, the list open with recents and a typed query, and the list open with
//! the class column and a filter on. They draw from the two faces egui ships in an in-memory catalog.

use std::sync::{Arc, OnceLock};

use craft_behavior::Mixed;
use craft_ui::contract::{Bind, Show as _, Validity};
use craft_ui::runtime::env::Message;
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::{Key, Modifiers, Ui};

use super::{FontFilters, FontPicker, FontPickerSettings};
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
}

fn show_with(ui: &mut Ui, data: &mut StoryData, init: Demo, catalog: &Arc<Catalog>, extras: &Extras<'_>) -> StoryOutput {
    let Demo { family, settings, filters } = data.value("picker", init);
    let mut picker = FontPicker::new("Font family", Bind::bound(family), catalog)
        .settings(Bind::bound(settings))
        .filters(Bind::bound(filters))
        .served(extras.served);
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

#[story(FontPicker<'static>, covers = [], drive = Drive::Custom(open_list))]
fn font_picker_open_with_filters(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = FontPickerSettings { favorites: vec!["Hack".to_owned()], ..FontPickerSettings::default() };
    show_picker(ui, data, demo("Hack", settings, FontFilters { favorites: true, ..FontFilters::default() }))
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
#[story(FontPicker<'static>, covers = [], drive = Drive::Custom(expand_inter))]
fn font_picker_expanded_styles(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_with(ui, data, demo("Inter", FontPickerSettings::default(), FontFilters::default()), &styled_catalog(), &Extras::default())
}

/// Families the host can fetch, listed with the "Not installed" mark.
#[story(FontPicker<'static>, covers = [], drive = Drive::Custom(open_list))]
fn font_picker_served(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let served = ["Gotham".to_owned(), "Proxima Nova".to_owned()];
    show_with(ui, data, demo("Hack", FontPickerSettings::default(), FontFilters::default()), &catalog(), &Extras { served: &served, banner: None })
}

/// The preview banner above the filters.
#[story(FontPicker<'static>, covers = [], drive = Drive::Custom(open_list))]
fn font_picker_banner_text(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let extras = Extras { served: &[], banner: Some("The quick brown fox") };
    show_with(ui, data, demo("Ubuntu", FontPickerSettings::default(), FontFilters::default()), &catalog(), &extras)
}

/// Large samples: a face with Arabic letters gets the second sample after the Latin one.
#[story(FontPicker<'static>, covers = [], drive = Drive::Custom(open_list))]
fn font_picker_arabic_sample(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = FontPickerSettings { preview: crate::picker::FontPreviewSize::Large, ..FontPickerSettings::default() };
    show_with(ui, data, demo("Inter", settings, FontFilters::default()), &styled_catalog(), &Extras::default())
}

/// The family of a selection whose members differ.
#[story(FontPicker<'static>, covers = [Mixed], drive = Drive::Never)]
fn font_picker_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let catalog = catalog();
    FontPicker::new("Font family", Bind::bound_mixed(data.value("family", Mixed::<String>::Mixed)), &catalog).show(ui).into()
}

/// A family the catalog does not hold, which the check refuses.
#[story(FontPicker<'static>, covers = [Invalid], drive = Drive::Never)]
fn font_picker_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let catalog = catalog();
    let known = catalog.clone();
    let check = move |family: &String| {
        if known.styles(family).is_empty() { Validity::Refuse(Message::new("No such font is installed")) } else { Validity::Ok }
    };
    FontPicker::new("Font family", Bind::bound(data.value("family", "Gotham".to_owned())).check(check), &catalog).show(ui).into()
}

/// A family name far longer than the field.
#[story(FontPicker<'static>, covers = [LongText], drive = Drive::Never)]
fn font_picker_long(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let catalog = catalog();
    FontPicker::new("Font family", Bind::bound(data.value("family", LONG_FAMILY.to_owned())), &catalog).show(ui).into()
}
