//! [`FontPicker`]: the font family field of the type tool, the Character panel, the style panels and the Glyphs
//! panel. A text field that filters the installed families as the user types, with a list under it of family rows
//! drawn in their own face, favourites (stars), recent families at the top, class and feature filters above the
//! list and a "similar fonts" filter. Behaviour follows Photoshop's font menu.

pub(crate) mod banner;
pub(crate) mod rank;
mod rows;
#[cfg(feature = "suite")]
pub(crate) mod stories;

use std::sync::Arc;

use craft_behavior::nav::{StepResult, ValueAxes, ValueSteps, wheel_step};
use craft_behavior::{Codec, EditEvent, IconId, Mixed, NotAValue, PartKey, Sense, Value};
use craft_behavior::{HitLevel, HitPriority, NavMove, Preview, WheelPhase};
use craft_font_catalog::Catalog;
use craft_host::Section;
use craft_theme::variant::Tone;
use craft_ui::buttons::{ControlPlace, IconButton};
use craft_ui::contract::{
    Bind, Common, Component, Cx, Edit as _, EditResponse, Focus as _, InBar, Input, MeasureCx, Outcome, OverlayUi, Overlays as _, PaintUi, PartsCx,
    Pointer as _, Props as _, Remember as _, Role, Styled as _, WordSetCx as _, step_stop,
};
use craft_ui::controls::disclosure::{DisclosureGlyph, DropdownArrow};
use craft_ui::fonts::FontRef;
use craft_ui::list::{ListRow, ThumbSize, TrailingPart};
use craft_ui::primitives::{IconSpec, TextKey, TextOptions, TextRun};
use craft_ui::runtime::edit::PreviewWatch;
use craft_ui::runtime::env::{EnvView, Label};
use craft_ui::runtime::memory::StageAccess;
use craft_ui::style::{ComponentStyle, Metrics, Stateful, States, StyleColor, StyleCx, SurfaceLook};
use craft_ui::text::{Adornment, FieldStyle, place, reserve};
use egui::{Rect, Response, Vec2};
use serde::{Deserialize, Serialize};

use crate::missing::{DocumentFont, FontStatus, MissingFontsWords, document_fonts};
use crate::panel::browse::SCAN_POLL_SECONDS;
pub use rank::FamilyInfos;
use rank::{Info, shown_name};

craft_ui::words! {
    /// The picker's words, English by default.
    #[non_exhaustive]
    pub struct FontPickerWords {
        search: "fontPicker.search" = "Search fonts";
        recent: "fontPicker.recent" = "Recent";
        /// The `Plain` picker's heading over the recent families.
        recently_used: "fontPicker.recentlyUsed" = "Recently Used Fonts";
        /// The `Plain` picker's heading over every family.
        all_fonts: "fontPicker.allFonts" = "All Fonts";
        favorites: "fontPicker.favorites" = "Show favorite fonts";
        similar: "fontPicker.similar" = "Show similar fonts";
        variable: "fontPicker.variable" = "Show variable fonts";
        classes: "fontPicker.classes" = "Filter by class";
        all_classes: "fontPicker.allClasses" = "All Classes";
        clear_all: "fontPicker.clearAll" = "Clear All";
        sample: "fontPicker.sample" = "Sample";
        /// The name of the preview banner, with `{family}` for the family it shows.
        preview_of: template "fontPicker.previewOf" = "Font preview: {family}";
        no_match: "fontPicker.noMatch" = "No fonts match";
        /// The name of a row's star, with `{row}` for the row's label.
        favorite: template "fontPicker.favorite" = "Favorite {row}";
        list: "fontPicker.list" = "Fonts";
        hide_classes: "fontPicker.hideClasses" = "Hide Font Classes";
        show_classes: "fontPicker.showClasses" = "Show Font Classes";
        no_similar: "fontPicker.noSimilar" = "No similar fonts: the current font doesn't say what it looks like.";
        no_favorites: "fontPicker.noFavorites" = "No favorite fonts yet: click the star beside a font to add it.";
        no_match_filters: "fontPicker.noMatchFilters" = "No fonts match these filters.";
        serif: "fontPicker.classSerif" = "Serif";
        slab_serif: "fontPicker.classSlabSerif" = "Slab Serif";
        sans_serif: "fontPicker.classSansSerif" = "Sans Serif";
        script: "fontPicker.classScript" = "Script";
        monospace: "fontPicker.classMonospace" = "Monospace";
        handwritten: "fontPicker.classHandwritten" = "Handwritten";
        blackletter: "fontPicker.classBlackletter" = "Blackletter";
        decorative: "fontPicker.classDecorative" = "Decorative";
        show_styles: "fontPicker.showStyles" = "Show styles";
        hide_styles: "fontPicker.hideStyles" = "Hide styles";
        format_open_type: "fontPicker.formatOpenType" = "OpenType";
        format_true_type: "fontPicker.formatTrueType" = "TrueType";
        format_variable: "fontPicker.formatVariable" = "Variable";
        served: "fontPicker.served" = "Not installed";
        all_scripts: "fontPicker.allScripts" = "All Scripts";
        script_latin: "fontPicker.scriptLatin" = "Latin";
        script_japanese: "fontPicker.scriptJapanese" = "Japanese";
        script_thai: "fontPicker.scriptThai" = "Thai";
        any_kind: "fontPicker.anyKind" = "Any Kind";
        font_technology: "fontPicker.fontTechnology" = "Font Technology";
        preview_size: "fontPicker.previewSize" = "Font Preview Size";
        preview_small: "fontPicker.previewSmall" = "Small";
        preview_medium: "fontPicker.previewMedium" = "Medium";
        preview_large: "fontPicker.previewLarge" = "Large";
        /// The similar-fonts chip, with `{family}` for the reference family.
        similar_to: template "fontPicker.similarTo" = "≈ {family}";
        clear_similar: "fontPicker.clearSimilar" = "Clear Similar Fonts";
        /// The name of a row's show-similar action, with `{row}` for the row's label.
        show_similar: template "fontPicker.showSimilar" = "Show fonts similar to {row}";
        /// The field's text while the installed fonts are still being read.
        loading: "fontPicker.loading" = "Loading fonts…";
    }
}

