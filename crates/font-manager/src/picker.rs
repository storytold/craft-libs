//! [`FontPicker`]: the font family field of the type tool, the Character panel, the style panels and the Glyphs
//! panel. A text field that filters the installed families as the user types, with a list under it of family rows
//! drawn in their own face, favourites (stars), recent families at the top, class and feature filters above the
//! list and a "similar fonts" filter. Behaviour follows Photoshop's font menu.

pub(crate) mod rank;
mod rows;
#[cfg(feature = "suite")]
pub(crate) mod stories;

use std::sync::Arc;

use craft_behavior::{Codec, EditEvent, IconId, Mixed, NotAValue, PartKey, Sense, Value};
use craft_behavior::{HitLevel, HitPriority, NavMove, Preview};
use craft_font_catalog::Catalog;
use craft_host::Section;
use craft_theme::variant::Tone;
use craft_ui::buttons::{ControlPlace, IconButton};
use craft_ui::contract::{
    Bind, Common, Component, Cx, Edit as _, EditResponse, Focus as _, Input, MeasureCx, Outcome, OverlayUi, Overlays as _, PaintUi, PartsCx,
    Pointer as _, Props as _, Remember as _, Role, Styled as _, WordSetCx as _,
};
use craft_ui::list::{ThumbSize, TrailingPart};
use craft_ui::primitives::{IconSpec, TextKey, TextOptions, TextRun};
use craft_ui::runtime::env::{EnvView, Label};
use craft_ui::runtime::memory::StageAccess;
use craft_ui::style::{ComponentStyle, Metrics, Stateful, States, StyleColor, StyleCx, SurfaceLook};
use craft_ui::text::{Adornment, FieldStyle, place, reserve};
use egui::{Rect, Response, Vec2};
use serde::{Deserialize, Serialize};

use crate::missing::{DocumentFont, FontStatus, MissingFontsWords, document_fonts};
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
        /// The short mark at the end of a row for a class.
        mark_serif: "fontPicker.markSerif" = "Se";
        mark_slab_serif: "fontPicker.markSlabSerif" = "Sl";
        mark_sans_serif: "fontPicker.markSansSerif" = "Sa";
        mark_script: "fontPicker.markScript" = "Sc";
        mark_monospace: "fontPicker.markMonospace" = "Mo";
        mark_handwritten: "fontPicker.markHandwritten" = "Ha";
        mark_blackletter: "fontPicker.markBlackletter" = "Bl";
        mark_decorative: "fontPicker.markDecorative" = "De";
        show_styles: "fontPicker.showStyles" = "Show styles";
        hide_styles: "fontPicker.hideStyles" = "Hide styles";
        sample_arabic: "fontPicker.sampleArabic" = "أبجد";
        format_open_type: "fontPicker.formatOpenType" = "OpenType";
        format_true_type: "fontPicker.formatTrueType" = "TrueType";
        format_variable: "fontPicker.formatVariable" = "Variable";
        served: "fontPicker.served" = "Not installed";
    }
}

/// Photoshop's font classes, in its filter menu order (the class glyph at the row's right).
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
    /// Families like the current one, most alike first ([`SIMILAR_COUNT`] of them).
    pub similar: bool,
}

