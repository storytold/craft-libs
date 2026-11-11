//! [`FontPicker`]: the font family field of the type tool, the Character panel, the style panels and the Glyphs
//! panel. A text field that filters the installed families as the user types, with a list under it of family rows
//! drawn in their own face, favourites (stars), recent families at the top, class and feature filters above the
//! list and a "similar fonts" filter. Behaviour follows Photoshop's font menu.

mod rank;
mod rows;
#[cfg(feature = "screen-testing")]
mod stories;

use std::sync::Arc;

use craft_behavior::{Codec, EditEvent, IconId, Mixed, NotAValue, PartKey, Sense, Value};
use craft_behavior::{HitLevel, HitPriority, NavMove};
use craft_font_catalog::Catalog;
use craft_host::Section;
use craft_ui::buttons::IconButton;
use craft_ui::contract::{
    Bind, Common, Component, Cx, Edit as _, EditResponse, Focus as _, Input, MeasureCx, Outcome, OverlayUi, Overlays as _, PaintUi, PartsCx,
    Pointer as _, Props as _, Remember as _, Role, WordSetCx as _,
};
use craft_ui::list::{ThumbSize, TrailingPart};
use craft_ui::primitives::{IconSpec, TextKey, TextOptions, TextRun};
use craft_ui::runtime::env::{EnvView, Label};
use craft_ui::runtime::memory::StageAccess;
use craft_ui::text::{Adornment, place, reserve};
use egui::{Rect, Response, Vec2};
use serde::{Deserialize, Serialize};

use rank::{Info, infos, shown_name};

craft_ui::words! {
    /// The picker's words, English by default.
    #[non_exhaustive]
    pub struct FontPickerWords {
        search: "fontPicker.search" = "Search fonts";
        recent: "fontPicker.recent" = "Recent";
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
    /// The family has a variable face.
    pub variable: bool,
    /// The row is in the Recent group at the top.
    pub recent: bool,
}

/// The families `listing` would show, in list order: recents first (when the query is empty and no filter is on),
/// then the matching families, the best match first and ties in the order of `language` (its code, `ja`, `pt-BR`).
/// For apps that draw their own list from the picker's filtering.
pub fn family_rows(
    catalog: &Arc<Catalog>,
    query: &str,
    settings: Option<&FontPickerSettings>,
    filters: &FontFilters,
    current: &str,
    language: &str,
) -> Vec<FamilyRow> {
    let (folder, collator) = (craft_i18n::Folder::new(language), craft_i18n::Collator::new(language));
    let listing = rank::Listing { query, settings, filters, current, language };
    rank::list(catalog, &infos(catalog), &listing, &rank::Language { folder: &folder, collator: &collator }, true).0
}

/// What a [`FontPicker`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct FontPickerOutput {
    /// The field's edit: a pick commits the family; a hovered or keyed row is a running proposal.
    pub edit: EditResponse<String>,
    /// A family was picked: Enter or a click on a row, Enter on the exact typed name, or the wheel or arrows on the
    /// closed field. The picker has already noted it as recent.
    pub picked: Option<String>,
    /// The family under the pointer or the keyboard row while the list is open, for a live preview of the selected
    /// text (Photoshop previews on hover). `None` restores the committed family.
    pub preview: Option<String>,
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
    infos: Arc<rank::Infos>,
    language: String,
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

    /// A family whose name, shown or in the catalog, is the text apart from case.
    fn parse(&self, text: &str, _: &String) -> Result<String, NotAValue> {
        let wanted = text.trim().to_lowercase();
        let named = |(family, info): (&String, &Info)| {
            (family.to_lowercase() == wanted || self.shown(family, info).to_lowercase() == wanted).then(|| family.clone())
        };
        self.infos.families.iter().find_map(named).ok_or(NotAValue)
    }

    fn sample(&self, current: &String) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Owned(format!("{}{}", NAME_SAMPLE, self.format(current)))
    }
}