/// Photoshop's font classes, in its filter menu order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FontClass {
    /// Serif faces.
    Serif,
    /// Slab serif faces.
    SlabSerif,
    /// Sans serif faces.
    SansSerif,
    /// Script faces.
    Script,
    /// Monospaced faces.
    Monospace,
    /// Handwritten faces.
    Handwritten,
    /// Blackletter faces.
    Blackletter,
    /// Decorative faces.
    Decorative,
}

impl FontClass {
    /// Every class, in menu order.
    pub const ALL: [FontClass; 8] = [
        FontClass::Serif,
        FontClass::SlabSerif,
        FontClass::SansSerif,
        FontClass::Script,
        FontClass::Monospace,
        FontClass::Handwritten,
        FontClass::Blackletter,
        FontClass::Decorative,
    ];

    /// The class of a family from its PANOSE and OS/2 data, else from its name; `None` when the
    /// face can't be read.
    pub fn of(catalog: &Catalog, family: &str) -> Option<FontClass> {
        rank::class_of(catalog, family)
    }
}

/// Photoshop's Font Preview Size (Type › Font Preview Size): how large the sample in each row is,
/// or none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FontPreviewSize {
    /// No sample in the rows.
    None,
    /// A small sample.
    Small,
    /// A medium sample (the default).
    #[default]
    Medium,
    /// A large sample.
    Large,
    /// An extra large sample.
    ExtraLarge,
    /// A huge sample.
    Huge,
}

impl FontPreviewSize {
    /// The height of the sample's thumbnail in a row; `ThumbSize::None` for no sample. The three largest sizes
    /// share the largest thumbnail.
    pub fn thumb(self) -> ThumbSize {
        match self {
            FontPreviewSize::None => ThumbSize::None,
            FontPreviewSize::Small => ThumbSize::Small,
            FontPreviewSize::Medium => ThumbSize::Medium,
            FontPreviewSize::Large | FontPreviewSize::ExtraLarge | FontPreviewSize::Huge => ThumbSize::Large,
        }
    }
}

/// Coverage from the face: Latin, the basic Latin letters in the cmap; Japanese, the OS/2 kana or CJK unicode-range
/// bits or the JIS code page bit, or a Japanese family name; Thai, U+0E01 and U+0E32 in the cmap. Read lazily like the
/// class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FontScript {
    /// The basic Latin letters.
    Latin,
    /// Japanese kana and CJK.
    Japanese,
    /// Thai.
    Thai,
}

impl FontScript {
    /// Every script the script row can offer, in order.
    pub const ALL: [FontScript; 3] = [FontScript::Latin, FontScript::Japanese, FontScript::Thai];
}

/// A family set the host gives (VectorCraft: Document, Recently Added), one filter toggle each. An `ordered` set that
/// is on lists in its own order. A disabled set's toggle is off and dimmed, with its hint as the reason.
pub struct FamilySet<'a> {
    key: &'a str,
    label: Label,
    hint: Option<Label>,
    families: &'a [String],
    ordered: bool,
    enabled: bool,
}

impl<'a> FamilySet<'a> {
    /// The set `key` named `label` over `families`.
    pub fn new(key: &'a str, label: impl Into<Label>, families: &'a [String]) -> Self {
        FamilySet { key, label: label.into(), hint: None, families, ordered: false, enabled: true }
    }

    /// The reason shown while the set is disabled.
    pub fn hint(mut self, hint: impl Into<Label>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Lists the set in its own order while it is on.
    pub fn ordered(mut self) -> Self {
        self.ordered = true;
        self
    }

    /// Whether the set's toggle can be turned on.
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    /// The family's place in the set. A host may give canonical lowercase names, so case does not matter.
    fn position(&self, family: &str) -> Option<usize> {
        self.families.iter().position(|f| f.eq_ignore_ascii_case(family))
    }
}

/// Named layout of the list's filter area. `Toolbar`: the toggle row and class column. `Chips`: rows of chip toggles
/// (the script row; Favorites, the host sets, Any Kind and the classes; Font Technology and Variable; the Font
/// Preview Size row).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum FilterLayout {
    /// The toggle row and class column.
    #[default]
    Toolbar,
    /// Rows of chip toggles.
    Chips,
}

/// What narrows the list. Every filter off shows every installed family.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct FontFilters {
    /// Only families of this class (`None`: All Classes).
    pub class: Option<FontClass>,
    /// Only starred families.
    pub favorites: bool,
    /// Only variable fonts.
    pub variable: bool,
    /// The reference family of Show Similar Fonts, most alike first ([`SIMILAR_COUNT`] of them); `None` is off.
    pub similar: Option<String>,
    /// Only families whose regular face covers the script.
    pub script: Option<FontScript>,
    /// Keys of the host family sets that are on; each keeps only its families.
    pub sets: Vec<String>,
}

impl FontFilters {
    /// Whether any filter narrows the list (Clear All is enabled).
    pub fn any(&self) -> bool {
        self.class.is_some() || self.favorites || self.variable || self.similar.is_some() || self.script.is_some() || !self.sets.is_empty()
    }

    /// Clear All: every filter off.
    pub fn clear(&mut self) {
        *self = FontFilters::default();
    }
}

/// How many families "similar fonts" shows.
pub const SIMILAR_COUNT: usize = 30;