impl FontFilters {
    /// Whether any filter narrows the list (Clear All is enabled).
    pub fn any(&self) -> bool {
        self.class.is_some() || self.favorites || self.variable || self.similar
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
}

impl Default for FontPickerSettings {
    fn default() -> Self {
        FontPickerSettings {
            favorites: Vec::new(),
            recent: Vec::new(),
            recent_limit: 10,
            preview: FontPreviewSize::default(),
            names_in_english: false,
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
    /// The family's class once it has been read: by a class filter, or by the list drawing the row.
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
    current: &str,
    language: &str,
) -> Vec<FamilyRow> {
    let (folder, collator) = (craft_i18n::Folder::new(language), craft_i18n::Collator::new(language));
    let listing = rank::Listing { query, settings, filters, current, language, served: &[] };
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
const CHEVRON_ICON: &str = "chevron_down";
/// The mark before the chevron of a shown font that is not installed; its tooltip is the note.
const WARNING: PartKey = PartKey::new("missing");
const WARNING_ICON: &str = "warning";
/// The popup the chevron and Alt+Down toggle.
const POPUP: &str = "list";
/// The star at the end of a row.
const STAR: TrailingPart = TrailingPart::new(0);
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
}

impl FamilyCodec {
    fn shown(&self, family: &str, info: &Info) -> String {
        shown_name(info, family, &self.language, false)
    }
}

impl Codec for FamilyCodec {
    type Value = String;

    fn format(&self, family: &String) -> String {
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

/// What the picker keeps between passes: the draft the list was last filtered for, the family the list previews,
/// whether the class column is hidden, and, for the open list only, the families whose styles show and the families
/// already reported as rested.
#[derive(Clone, Default)]
struct Typing {
    draft: Option<String>,
    lead: Option<String>,
    /// The lead's family was proposed by the pointer, so it ends when the pointer leaves the rows.
    lead_by_pointer: bool,
    hide_classes: bool,
    expanded: Vec<String>,
    rested: Vec<String>,
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

/// The font family field. Typing filters the list to the families whose name (in any language) matches, the best
/// match first, and opens it; a click on the name while the list is closed, the chevron and Alt+Down open it on every family (the click selects the name), Alt+Up closes it. Rows: a star, the
/// family name in the interface face, a sample in the family's own face at the preview size and the class mark.
/// Recent families form a group at the top. Up, Down and Page keys move the keyboard row, which previews, Enter or a
/// click on a row picks it, Enter on a typed name that is a family picks that family, Escape closes the list and
/// restores the field, and Tab picks the keyboard row and moves on. While an input method composes, Enter, Escape
/// and the arrows are the composition's. On the closed, focused field the arrows and the wheel step to the previous
/// or next family in list order and pick it. The list is the field's own popup: filter toggles above, the class
/// column at the reading start and the families beside it.
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
    /// The preview banner's text.
    #[prop]
    preview_text: Option<&'a str>,
    /// Families the host can fetch but that are not installed, listed with the served mark.
    #[prop]
    served: &'a [String],
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
            kind: FontPickerKind::default(),
            actions: &[],
            shown_as: None,
            advice: MissingAdvice::default(),
            action: None,
        }
        .label(label)
    }

    /// The adornments at the end of the field: the missing mark when `missing`, then the chevron. The mark is a glyph
    /// whose part carries the note.
    fn trailing(name: Label, missing: bool) -> Vec<Adornment> {
        let mut all = Vec::with_capacity(2);
        if missing {
            all.push(Adornment::glyph(IconSpec::svg(IconId::new(WARNING_ICON))));
        }
        all.push(Adornment::button(CHEVRON, IconSpec::svg(IconId::new(CHEVRON_ICON)), name));
        all
    }

    /// The note for `family` when it is shown in another face because the catalog lacks it, by the rule the Missing
    /// Fonts dialog uses: the missing-font sentence, then the advice the host chose.
    fn missing_note(&self, family: &str, words: &MissingFontsWords) -> Option<Label> {
        let face = self.shown_as?;
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
        FamilyCodec { infos, language: ui_language(env), free: self.kind == FontPickerKind::Plain }
    }

    /// The families in list order without filters or recents, which the arrows and the wheel step through.
    fn ordered(&self, cx: &Cx<'_, Self>, settings: Option<&FontPickerSettings>) -> Vec<FamilyRow> {
        let infos = infos_of(&cx.response.ctx, cx.id, self.catalog);
        let (folder, collator) = (cx.env.folder(), cx.env.collator());
        let filters = FontFilters::default();
        let language = ui_language(&cx.env);
        let listing = rank::Listing { query: "", settings, filters: &filters, current: "", language: &language, served: &[] };
        rank::list(self.catalog, &infos, &listing, &rank::Language { folder: &folder, collator: &collator }, false).0
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
}

impl Component for FontPicker<'_> {
    const ROLE: Role = Role::COMBO_BOX;
    const INPUT: Input = Input::Text;
    const KEYS: craft_ui::runtime::focus::KeyUse = craft_ui::runtime::focus::KeyUse::of(Input::Text).with_moves();
    type Style = FontPickerStyle;
    type Output = FontPickerOutput;

    /// A field; in a ribbon (`ControlPlace::Ribbon`) held at the small field length (`size.length.field.sm`), as a
    /// ribbon's font box is: wide enough for a family name and its menu arrow, the same width whichever it shows.
    fn style(cx: &StyleCx<'_>, _: FontPickerStyleKey) -> FontPickerStyle {
        let field = FieldStyle::at(cx);
        let field = if cx.env::<ControlPlace>() == ControlPlace::Ribbon { field.held_at(cx.tokens.size.length.field.sm) } else { field };
        let warn = cx.tone(Tone::Warn);
        let off = cx.color(cx.tokens.color.text.disabled);
        FontPickerStyle { surface: cx.field(), metrics: field.metrics(), field, warning: Stateful::new(warn).off(States::new(warn, warn, warn, off)) }
    }

    /// The wheel steps the family through the list.
    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS | Sense::WHEEL_Y
    }

    /// The width of the longest family names in common use, so the field keeps its size whichever family is picked.
    /// In a ribbon the name field is the short field length (`size.length.short`) wide whichever family it shows, a
    /// longer name scrolling in it as in a word processor's font box: that is its least, about the old ribbon's font
    /// name, so a row short of room never narrows the box to its menu arrow; where the room allows the style holds
    /// the box at the small field length (`FieldStyle::held_at`).
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let current = cx.value().map_or("", String::as_str);
        let widest = [NAME_SAMPLE, current].into_iter().map(|text| cx.field_text(&TextRun::new(text))).fold(Vec2::ZERO, Vec2::max);
        let ribbon = cx.env.get::<ControlPlace>() == ControlPlace::Ribbon;
        let widest = if ribbon { Vec2::new(cx.tokens.size.length.short, widest.y) } else { widest };
        let name = cx.words::<FontPickerWords>().list.clone();
        let note = self.missing_note(current, &cx.words::<MissingFontsWords>());
        Vec2::new(widest.x + reserve(&cx.style.field, &[], &Self::trailing(name, note.is_some())), widest.y)
    }

    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let name = cx.words::<FontPickerWords>().list.clone();
        let family = cx.value().cloned().unwrap_or_default();
        let note = self.missing_note(&family, &cx.words::<MissingFontsWords>());
        let trailing = Self::trailing(name, note.is_some());
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
        let codec = self.codec(infos_of(cx.ui.ctx(), cx.id, self.catalog), &cx.env);
        let mut options = TextOptions::new();
        options.select_on_focus = true;
        cx.text(text_key(), placed.text, &codec, &family, &options);
    }