/// What the picker keeps between passes: the draft the list was last filtered for, the family the list previews
/// and whether the class column is hidden.
#[derive(Clone, Default)]
struct Typing {
    draft: Option<String>,
    lead: Option<String>,
    hide_classes: bool,
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
/// match first, and opens it; the chevron and Alt+Down open it on every family, Alt+Up closes it. Rows: a star, the
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
}

impl<'a> FontPicker<'a> {
    /// A picker named `label` over the family `family` of `catalog`.
    pub fn new(label: impl Into<Label>, family: Bind<'a, String>, catalog: &'a Arc<Catalog>) -> Self {
        FontPicker { common: Common::new(), family, catalog, settings: None, filters: None, picked: None }.label(label)
    }

    fn trailing(name: Label) -> [Adornment; 1] {
        [Adornment::button(CHEVRON, IconSpec::svg(IconId::new(CHEVRON_ICON)), name)]
    }

    fn codec(&self, env: &EnvView<'_>) -> FamilyCodec {
        FamilyCodec { infos: infos(self.catalog), language: ui_language(env) }
    }

    /// The families in list order without filters or recents, which the arrows and the wheel step through.
    fn ordered(&self, cx: &Cx<'_, Self>, settings: Option<&FontPickerSettings>) -> Vec<FamilyRow> {
        let infos = infos(self.catalog);
        let (folder, collator) = (cx.env.folder(), cx.env.collator());
        let filters = FontFilters::default();
        let language = ui_language(&cx.env);
        let listing = rank::Listing { query: "", settings, filters: &filters, current: "", language: &language };
        rank::list(self.catalog, &infos, &listing, &rank::Language { folder: &folder, collator: &collator }, false).0
    }
}

impl Component for FontPicker<'_> {
    const ROLE: Role = Role::COMBO_BOX;
    const INPUT: Input = Input::Text;
    const KEYS: craft_ui::runtime::focus::KeyUse = craft_ui::runtime::focus::KeyUse::of(Input::Text).with_moves();
    type Style = craft_ui::text::FieldStyle;
    type Output = FontPickerOutput;

    fn style(cx: &craft_ui::style::StyleCx<'_>, _: FontPickerStyleKey) -> craft_ui::text::FieldStyle {
        craft_ui::text::FieldStyle::at(cx)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    /// The width of the longest family names in common use, so the field keeps its size whichever family is picked.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let current = cx.value().map_or("", String::as_str);
        let widest = [NAME_SAMPLE, current].into_iter().map(|text| cx.layout(&TextRun::new(text)).size()).fold(Vec2::ZERO, Vec2::max);
        let name = cx.words::<FontPickerWords>().list.clone();
        Vec2::new(widest.x + reserve(cx.style, &[], &Self::trailing(name)), widest.y)
    }

    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let name = cx.words::<FontPickerWords>().list.clone();
        let trailing = Self::trailing(name);
        let placed = place(cx.content_rect, cx.style, &[], &trailing, cx.flow.is_rtl());
        if let Some(at) = placed.trailing.first() {
            cx.part(CHEVRON, *at)
                .sense(Sense::CLICK | Sense::KEEP_FOCUS)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .pointer_only("Alt+Down opens the list")
                .toggles(POPUP)
                .on_press();
        }
        let codec = self.codec(&cx.env);
        let family = cx.value().cloned().unwrap_or_default();
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
        if list.is_open(cx) {
            return;
        }
        cx.memory::<Typing>().lead = None;
        if matches!(cx.gesture(), Some(craft_behavior::GesturePhase::Begin | craft_behavior::GesturePhase::Update)) {
            cx.cancel(craft_behavior::CancelReason::Dismissed);
        }
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
        ui.face(CHEVRON, IconButton::new(IconId::new(CHEVRON_ICON)).label(name));
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> FontPickerOutput {
        let list = cx.popup(POPUP);
        let preview = if list.is_open(&mut cx) { cx.memory::<Typing>().lead.clone() } else { None };
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
        FontPickerOutput { edit, picked, preview }
    }
}