/// The picker's remembered state: starred families and the most recent picks, one settings
/// section shared by every picker of the app (`[font_picker]`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct FontPickerSettings {
    /// Starred families, in star order.
    pub favorites: Vec<String>,
    /// Most recent first, at most [`FontPickerSettings::recent_limit`].
    pub recent: Vec<String>,
    /// How many recent families the list shows (Photoshop: Number of Recent Fonts to Display, 10).
    pub recent_limit: usize,
    /// How large the sample in each row is.
    pub preview: FontPreviewSize,
    /// Show family names in English rather than their own language (Type › Show Font Names in English).
    pub names_in_english: bool,
    /// Row samples and the banner paint black type on a white paper fill, in every theme.
    pub samples_on_white: bool,
}

impl Default for FontPickerSettings {
    fn default() -> Self {
        FontPickerSettings {
            favorites: Vec::new(),
            recent: Vec::new(),
            recent_limit: 10,
            preview: FontPreviewSize::default(),
            names_in_english: false,
            samples_on_white: false,
        }
    }
}

impl Section for FontPickerSettings {
    const KEY: &'static str = "font_picker";
}

impl FontPickerSettings {
    /// Put `family` first in the recents, dropping a duplicate and anything past the limit.
    // pc: crates/ui-egui/src/app.rs `note_recent_font`.
    pub fn note_recent(&mut self, family: &str) {
        self.recent.retain(|f| f != family);
        self.recent.insert(0, family.to_owned());
        self.recent.truncate(self.recent_limit);
    }

    /// Star or unstar `family`.
    pub fn set_favorite(&mut self, family: &str, on: bool) {
        let starred = self.favorites.iter().any(|f| f == family);
        if on && !starred {
            self.favorites.push(family.to_owned());
        } else if !on {
            self.favorites.retain(|f| f != family);
        }
    }
}

/// The format of a family's fonts, shown as a badge on its row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FontFormat {
    /// CFF or composite outlines.
    OpenType,
    /// TrueType outlines.
    TrueType,
    /// Any face of the family is variable.
    Variable,
}

impl FontFormat {
    /// The badge text: "O", "TT" or "VAR".
    pub fn badge(self) -> &'static str {
        match self {
            FontFormat::OpenType => "O",
            FontFormat::TrueType => "TT",
            FontFormat::Variable => "VAR",
        }
    }
}

/// One row of the list, for apps that draw their own list from the picker's filtering ([`family_rows`]).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FamilyRow {
    /// The family as the catalog names it.
    pub family: String,
    /// The name shown: the family's name in the UI language unless `names_in_english`.
    pub shown: String,
    /// The family is a favourite.
    pub starred: bool,
    /// The family's class; `None` for a served row and a family no class fits.
    pub class: Option<FontClass>,
    /// The family's format; `None` only for a served row.
    pub format: Option<FontFormat>,
    /// How many styles the family has.
    pub styles: usize,
    /// The row is in the Recent group at the top.
    pub recent: bool,
}

/// The families `listing` would show, in list order: recents first (when the query is empty and no filter is on),
/// then the matching families, the best match first and ties in the order of `language` (its code, `ja`, `pt-BR`).
/// For apps that draw their own list from the picker's filtering. `infos` is [`FamilyInfos::of`] of `catalog`, built
/// again by the app when the catalog's generation moves.
pub fn family_rows(
    catalog: &Arc<Catalog>,
    infos: &FamilyInfos,
    query: &str,
    settings: Option<&FontPickerSettings>,
    filters: &FontFilters,
    sets: &[FamilySet<'_>],
    language: &str,
) -> Vec<FamilyRow> {
    let (folder, collator) = (craft_i18n::Folder::new(language), craft_i18n::Collator::new(language));
    let listing = rank::Listing { query, settings, filters, language, served: &[], sets };
    rank::list(catalog, infos, &listing, &rank::Language { folder: &folder, collator: &collator }, true).0
}

/// The family data of `catalog` for the component `cx`: kept across passes while the catalog and its generation are unchanged.
pub(super) fn infos_of(ctx: &egui::Context, id: egui::Id, catalog: &Catalog) -> Arc<FamilyInfos> {
    craft_ui::runtime::derived::derived(ctx, id, &(catalog.id(), catalog.generation()), || FamilyInfos::of(catalog))
}

/// How a [`FontPicker`] lays out its list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum FontPickerKind {
    /// The library picker: stars, recents, filters and the class column.
    #[default]
    Library,
    /// Word's plain font box: recent fonts, actions and all fonts.
    Plain,
}

/// What the note of a shown font that is not installed advises.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum MissingAdvice {
    /// The note alone.
    #[default]
    None,
    /// The note, then the advice to add the font file with the host's Add Font row.
    AddFont,
}

/// A group of host rows listed above the catalogue families, under an optional heading.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct HostFontGroup<'a> {
    /// The group's heading, if it has one.
    pub heading: Option<Label>,
    /// The group's rows, in order.
    pub rows: &'a [HostFontRow],
}

impl<'a> HostFontGroup<'a> {
    /// A group of `rows` under `heading`.
    pub fn new(heading: Option<Label>, rows: &'a [HostFontRow]) -> Self {
        HostFontGroup { heading, rows }
    }
}

/// A row the host adds to the picker: a face, an action or a notice.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct HostFontRow {
    /// The key `host_picked` reports when the row is run.
    pub key: String,
    /// The row's name.
    pub name: Label,
    /// What the row is.
    pub kind: HostFontKind,
}

impl HostFontRow {
    /// A face row; it shows its sample in `face` when given.
    pub fn face(key: impl Into<String>, name: impl Into<Label>, face: Option<FontRef>) -> Self {
        HostFontRow { key: key.into(), name: name.into(), kind: HostFontKind::Face(face) }
    }