    fn bind(&mut self) -> Option<craft_ui::contract::BindMut<'_, String>> {
        Some(self.family.reborrow())
    }

    /// A changed draft filters the list: it opens on the matches, and closes when the draft ended (committed,
    /// cancelled or replaced by a pick). A preview that the list closed on without a pick is cancelled. While the
    /// list is closed, the arrows and the wheel step through the families.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let list = cx.popup(POPUP);
        let draft = cx.text(&text_key()).and_then(|outcome| outcome.draft.clone());
        let before = std::mem::replace(&mut cx.memory::<Typing>().draft, draft.clone());
        if before != draft {
            match &draft {
                Some(_) => list.open(cx),
                None => list.close(cx),
            }
        }
        // A click on the name while the list is closed opens it on every family and selects the name, so typing
        // replaces it (Photoshop's font field).
        if !list.is_open(cx) && cx.text(&text_key()).is_some_and(|outcome| outcome.clicked) {
            cx.select_all_text();
            list.open(cx);
        }
        if list.is_open(cx) {
            return;
        }
        // The one place the picker forgets what belongs to an open list.
        rows::end_preview(self, cx);
        let typing = cx.memory::<Typing>();
        typing.expanded.clear();
        typing.rested.clear();
        let settings = read(&mut self.settings, cx);
        let current = cx.value().cloned().unwrap_or_default();
        let all = self.ordered(cx, settings.as_ref());
        let Some(here) = all.iter().position(|row| row.family == current) else { return };
        let mut to = here;
        while let Some((step, mods)) = cx.intents().take_move() {
            match step {
                NavMove::Down if !mods.fine => to = (to + 1).min(all.len() - 1),
                NavMove::Up if !mods.fine => to = to.saturating_sub(1),
                _ => {}
            }
        }
        if let Some(wheel) = cx.take_wheel()
            && wheel.scroll.y != 0.0
        {
            to = if wheel.scroll.y < 0.0 { (to + 1).min(all.len() - 1) } else { to.saturating_sub(1) };
        }
        if to != here {
            self.picked = Some(all[to].family.clone());
            cx.set(all[to].family.clone());
        }
    }

    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        rows::popup(self, ui, cx);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let name = cx.words::<FontPickerWords>().list.clone();
        let family = cx.value().cloned().unwrap_or_default();
        let missing = self.missing_note(&family, &cx.words::<MissingFontsWords>()).is_some();
        let trailing = Self::trailing(name.clone(), missing);
        let placed = place(cx.content_rect, &cx.style.field, &[], &trailing, cx.env.direction().is_rtl());
        if let (true, Some(at)) = (missing, placed.trailing.first()) {
            let color = cx.resolve(&cx.style.warning);
            ui.draw().icon(*at, &IconSpec::svg(IconId::new(WARNING_ICON)).color(color));
        }
        ui.face(CHEVRON, IconButton::new(IconId::new(CHEVRON_ICON)).label(name));
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> FontPickerOutput {
        let edit = cx.edit_response();
        let committed = edit.events.iter().rev().find_map(|event| match event {
            EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => Some(after.clone()),
            _ => None,
        });
        let picked = committed.or(self.picked);
        let mut bind = self.settings;
        if let Some(family) = &picked
            && let Some(mut settings) = read(&mut bind, &mut cx)
        {
            settings.note_recent(family);
            write(&mut bind, &mut cx, settings, "Recent font");
        }
        FontPickerOutput { edit, picked, preview: self.preview, picked_style: self.picked_style, rested: self.rested, action: self.action }
    }
}