    /// An action row; it runs and picks no family.
    pub fn action(key: impl Into<String>, name: impl Into<Label>) -> Self {
        HostFontRow { key: key.into(), name: name.into(), kind: HostFontKind::Action }
    }

    /// A notice row: disabled text.
    pub fn notice(name: impl Into<Label>) -> Self {
        HostFontRow { key: String::new(), name: name.into(), kind: HostFontKind::Notice }
    }
}

/// What a [`HostFontRow`] is.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum HostFontKind {
    /// A face to pick, with the face its sample shows in.
    Face(Option<FontRef>),
    /// An action that runs and picks no family.
    Action,
    /// Disabled text.
    Notice,
}

/// What a [`FontPicker`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct FontPickerOutput {
    /// The field's edit: a pick commits the family.
    pub edit: EditResponse<String>,
    /// The style picked in an expanded family's style rows.
    pub picked_style: Option<String>,
    /// The family the pointer or the keyboard has rested on, for the host to fetch it when it is served.
    pub rested: Option<String>,
    /// A family was picked: Enter or a click on a row, Enter on the exact typed name, or the wheel or arrows on the
    /// closed field. The picker has already noted it as recent.
    pub picked: Option<String>,
    /// What this pass did to the live preview of the selected text: the family under the pointer or the keyboard row
    /// of the open list (Photoshop previews on hover). It ends when the pointer leaves the rows, the list closes
    /// (Escape, a press outside) or a row is picked. The field's value and text stay.
    pub preview: Preview<String>,
    /// The index of the action row picked, in the `actions` the host gave.
    pub action: Option<usize>,
    /// The key of the host face or action row run in this pass; a host pick does not write the `family` binding.
    pub host_picked: Option<String>,
}

impl Outcome for FontPickerOutput {
    type Value = String;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// The text part.
const TEXT: PartKey = PartKey::new("text");
/// The chevron that opens and closes the list.
const CHEVRON: PartKey = PartKey::new("chevron");
/// The mark before the chevron of a shown font that is not installed; its tooltip is the note.
const WARNING: PartKey = PartKey::new("missing");
const WARNING_ICON: &str = "warning_filled";
/// The popup the chevron and Alt+Down toggle.
const POPUP: &str = "list";
/// The Show Similar part at the end of a family row, in the `Chips` layout.
const SIMILAR: TrailingPart = TrailingPart::new(0);
/// The share of a sample band's height the sample's em takes, in the list's rows and the preview banner.
const SAMPLE_EM: f32 = 0.7;
/// The widest family names in common use, which the field's width is sized by.
const NAME_SAMPLE: &str = "Source Sans Pro Semibold Condensed";

fn text_key() -> TextKey<String> {
    TextKey::new(TEXT)
}

/// The family field's text: the family's name in the interface language, and a typed name read back as a family.
#[derive(Clone)]
struct FamilyCodec {
    infos: Arc<FamilyInfos>,
    language: String,
    /// A typed name that is no installed family is a value too (Word's font box applies it as typed).
    free: bool,
    /// The name of the host row shown as the field's value, which stands in for the family's name.
    host: Option<String>,
}

impl FamilyCodec {
    fn shown(&self, family: &str, info: &Info) -> String {
        shown_name(info, family, &self.language, false)
    }
}

impl Codec for FamilyCodec {
    type Value = String;

    fn format(&self, family: &String) -> String {
        if let Some(name) = &self.host {
            return name.clone();
        }
        self.infos.families.get(family).map_or_else(|| family.clone(), |info| self.shown(family, info))
    }

    /// A family whose name, shown or in the catalog, is the text apart from case; for a `free` codec, else the
    /// trimmed text when it is not empty.
    fn parse(&self, text: &str, _: &String) -> Result<String, NotAValue> {
        let wanted = text.trim().to_lowercase();
        let named = |(family, info): (&String, &Info)| {
            (family.to_lowercase() == wanted || self.shown(family, info).to_lowercase() == wanted).then(|| family.clone())
        };
        match self.infos.families.iter().find_map(named) {
            Some(family) => Ok(family),
            None if self.free && !wanted.is_empty() => Ok(text.trim().to_owned()),
            None => Err(NotAValue),
        }
    }

    fn sample(&self, current: &String) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Owned(format!("{}{}", NAME_SAMPLE, self.format(current)))
    }
}

/// The field's text while the installed fonts are still being read: the loading words, whatever the family.
#[derive(Clone)]
struct Loading(Label);

impl Codec for Loading {
    type Value = String;

    fn format(&self, _: &String) -> String {
        self.0.as_str().to_owned()
    }

    fn parse(&self, _: &str, _: &String) -> Result<String, NotAValue> {
        Err(NotAValue)
    }
}

/// What the picker keeps between passes: the draft the list was last filtered for, the family the list previews,
/// whether the class column is hidden, the watch that reports a preview the picker stopped showing, and, for the open
/// list only, the families whose styles show and the families already reported as rested.
#[derive(Clone, Default)]
struct Typing {
    /// Armed while a preview shows: the picker's eviction then ends the preview it proposed.
    watch: PreviewWatch,
    draft: Option<String>,
    lead: Option<String>,
    /// The lead's family was proposed by the pointer, so it ends when the pointer leaves the rows.
    lead_by_pointer: bool,
    /// The pointer was over the open list's rows in the last pass.
    over_rows: bool,
    hide_classes: bool,
    expanded: Vec<String>,
    rested: Vec<String>,
    focused: bool,
}

/// The interface language code of the host, English without one.
fn ui_language(env: &EnvView<'_>) -> String {
    env.host().and_then(|host| host.i18n_shared().map(|i18n| i18n.code(host.lang()).to_owned())).unwrap_or_else(|| "en".to_owned())
}

/// Writes `event` to `bind` as the pipeline would: the binding's check and policy apply, and a given binding takes
/// nothing. Returns whether the value changed.
pub(crate) fn write_event<T: Value>(bind: &mut Bind<'_, T>, cx: &mut impl StageAccess, event: &EditEvent<T>) -> bool {
    bind.reborrow().write(cx, event)
}

/// Writes `after` to `bind` as one change named `label`.
pub(crate) fn write_bind<T: Value>(bind: &mut Bind<'_, T>, cx: &mut impl StageAccess, after: T, label: &str) {
    let mut at = bind.reborrow();
    let before = at.read(cx).unwrap_or(Mixed::Mixed);
    at.write(cx, &EditEvent::set(before, after, Arc::from(label), None));
}

/// The value of `bind`, if it holds one.
pub(crate) fn read_bind<T: Value>(bind: &mut Bind<'_, T>, cx: &mut impl StageAccess) -> Option<T> {
    match bind.reborrow().read(cx)? {
        Mixed::Same(value) => Some(value),
        Mixed::Mixed => None,
    }
}

fn write<T: Value>(bind: &mut Option<Bind<'_, T>>, cx: &mut impl StageAccess, after: T, label: &str) {
    if let Some(bind) = bind {
        write_bind(bind, cx, after, label);
    }
}

fn read<T: Value>(bind: &mut Option<Bind<'_, T>>, cx: &mut impl StageAccess) -> Option<T> {
    read_bind(bind.as_mut()?, cx)
}

/// A stop of the closed field's arrows and wheel: a host face or action row, or a family.
enum Stop {
    Host(String),
    Family(String),
}

/// The font family field. Typing filters the list to the families whose name (in any language) matches, the best
/// match first, and opens it; in the library picker a click on the name while the list is closed opens it on every
/// family and selects the name, whereas in `Plain` only a click that focuses the field selects the name without opening
/// the list (subsequent clicks place the caret). The chevron and Alt+Down open the list on every family, and Alt+Up
/// closes it. Rows: a star, the
/// family name in the interface face, the sample text (the preview text, else "Sample") in the family's own face at the
/// preview size and the class mark.
/// Recent families form a group at the top. Up, Down and Page keys move the keyboard row, which previews, Enter or a
/// click on a row picks it, Enter on a typed name that is a family picks that family, Escape closes the list and
/// restores the field, and Tab picks the keyboard row and moves on. While an input method composes, Enter, Escape
/// and the arrows are the composition's. On the closed, focused field the arrows and the wheel step to the previous
/// or next family in list order and pick it. The list is the field's own popup: filter toggles above, the class
/// column at the reading start and the families beside it. Host groups (`host`) list above the families: a face or
/// action row is reported as `host_picked` and writes no family, and a notice row is disabled text.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(overlay)]
pub struct FontPicker<'a> {
    #[common]
    common: Common,
    family: Bind<'a, String>,
    catalog: &'a Arc<Catalog>,
    /// Stars, recents, the preview size and the names language; the picker records a pick as recent through it.
    #[prop]
    settings: Option<Bind<'a, FontPickerSettings>>,
    /// The filters, kept by the caller so they survive the list closing; without them the filter row is hidden.
    #[prop]
    filters: Option<Bind<'a, FontFilters>>,
    picked: Option<String>,
    picked_style: Option<String>,
    rested: Option<String>,
    preview: Preview<String>,
    /// The preview banner's text and every row's sample (the sample word when not given).
    #[prop]
    preview_text: Option<&'a str>,
    /// Families the host can fetch but that are not installed, listed with the served mark.
    #[prop]
    served: &'a [String],
    /// The host's family sets; their toggles follow Favorites.
    #[prop]
    sets: &'a [FamilySet<'a>],
    /// The scripts the script row offers (with All Scripts first); empty: no script row.
    #[prop]
    scripts: &'a [FontScript],
    /// Picks the layout of the filter area.
    #[prop]
    filter_layout: FilterLayout,
    /// How the list is laid out: the library picker, or Word's plain font box.
    #[prop]
    kind: FontPickerKind,
    /// Action rows after the families (the `Plain` box), reported through `FontPickerOutput::action`.
    #[prop]
    actions: &'a [Label],
    /// The face the shown font is displayed in when it is not installed, for the note beside it.
    #[prop]
    shown_as: Option<&'a str>,
    /// What the note advises beyond naming the face; `AddFont` when an action row adds a font.
    #[prop]
    advice: MissingAdvice,
    action: Option<usize>,
    /// Host groups listed above the catalogue families, in order, with a separator between groups.
    #[prop]
    host: &'a [HostFontGroup<'a>],
    /// The key of the host row shown as the field's value.
    #[prop]
    current_host: Option<String>,
    host_picked: Option<String>,
    /// Whether the font picker uses compact bar sizing (`size.length.field.sm` / `size.length.short`).
    #[style_key]
    #[prop]
    compact: bool,
}

/// How many times a closed [`FontPicker`] built its stop list on `ctx`, for regression trials.
#[cfg(feature = "screen-testing")]
#[derive(Clone, Copy, Default)]
struct StopBuilds(u64);

/// Where the stop-build count lives in the context's memory.
#[cfg(feature = "screen-testing")]
fn stop_builds_id() -> egui::Id {
    craft_ui::runtime::memory::global("font_picker_stop_builds", ())
}

/// How many times a closed [`FontPicker`] built its stop list on `ctx`.
#[doc(hidden)]
#[cfg(feature = "screen-testing")]
pub fn stop_builds(ctx: &egui::Context) -> u64 {
    craft_ui::runtime::memory::with_session::<StopBuilds, _>(ctx, stop_builds_id(), |builds| builds.0)
}

impl<'a> FontPicker<'a> {
    /// A picker named `label` over the family `family` of `catalog`.
    pub fn new(label: impl Into<Label>, family: Bind<'a, String>, catalog: &'a Arc<Catalog>) -> Self {
        FontPicker {
            common: Common::new(),
            family,
            catalog,
            settings: None,
            filters: None,
            picked: None,
            picked_style: None,
            rested: None,
            preview: Preview::Unchanged,
            preview_text: None,
            served: &[],
            sets: &[],
            scripts: &[],
            filter_layout: FilterLayout::default(),
            kind: FontPickerKind::default(),
            actions: &[],
            shown_as: None,
            advice: MissingAdvice::default(),
            action: None,
            host: &[],
            current_host: None,
            host_picked: None,
            compact: false,
        }
        .label(label)
        .enabled(catalog.is_ready())
    }

    /// The adornments at the end of the field: the missing mark when `missing`, then the chevron. The mark is a glyph
    /// whose part carries the note.
    fn trailing(name: Label, missing: bool, arrow: DropdownArrow) -> Vec<Adornment> {
        let mut all = Vec::with_capacity(2);
        if missing {
            all.push(Adornment::glyph(IconSpec::svg(IconId::new(WARNING_ICON))));
        }
        all.push(Adornment::button(CHEVRON, DisclosureGlyph::dropdown_of(arrow), name));
        all
    }

    /// The note for `family` when it is shown in another face because the catalog lacks it, by the rule the Missing
    /// Fonts dialog uses: the missing-font sentence, then the advice the host chose. A host row shown as the value
    /// has none.
    fn missing_note(&self, family: &str, words: &MissingFontsWords) -> Option<Label> {
        let face = self.shown_as.filter(|_| self.catalog.is_ready() && self.host_name().is_none())?;
        let status = document_fonts(self.catalog, &[DocumentFont::new(family, "", None)]).into_iter().next()?.status;
        if status != FontStatus::Missing {
            return None;
        }
        let mut note = words.shown_as.fill(&[("family", family), ("face", face)]);
        if self.advice == MissingAdvice::AddFont {
            note.push(' ');
            note.push_str(words.add_font.as_str());
        }
        Some(Label::from(note))
    }

    fn codec(&self, infos: Arc<FamilyInfos>, env: &EnvView<'_>) -> FamilyCodec {
        FamilyCodec { infos, language: ui_language(env), free: self.kind == FontPickerKind::Plain, host: self.host_name() }
    }

    /// The name of the host row `current_host` names, which the field shows in place of the family.
    fn host_name(&self) -> Option<String> {
        let key = self.current_host.as_deref()?;
        self.host
            .iter()
            .flat_map(|group| group.rows.iter())
            .find(|row| row.key == key && !matches!(row.kind, HostFontKind::Notice))
            .map(|row| row.name.as_str().to_owned())
    }

    /// What the closed field's arrows and wheel step through: the host face and action rows in group order, then the
    /// families in the list's narrowed order. A notice takes no step.
    fn stops(&self, cx: &Cx<'_, Self>, settings: Option<&FontPickerSettings>, filters: &FontFilters) -> Vec<Stop> {
        #[cfg(feature = "screen-testing")]
        craft_ui::runtime::memory::with_session::<StopBuilds, _>(&cx.response.ctx, stop_builds_id(), |builds| builds.0 += 1);
        let folder = cx.env.folder();
        let host = rank::host_rows(self.host, "", &folder)
            .into_iter()
            .filter_map(|(group, at)| self.host.get(group)?.rows.get(at))
            .filter(|row| !matches!(row.kind, HostFontKind::Notice))
            .map(|row| Stop::Host(row.key.clone()));
        host.chain(self.ordered(cx, settings, filters).into_iter().map(|row| Stop::Family(row.family))).collect()
    }

    /// The families in the list's narrowed order (the bound filters and the host's sets apply) without the Recent
    /// group, which the arrows and the wheel step through.
    fn ordered(&self, cx: &Cx<'_, Self>, settings: Option<&FontPickerSettings>, filters: &FontFilters) -> Vec<FamilyRow> {
        let infos = infos_of(&cx.response.ctx, cx.id, self.catalog);
        let (folder, collator) = (cx.env.folder(), cx.env.collator());
        let language = ui_language(&cx.env);
        let listing = rank::Listing { query: "", settings, filters, language: &language, served: &[], sets: self.sets };
        let mut rows = rank::list(self.catalog, &infos, &listing, &rank::Language { folder: &folder, collator: &collator }, true).0;
        rows.retain(|row| !row.recent);
        rows
    }
}

/// The style of [`FontPicker`]: the field look and the colour of the missing-font warning icon.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FontPickerStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    /// The field look the adornments are placed by; `surface` and `metrics` are its own.
    field: FieldStyle,
    /// The ink of the missing-font warning icon (`status.warn.fg`), dimmed to `text.disabled` while disabled.
    warning: Stateful<StyleColor>,
    /// The list's sample height and family row height at each preview size, none first: the thumbnail sides
    /// (`size.thumb.*`) and the height of a list row holding one.
    pub(crate) samples: [Sample; 4],
    /// The least width of the list's name column: [`NAME_SAMPLE`] in a row's text, the names the field holds.
    pub(crate) name_column: f32,
}

/// The sample of a family row at one preview size: how high its band is, and how high the row holding it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sample {
    pub(crate) side: f32,
    pub(crate) row: f32,
}

impl FontPickerStyle {
    /// The sample at `size`.
    pub(crate) fn sample(&self, size: ThumbSize) -> Sample {
        match size {
            ThumbSize::Small => self.samples[1],
            ThumbSize::Medium => self.samples[2],
            ThumbSize::Large => self.samples[3],
            _ => self.samples[0],
        }
    }
}

impl Component for FontPicker<'_> {
    const ROLE: Role = Role::COMBO_BOX;
    const INPUT: Input = Input::Text;
    const KEYS: craft_ui::runtime::focus::KeyUse = craft_ui::runtime::focus::KeyUse::of(Input::Text).with_moves();
    type Style = FontPickerStyle;
    type Output = FontPickerOutput;

    /// A field; in a ribbon (`ControlPlace::Ribbon`) or toolbar (`InBar`), or when `compact` is set,
    /// held at the small field length (`size.length.field.sm`), as a ribbon's font box is:
    /// wide enough for a family name and its menu arrow, the same width whichever it shows.
    fn style(cx: &StyleCx<'_>, key: FontPickerStyleKey) -> FontPickerStyle {
        let field = FieldStyle::at(cx);
        let in_bar = cx.env::<InBar>().0;
        let ribbon = cx.env::<ControlPlace>() == ControlPlace::Ribbon;
        let field = if key.compact || ribbon || in_bar { field.held_at(cx.tokens.size.length.field.sm) } else { field };
        let warn = cx.tone(Tone::Warn);
        let off = cx.color(cx.tokens.color.text.disabled);
        let thumb = &cx.tokens.size.thumb;
        let sample = |size: ThumbSize, side: f32| Sample { side, row: ListRow::row_height(cx, size, false) };
        let samples = [
            sample(ThumbSize::None, 0.0),
            sample(ThumbSize::Small, thumb.sm),
            sample(ThumbSize::Medium, thumb.md),
            sample(ThumbSize::Large, thumb.lg),
        ];
        let warning = Stateful::new(warn).off(States::new(warn, warn, warn, off));
        let name_column = cx.text_width(craft_theme::TypeRole::Body, NAME_SAMPLE);
        FontPickerStyle { surface: cx.field(), metrics: field.metrics(), field, warning, samples, name_column }
    }

    /// The wheel steps the family through the list.
    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS | Sense::WHEEL_Y
    }

    /// The width of the longest family names in common use, so the field keeps its size whichever family is picked.
    /// In a ribbon or toolbar the name field is the short field length (`size.length.short`) wide whichever family it shows, a
    /// longer name scrolling in it as in a word processor's font box: that is its least, about the old ribbon's font
    /// name, so a row short of room never narrows the box to its menu arrow; where the room allows the style holds
    /// the box at the small field length (`FieldStyle::held_at`).
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let host = self.host_name();
        let current = host.as_deref().or(cx.value().map(String::as_str)).unwrap_or("");
        let widest = [NAME_SAMPLE, current].into_iter().map(|text| cx.field_text(&TextRun::new(text))).fold(Vec2::ZERO, Vec2::max);
        let compact = self.compact || cx.env.get::<ControlPlace>() == ControlPlace::Ribbon || cx.env.get::<InBar>().0;
        let widest = if compact { Vec2::new(cx.tokens.size.length.short, widest.y) } else { widest };
        let name = cx.words::<FontPickerWords>().list.clone();
        let note = self.missing_note(current, &cx.words::<MissingFontsWords>());
        Vec2::new(widest.x + reserve(&cx.style.field, &[], &Self::trailing(name, note.is_some(), cx.env.get())), widest.y)
    }

    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let name = cx.words::<FontPickerWords>().list.clone();
        let family = cx.value().cloned().unwrap_or_default();
        let note = self.missing_note(&family, &cx.words::<MissingFontsWords>());
        let trailing = Self::trailing(name, note.is_some(), cx.env.get());
        let placed = place(cx.content_rect, &cx.style.field, &[], &trailing, cx.flow.is_rtl());
        if let (Some(note), Some(at)) = (note, placed.trailing.first()) {
            cx.part(WARNING, *at).tooltip(note);
        }
        if let Some(at) = placed.trailing.last() {
            cx.part(CHEVRON, *at)
                .sense(Sense::CLICK | Sense::KEEP_FOCUS)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .pointer_only("Alt+Down opens the list")
                .toggles(POPUP);
        }
        let mut options = TextOptions::new();
        options.select_on_focus = true;
        // A selection of several families shows an empty field, as the reference applications do.
        options.mixed = cx.value().is_none();
        if !self.catalog.is_ready() {
            // The first scan runs on a background thread; the field asks again until it ends.
            self.catalog.scan_in_background();
            craft_ui::runtime::frame::request_repaint_after(cx.ui.ctx(), SCAN_POLL_SECONDS);
            let loading = Loading(cx.words::<FontPickerWords>().loading.clone());
            cx.text(text_key(), placed.text, &loading, &family, &options);
            return;
        }
        let codec = self.codec(infos_of(cx.ui.ctx(), cx.id, self.catalog), &cx.env);
        cx.text(text_key(), placed.text, &codec, &family, &options);
    }

    fn bind(&mut self) -> Option<craft_ui::contract::BindMut<'_, String>> {
        Some(self.family.reborrow())
    }

    /// A changed draft filters the list: it opens on the matches, and closes when the draft ended (committed,
    /// cancelled or replaced by a pick). A preview that the list closed on without a pick is cancelled. While the
    /// list is closed, the arrows and the wheel step through the host rows and the families.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if !self.catalog.is_ready() {
            return;
        }
        let focused = cx.is_focused();
        let was_focused = std::mem::replace(&mut cx.memory::<Typing>().focused, focused);
        let list = cx.popup(POPUP);
        let draft = cx.text(&text_key()).and_then(|outcome| outcome.draft.clone());
        let before = std::mem::replace(&mut cx.memory::<Typing>().draft, draft.clone());
        if before != draft {
            // A press on a row can take the focus from the text, which ends the draft: the list stays, filtered by
            // the draft, until the release picks the row.
            let pressing_rows =
                cx.memory::<Typing>().over_rows && cx.response.ctx.input(|input| input.pointer.any_down() || input.pointer.any_released());
            match &draft {
                Some(_) => list.open(cx),
                None if pressing_rows && list.is_open(cx) => cx.memory::<Typing>().draft = before,
                None => list.close(cx),
            }
        }
        // A click on the name while the list is closed opens it on every family and selects the name, so typing
        // replaces it (Photoshop's font field). In Word's plain font picker, clicking selects the name without
        // auto-opening the popup list so the user can immediately type a replacement family. Subsequent clicks on
        // an already-focused plain field place the caret rather than re-selecting all text.
        if !list.is_open(cx) && cx.text(&text_key()).is_some_and(|outcome| outcome.clicked) {
            if self.kind == FontPickerKind::Plain {
                if !was_focused {
                    cx.select_all_text();
                }
            } else {
                cx.select_all_text();
                list.open(cx);
            }
        }
        if list.is_open(cx) {
            return;
        }
        // The one place the picker forgets what belongs to an open list.
        rows::end_preview(self, cx);
        let typing = cx.memory::<Typing>();
        typing.expanded.clear();
        typing.rested.clear();
        // Collect stepping input before building the sorted stop list: idle frames carry neither arrow moves nor
        // wheel ticks, so skipping `stops` avoids collating every installed family on every repaint.
        let mut moves: Vec<bool> = Vec::new();
        while let Some((step, mods)) = cx.intents().take_move() {
            match step {
                NavMove::Down if !mods.fine => moves.push(true),
                NavMove::Up if !mods.fine => moves.push(false),
                _ => {}
            }
        }
        let wheel = cx.take_wheel();
        let wheel_along = wheel.as_ref().and_then(|wheel| {
            if wheel.phase == WheelPhase::End {
                None
            } else if let StepResult::Set(along) =
                wheel_step(&ValueSteps::new(0.0, -f64::MAX, f64::MAX, 1.0, 1.0).fine(1.0), ValueAxes::Both, wheel, cx.env.direction().is_rtl())
                && along != 0.0
            {
                Some((wheel.phase, along))
            } else {
                None
            }
        });
        let wheel_ended = wheel.is_some_and(|wheel| wheel.phase == WheelPhase::End);
        if moves.is_empty() && wheel_along.is_none() {
            if wheel_ended {
                cx.wheel_run(WheelPhase::End, None);
            }
            return;
        }
        let settings = read(&mut self.settings, cx);
        let filters = read(&mut self.filters, cx).unwrap_or_default();
        let all = self.stops(cx, settings.as_ref(), &filters);
        let stops: Vec<usize> = (0..all.len()).collect();
        if !moves.is_empty() {
            let here = match self.current_host.as_deref() {
                Some(key) => all.iter().position(|stop| matches!(stop, Stop::Host(host) if host == key)),
                None => cx.value().and_then(|current| all.iter().position(|stop| matches!(stop, Stop::Family(family) if family == current))),
            };
            let mut to = here;
            for forward in moves {
                to = step_stop(&stops, to, forward);
            }
            if to != here {
                match to.and_then(|at| all.get(at)) {
                    Some(Stop::Host(key)) => self.host_picked = Some(key.clone()),
                    Some(Stop::Family(family)) => {
                        self.picked = Some(family.clone());
                        cx.set(family.clone());
                    }
                    None => {}
                }
            }
        }
        // The wheel steps one stop a line down for a step forward, as one run the wheel's end commits.
        if wheel_ended {
            cx.wheel_run(WheelPhase::End, None);
        } else if let Some((phase, along)) = wheel_along {
            let mut stop = match self.current_host.as_deref() {
                Some(key) => all.iter().position(|stop| matches!(stop, Stop::Host(host) if host == key)),
                None => cx.value().and_then(|current| all.iter().position(|stop| matches!(stop, Stop::Family(family) if family == current))),
            };
            for _ in 0..along.abs().round() as usize {
                stop = step_stop(&stops, stop, along < 0.0);
            }
            match stop.and_then(|at| all.get(at)) {
                Some(Stop::Host(key)) => self.host_picked = Some(key.clone()),
                Some(Stop::Family(family)) => cx.wheel_run(phase, Some(family.clone())),
                None => {}
            }
        }
    }

    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        if self.catalog.is_ready() {
            rows::popup(self, ui, cx);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let name = cx.words::<FontPickerWords>().list.clone();
        let family = cx.value().cloned().unwrap_or_default();
        let missing = self.missing_note(&family, &cx.words::<MissingFontsWords>()).is_some();
        let trailing = Self::trailing(name.clone(), missing, cx.env.get());
        let placed = place(cx.content_rect, &cx.style.field, &[], &trailing, cx.env.direction().is_rtl());
        if let (true, Some(at)) = (missing, placed.trailing.first()) {
            let color = cx.resolve(&cx.style.warning);
            ui.draw().icon(*at, &IconSpec::svg(IconId::new(WARNING_ICON)).color(color));
        }
        ui.face(CHEVRON, IconButton::new(DisclosureGlyph::dropdown_of(cx.env.get())).label(name));
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> FontPickerOutput {
        let edit = cx.edit_response();
        let committed = edit.events.iter().rev().find_map(|event| match event {
            EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => Some(after.clone()),
            _ => None,
        });
        let host_picked = self.host_picked;
        // A host row is no family: it writes no binding and is not a recent family.
        let picked = if host_picked.is_some() { None } else { committed.or(self.picked) };
        let mut bind = self.settings;
        if let Some(family) = &picked
            && let Some(mut settings) = read(&mut bind, &mut cx)
        {
            settings.note_recent(family);
            write(&mut bind, &mut cx, settings, "Recent font");
        }
        FontPickerOutput {
            edit,
            picked,
            preview: self.preview,
            picked_style: self.picked_style,
            rested: self.rested,
            action: self.action,
            host_picked,
        }
    }
}
