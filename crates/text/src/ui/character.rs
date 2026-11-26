//! [`CharacterControls`] and [`ParagraphControls`]: Photoshop's Character and Paragraph
//! panels as patterns for a panel body, a Properties section or a style options editor. Values
//! are in points (sizes, leading, baseline shift, indents, spacing) and thousandths of an em
//! (kerning, tracking); the length fields show them in the app's type unit ([`DocUnit`]). The
//! controls change nothing themselves: each returns the edit events of the changes the user made
//! this pass.

#[cfg(feature = "font-picker")]
use craft_font_manager::Catalog;
#[cfg(feature = "font-picker")]
use craft_font_manager::axes::{AxesButton, AxisSliders};
#[cfg(feature = "font-picker")]
use craft_font_manager::picker::{FamilySet, FilterLayout, FontFilters, FontPickerSettings, FontScript, HostFontGroup};
use std::ops::RangeInclusive;
#[cfg(feature = "font-picker")]
use std::sync::Arc;

use crate::ui::length::DocUnit;
use crate::ui::type_options::{TextAntiAlias, TypeOptionWords};
use craft_behavior::{ColumnWidth, EditEvent, IconId, Preview};
use craft_theme::{Rgba, Srgba};
use craft_ui::choice::Swatch;
#[cfg(feature = "font-picker")]
use craft_ui::contract::Bind;
use craft_ui::contract::{Entry, FieldLayout, GlyphLabel, LabelKind, NoValue, Outcome, Props as _, Show as _, ShowContainer as _, SideLabel, Sizing};
use craft_ui::form::Field;
use craft_ui::layout_components::{Grid, Stack};
use craft_ui::patterns::form_dialogs::HostRequests;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::text::{NamedOr, NumberName, NumberPick, NumberValue as _};
use egui::{Response, Ui};

pub(crate) mod format_text;
pub(crate) mod paragraph;
pub(crate) mod paragraph_dialog;
pub(crate) mod rows;
#[cfg(feature = "suite")]
pub(crate) mod stories;
pub(crate) mod style_options;

pub use format_text::{FormatControl, FormatText, FormatTextChange, FormatTextOutput, FormatTextWords, ListKind};
pub use paragraph::{
    DirectionRow, ParagraphChange, ParagraphControls, ParagraphControlsOutput, ParagraphDirection, ParagraphValues, ParagraphWords, TextAlign,
};
pub use paragraph_dialog::{
    AsianTypography, LineBreakOptions, LineSpacing, ParagraphAlign, ParagraphAnswer, ParagraphDialog, ParagraphDialogWords, ParagraphFormat,
    ParagraphPreview, ParagraphPreviewStyle, ParagraphTab, SpacingKind, SpecialIndent,
};
use rows::{
    LengthRow, Lengths, PlainRow, Toggle, agree, family_field_keyed, font_height_entries, hidden_side, leading_menu, leading_row_keyed, mark, mixed,
    number_change, number_presets, plain, select_row, size_menu, string_entries, toggle_row,
};
pub use style_options::{
    STYLE_COLOR_KEY, StyleAttribute, StyleOptionValues, StyleOptionsChange, TextStyleOptions, TextStyleOptionsOutput, TextStyleOptionsWords,
};

craft_ui::words! {
    /// The Character controls' words, English by default (PhotoCraft's tooltips). A control whose label is an icon or a
    /// mark shows only that; the word is its accessible name.
    #[non_exhaustive]
    pub struct CharacterWords {
        character: "character.title" = "Character";
        family: "character.family" = "Font family";
        style: "character.style" = "Style";
        size: "character.size" = "Font size";
        leading: "character.leading" = "Leading";
        auto: "character.auto" = "Auto";
        kerning: "character.kerning" = "Kerning";
        metrics: "character.metrics" = "Metrics";
        optical: "character.optical" = "Optical";
        tracking: "character.tracking" = "Tracking";
        leading_hint: "character.leadingHint" = "Leading (set to the font size × auto-leading when Auto)";
        kerning_hint: "character.kerningHint" = "Kerning: Metrics, Optical or a value in 1/1000 em (Alt+←/→ at the caret)";
        tracking_hint: "character.trackingHint" = "Tracking (1/1000 em)";
        vertical_scale: "character.verticalScale" = "Vertical scale";
        horizontal_scale: "character.horizontalScale" = "Horizontal scale";
        baseline_shift: "character.baselineShift" = "Baseline shift";
        color: "character.color" = "Color";
        text_color: "character.textColor" = "Text color";
        faux_bold: "character.fauxBold" = "Faux Bold";
        faux_italic: "character.fauxItalic" = "Faux Italic";
        axes: "character.axes" = "Variable font axes";
        all_caps: "character.allCaps" = "All Caps";
        small_caps: "character.smallCaps" = "Small Caps";
        superscript: "character.superscript" = "Superscript";
        subscript: "character.subscript" = "Subscript";
        underline: "character.underline" = "Underline";
        strikethrough: "character.strikethrough" = "Strikethrough";
        standard_ligatures: "character.standardLigatures" = "Standard Ligatures";
        contextual_alternates: "character.contextualAlternates" = "Contextual Alternates";
        discretionary_ligatures: "character.discretionaryLigatures" = "Discretionary Ligatures";
        swash: "character.swash" = "Swash";
        stylistic_alternates: "character.stylisticAlternates" = "Stylistic Alternates";
        titling_alternates: "character.titlingAlternates" = "Titling Alternates";
        ordinals: "character.ordinals" = "Ordinals";
        fractions: "character.fractions" = "Fractions";
        language: "character.language" = "Language";
        rotation: "character.rotation" = "Rotation";
        anti_alias: "character.antiAlias" = "Anti-aliasing";
        font_height: "character.fontHeight" = "Set Font Height Reference";
        em_box: "character.emBox" = "Em Box";
        cap_height: "character.capHeight" = "Cap Height";
        x_height: "character.xHeight" = "x-Height";
        icf_box: "character.icfBox" = "ICF Box";
        aki_left: "character.akiLeft" = "Insert Aki Left";
        aki_right: "character.akiRight" = "Insert Aki Right";
        tsume: "character.tsume" = "Tsume";
        diacritic_x: "character.diacriticX" = "Horizontal Diacritic Position";
        diacritic_y: "character.diacriticY" = "Vertical Diacritic Position";
        variable_font: "character.variableFont" = "Variable Font";
    }
}

/// The document keys of the family, size, leading, kerning and tracking fields, which a host focuses with
/// `runtime::focus::request(ctx, FocusTarget::Doc(keys::SIZE))`.
pub mod keys {
    use craft_behavior::DocKey;

    /// The family field.
    pub const FAMILY: DocKey = DocKey::new("craft-text.character.family");
    /// The size field.
    pub const SIZE: DocKey = DocKey::new("craft-text.character.size");
    /// The leading field.
    pub const LEADING: DocKey = DocKey::new("craft-text.character.leading");
    /// The kerning field.
    pub const KERNING: DocKey = DocKey::new("craft-text.character.kerning");
    /// The tracking field.
    pub const TRACKING: DocKey = DocKey::new("craft-text.character.tracking");
}

/// The groups of rows the Character controls show, as named sets the host turns on or off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CharacterRows(u16);

impl CharacterRows {
    /// The colour swatch beside the baseline shift.
    pub const COLOR: CharacterRows = CharacterRows(1 << 0);
    /// Faux Bold and Faux Italic.
    pub const FAUX: CharacterRows = CharacterRows(1 << 1);
    /// Vertical and horizontal scale.
    pub const SCALES: CharacterRows = CharacterRows(1 << 2);
    /// Rotation.
    pub const ROTATION: CharacterRows = CharacterRows(1 << 3);
    /// Anti-aliasing.
    pub const ANTI_ALIAS: CharacterRows = CharacterRows(1 << 4);
    /// Set Font Height Reference.
    pub const FONT_HEIGHT: CharacterRows = CharacterRows(1 << 5);
    /// Superscript and Subscript.
    pub const SUPERSCRIPT: CharacterRows = CharacterRows(1 << 6);
    /// The OpenType buttons.
    pub const OPENTYPE: CharacterRows = CharacterRows(1 << 7);
    /// Insert Aki Left and Right, Tsume.
    pub const EAST_ASIAN: CharacterRows = CharacterRows(1 << 8);
    /// Horizontal and vertical diacritic positions.
    pub const MIDDLE_EASTERN: CharacterRows = CharacterRows(1 << 9);
    /// Baseline shift.
    pub const BASELINE: CharacterRows = CharacterRows(1 << 10);
    /// The style toggles: All Caps, Small Caps, Underline and Strikethrough, with Faux Bold and Italic under
    /// [`FAUX`](Self::FAUX) and Superscript and Subscript under [`SUPERSCRIPT`](Self::SUPERSCRIPT).
    pub const STYLE_TOGGLES: CharacterRows = CharacterRows(1 << 11);
    /// The Properties panel's Character section: family, style, the axes, size, leading, kerning and tracking, with
    /// no other group on.
    pub const PROPERTIES: CharacterRows = CharacterRows(1 << 12);
    /// The set `show` draws by default.
    pub const PHOTOSHOP: CharacterRows =
        CharacterRows(Self::COLOR.0 | Self::FAUX.0 | Self::SCALES.0 | Self::BASELINE.0 | Self::STYLE_TOGGLES.0 | Self::OPENTYPE.0);

    /// Whether every group in `rows` is on.
    pub const fn contains(self, rows: CharacterRows) -> bool {
        self.0 & rows.0 == rows.0
    }

    /// The set with every group in `rows` turned on or off.
    #[must_use]
    pub const fn with(self, rows: CharacterRows, on: bool) -> Self {
        CharacterRows(if on { self.0 | rows.0 } else { self.0 & !rows.0 })
    }
}

impl std::ops::BitOr for CharacterRows {
    type Output = CharacterRows;

    fn bitor(self, other: CharacterRows) -> CharacterRows {
        CharacterRows(self.0 | other.0)
    }
}

impl Default for CharacterRows {
    fn default() -> Self {
        CharacterRows::PHOTOSHOP
    }
}

/// The Set Font Height Reference values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, craft_ui::contract::MemoProps)]
#[non_exhaustive]
pub enum FontHeightReference {
    /// The em box.
    #[default]
    EmBox,
    /// The cap height.
    CapHeight,
    /// The x-height.
    XHeight,
    /// The ideographic character face box.
    IcfBox,
}

/// The font size menu's presets, in points (PhotoCraft's `SIZE_PRESETS`, Photoshop's list); the
/// leading menu uses them too.
pub const SIZE_PRESETS: [f32; 17] = [6.0, 8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 24.0, 30.0, 36.0, 48.0, 60.0, 72.0, 150.0, 300.0];

/// The tracking menu's presets in thousandths of an em, in groups split by separators
/// (PhotoCraft's `TRACKING_PRESETS`).
pub const TRACKING_PRESETS: [&[f32]; 3] = [&[-100.0, -75.0, -50.0, -25.0, -10.0, -5.0], &[0.0], &[5.0, 10.0, 25.0, 50.0, 75.0, 100.0, 200.0]];

/// The OpenType buttons' feature tags in Photoshop's order (PhotoCraft's `OPENTYPE_BUTTONS`);
/// their names are the matching [`CharacterWords`] fields.
pub const OPENTYPE_FEATURES: [&str; 8] = ["liga", "calt", "dlig", "swsh", "salt", "titl", "ordn", "frac"];

/// The key of the colour request the swatch makes.
pub const COLOR_KEY: &str = "character.color";

/// Auto leading is the font size × 120 % (Photoshop's default).
const AUTO_LEADING: f32 = 1.2;

/// The kerning menu's number rows.
const KERNING_PRESETS: [f32; 14] = [0.0, -100.0, -75.0, -50.0, -25.0, -10.0, -5.0, 5.0, 10.0, 25.0, 50.0, 75.0, 100.0, 200.0];

/// The most decimals the menus' number rows show.
const PRESET_DECIMALS: usize = 2;

/// The leading menu's Auto row.
const AUTO_ROW: u32 = 0;
/// The kerning menu's Metrics row.
const METRICS_ROW: u32 = 0;
/// The kerning menu's Optical row.
const OPTICAL_ROW: u32 = 1;

/// The range of the size field, points.
pub(crate) const SIZE_RANGE: RangeInclusive<f64> = 0.1..=1296.0;
/// The range of the leading field, points.
const LEADING_RANGE: RangeInclusive<f64> = 0.1..=5000.0;
/// The range of the baseline shift field, points.
const SHIFT_RANGE: RangeInclusive<f64> = -1296.0..=1296.0;
/// The range of the tracking and kerning fields, thousandths of an em.
const TRACKING_RANGE: RangeInclusive<f64> = -1000.0..=10000.0;
/// The range of the scale fields, percent.
const SCALE_RANGE: RangeInclusive<f64> = 0.0..=1000.0;
/// The rotation presets, degrees.
pub const ROTATION_PRESETS: [f32; 9] = [-180.0, -135.0, -90.0, -45.0, 0.0, 45.0, 90.0, 135.0, 180.0];
/// The Insert Aki Left and Right presets, thousandths of an em.
pub const AKI_PRESETS: [f32; 4] = [0.0, 250.0, 500.0, 1000.0];
/// The Tsume presets, percent.
pub const TSUME_PRESETS: [f32; 5] = [0.0, 25.0, 50.0, 75.0, 100.0];
/// The diacritic position presets, thousandths of an em.
pub const DIACRITIC_PRESETS: [f32; 7] = [-200.0, -100.0, -50.0, 0.0, 50.0, 100.0, 200.0];
/// The range of the rotation field, degrees.
const ROTATION_RANGE: RangeInclusive<f64> = -360.0..=360.0;
/// The range of the Insert Aki fields, thousandths of an em.
const AKI_RANGE: RangeInclusive<f64> = 0.0..=1000.0;
/// The range of the Tsume field, percent.
const TSUME_RANGE: RangeInclusive<f64> = 0.0..=100.0;
/// The range of the diacritic position fields, thousandths of an em.
const DIACRITIC_RANGE: RangeInclusive<f64> = -1000.0..=1000.0;
/// Two columns of the same share, for the rows of two fields.
const TWO_COLUMNS: [ColumnWidth; 2] = [ColumnWidth::fill(1.0), ColumnWidth::fill(1.0)];

/// A field of the rows' flow, in the order the rows are drawn. Cells flow two to a row; a full cell takes a row of its
/// own and ends the run of cells before it.
#[derive(Clone, Copy)]
enum Cell {
    Size,
    Leading,
    FontHeight,
    Kerning,
    Tracking,
    VerticalScale,
    HorizontalScale,
    BaselineShift,
    Color,
    Rotation,
    AntiAlias,
    AkiLeft,
    AkiRight,
    Tsume,
    DiacriticX,
    DiacriticY,
}

/// A number field that is neither a length nor named: its mark, value, range and the change it makes.
struct PlainCell {
    label: Label,
    mark: &'static str,
    value: Option<f32>,
    range: RangeInclusive<f64>,
    percent: bool,
    presets: &'static [f32],
    change: fn(f32) -> CharacterChange,
}

impl Cell {
    /// Every cell in drawing order.
    const ALL: [Cell; 16] = [
        Cell::Size,
        Cell::Leading,
        Cell::FontHeight,
        Cell::Kerning,
        Cell::Tracking,
        Cell::VerticalScale,
        Cell::HorizontalScale,
        Cell::BaselineShift,
        Cell::Color,
        Cell::Rotation,
        Cell::AntiAlias,
        Cell::AkiLeft,
        Cell::AkiRight,
        Cell::Tsume,
        Cell::DiacriticX,
        Cell::DiacriticY,
    ];

    /// Whether the cell takes a row of its own.
    fn full(self) -> bool {
        matches!(self, Cell::FontHeight | Cell::AntiAlias)
    }

    /// Whether the groups `rows` turns on include the cell.
    fn shown(self, rows: CharacterRows) -> bool {
        match self {
            Cell::Size | Cell::Leading | Cell::Kerning | Cell::Tracking => true,
            Cell::FontHeight => rows.contains(CharacterRows::FONT_HEIGHT),
            Cell::VerticalScale | Cell::HorizontalScale => rows.contains(CharacterRows::SCALES),
            Cell::BaselineShift => rows.contains(CharacterRows::BASELINE),
            Cell::Color => rows.contains(CharacterRows::COLOR),
            Cell::Rotation => rows.contains(CharacterRows::ROTATION),
            Cell::AntiAlias => rows.contains(CharacterRows::ANTI_ALIAS),
            Cell::AkiLeft | Cell::AkiRight | Cell::Tsume => rows.contains(CharacterRows::EAST_ASIAN),
            Cell::DiacriticX | Cell::DiacriticY => rows.contains(CharacterRows::MIDDLE_EASTERN),
        }
    }

    /// The field of a cell that is a plain number; `None` for the others.
    fn plain(self, words: &CharacterWords, values: &CharacterValues) -> Option<PlainCell> {
        let cell =
            |label: &Label,
             mark: &'static str,
             value: Option<f32>,
             range: RangeInclusive<f64>,
             percent: bool,
             presets: &'static [f32],
             change: fn(f32) -> CharacterChange| PlainCell { label: label.clone(), mark, value, range, percent, presets, change };
        Some(match self {
            Cell::VerticalScale => cell(&words.vertical_scale, "↕T", values.vertical_scale, SCALE_RANGE, true, &[], CharacterChange::VerticalScale),
            Cell::HorizontalScale => {
                cell(&words.horizontal_scale, "↔T", values.horizontal_scale, SCALE_RANGE, true, &[], CharacterChange::HorizontalScale)
            }
            Cell::Rotation => cell(&words.rotation, "↻", values.rotation, ROTATION_RANGE, false, &ROTATION_PRESETS, CharacterChange::Rotation),
            Cell::AkiLeft => cell(&words.aki_left, "|A", values.aki_left, AKI_RANGE, false, &AKI_PRESETS, CharacterChange::AkiLeft),
            Cell::AkiRight => cell(&words.aki_right, "A|", values.aki_right, AKI_RANGE, false, &AKI_PRESETS, CharacterChange::AkiRight),
            Cell::Tsume => cell(&words.tsume, "|A|", values.tsume, TSUME_RANGE, true, &TSUME_PRESETS, CharacterChange::Tsume),
            Cell::DiacriticX => {
                cell(&words.diacritic_x, "Á↔", values.diacritic_x, DIACRITIC_RANGE, false, &DIACRITIC_PRESETS, CharacterChange::DiacriticX)
            }
            Cell::DiacriticY => {
                cell(&words.diacritic_y, "Á↕", values.diacritic_y, DIACRITIC_RANGE, false, &DIACRITIC_PRESETS, CharacterChange::DiacriticY)
            }
            Cell::Size | Cell::Leading | Cell::FontHeight | Cell::Kerning | Cell::Tracking | Cell::BaselineShift | Cell::Color | Cell::AntiAlias => {
                return None;
            }
        })
    }
}

/// The variable-font axes of the current face: what the axis sliders under the style row show.
#[cfg(feature = "font-picker")]
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct FaceAxes<'a> {
    /// The face's axes.
    pub axes: &'a [craft_font_manager::axes::FontAxis],
    /// The face's named instances.
    pub instances: &'a [craft_font_manager::axes::NamedInstance],
    /// The weight of the selection, when it has one.
    pub weight: Option<f32>,
    /// The selection is italic.
    pub italic: bool,
}

#[cfg(feature = "font-picker")]
impl<'a> FaceAxes<'a> {
    /// The axes and named instances of a face; no weight, upright.
    pub fn new(axes: &'a [craft_font_manager::axes::FontAxis], instances: &'a [craft_font_manager::axes::NamedInstance]) -> Self {
        FaceAxes { axes, instances, weight: None, italic: false }
    }

    /// The weight of the selection.
    pub fn weight(mut self, weight: Option<f32>) -> Self {
        self.weight = weight;
        self
    }

    /// The selection is italic.
    pub fn italic(mut self, italic: bool) -> Self {
        self.italic = italic;
        self
    }
}

/// How the axes show: the sliders under the style row, or one button that opens them.
#[cfg(feature = "font-picker")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum AxesLayout {
    /// The sliders under the style row.
    #[default]
    Sliders,
    /// `AxesButton` under the style row.
    Button,
}

/// What the family field's font picker takes: the catalog, the app's stars, recents and filters, and the preview.
#[cfg(feature = "font-picker")]
#[non_exhaustive]
pub struct FontPickerSource<'a> {
    /// The catalog the picker lists.
    pub catalog: &'a Arc<Catalog>,
    /// The picker's settings (stars, recents); `None` for a caller without them.
    pub settings: Option<Bind<'a, FontPickerSettings>>,
    /// The picker's filters; `None` for a caller without them.
    pub filters: Option<Bind<'a, FontFilters>>,
    /// The text the previews show; the family's name when `None`.
    pub preview_text: Option<&'a str>,
    /// The families the app serves.
    pub served: &'a [String],
    /// The host's family sets.
    pub sets: &'a [FamilySet<'a>],
    /// The scripts the script row offers.
    pub scripts: &'a [FontScript],
    /// The layout of the list's filter area.
    pub filter_layout: FilterLayout,
    /// The host's font rows, listed above the catalogue families.
    pub host: &'a [HostFontGroup<'a>],
    /// The key of the host row shown as the field's value.
    pub current_host: Option<String>,
}

#[cfg(feature = "font-picker")]
impl<'a> FontPickerSource<'a> {
    /// A source over `catalog`, with no settings, filters or host rows.
    pub fn new(catalog: &'a Arc<Catalog>) -> Self {
        FontPickerSource {
            catalog,
            settings: None,
            filters: None,
            preview_text: None,
            served: &[],
            sets: &[],
            scripts: &[],
            filter_layout: FilterLayout::default(),
            host: &[],
            current_host: None,
        }
    }

    /// The app's stars and recents.
    pub fn settings(mut self, settings: Bind<'a, FontPickerSettings>) -> Self {
        self.settings = Some(settings);
        self
    }

    /// The app's filters.
    pub fn filters(mut self, filters: Bind<'a, FontFilters>) -> Self {
        self.filters = Some(filters);
        self
    }

    /// The host's font rows, listed above the catalogue families.
    pub fn host(mut self, groups: &'a [HostFontGroup<'a>]) -> Self {
        self.host = groups;
        self
    }

    /// The key of the host row shown as the field's value.
    pub fn current_host(mut self, key: Option<String>) -> Self {
        self.current_host = key;
        self
    }

    /// The text the previews show.
    pub fn preview_text(mut self, text: &'a str) -> Self {
        self.preview_text = Some(text);
        self
    }

    /// The families the app serves.
    pub fn served(mut self, served: &'a [String]) -> Self {
        self.served = served;
        self
    }

    /// The host's family sets, passed on to the family field's picker.
    pub fn sets(mut self, sets: &'a [FamilySet<'a>]) -> Self {
        self.sets = sets;
        self
    }

    /// The scripts the picker's script row offers.
    pub fn scripts(mut self, scripts: &'a [FontScript]) -> Self {
        self.scripts = scripts;
        self
    }

    /// The layout of the picker's filter area.
    pub fn filter_layout(mut self, layout: FilterLayout) -> Self {
        self.filter_layout = layout;
        self
    }
}

/// Leading: automatic (the font size × the paragraph's auto-leading, 120 % by default) or set.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Leading {
    /// Auto.
    Auto,
    /// The leading in document points, the representation of `CharacterValues::size`.
    Fixed(f32),
}

/// Kerning: from the font's pairs, from the glyph shapes, or a value in thousandths of an em
/// (0 is none). The field shows the number; the menu checks Metrics or Optical.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Kerning {
    /// Metrics.
    Metrics,
    /// Optical.
    Optical,
    /// A value in thousandths of an em; 0 is none.
    Manual(f32),
}

/// The names a kerning field holds besides a number.
#[derive(Clone, Copy, Debug, PartialEq)]
enum KerningName {
    Metrics,
    Optical,
}

impl NumberName for KerningName {
    fn id(self) -> u32 {
        match self {
            KerningName::Metrics => METRICS_ROW,
            KerningName::Optical => OPTICAL_ROW,
        }
    }

    fn from_id(id: u32) -> Option<Self> {
        match id {
            METRICS_ROW => Some(KerningName::Metrics),
            OPTICAL_ROW => Some(KerningName::Optical),
            _ => None,
        }
    }
}

impl KerningName {
    fn kerning(self) -> Kerning {
        match self {
            KerningName::Metrics => Kerning::Metrics,
            KerningName::Optical => Kerning::Optical,
        }
    }
}

/// The kerning a kerning field's value stands for; a value that is neither a number nor one of the names is the
/// default, Metrics.
fn kerning_of(value: NamedOr<KerningName>) -> Kerning {
    let named = || value.named().and_then(KerningName::from_id).map(KerningName::kerning);
    value.number().map(|number| Kerning::Manual(number as f32)).or_else(named).unwrap_or(Kerning::Metrics)
}

/// How letters are capitalized.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Caps {
    /// Normal.
    #[default]
    Normal,
    /// All caps.
    AllCaps,
    /// Small caps.
    SmallCaps,
}

/// Where the text sits relative to the baseline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BaselinePosition {
    /// Normal.
    #[default]
    Normal,
    /// Superscript.
    Superscript,
    /// Subscript.
    Subscript,
}

/// The character attributes at the type target. `None` in a field: the selection mixes values,
/// and the field shows empty (Photoshop).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct CharacterValues {
    /// The family.
    pub family: Option<String>,
    /// The style.
    pub style: Option<String>,
    /// Size in points.
    pub size: Option<f32>,
    /// The leading.
    pub leading: Option<Leading>,
    /// The kerning.
    pub kerning: Option<Kerning>,
    /// Tracking in thousandths of an em.
    pub tracking: Option<f32>,
    /// Percent, 100 is unscaled.
    pub vertical_scale: Option<f32>,
    /// Percent, 100 is unscaled.
    pub horizontal_scale: Option<f32>,
    /// Baseline shift in points; positive raises the text.
    pub baseline_shift: Option<f32>,
    /// The swatch's colour.
    pub color: Option<Rgba>,
    /// The faux bold.
    pub faux_bold: Option<bool>,
    /// The faux italic.
    pub faux_italic: Option<bool>,
    /// The caps.
    pub caps: Option<Caps>,
    /// The position.
    pub position: Option<BaselinePosition>,
    /// The underline.
    pub underline: Option<bool>,
    /// The strikethrough.
    pub strikethrough: Option<bool>,
    /// The [`OPENTYPE_FEATURES`] that are on, by tag.
    pub opentype: Vec<&'static str>,
    /// BCP 47 tag of the text's language (hyphenation, spelling, smart quotes).
    pub language: Option<String>,
    /// The variable-font axis values of the selection, by axis tag; `None` while the selection has no variable axes.
    pub axes: Option<Vec<([u8; 4], f32)>>,
    /// The axes (by tag) whose values differ across the selection; `axes` holds the shared ones.
    pub mixed_axes: Vec<[u8; 4]>,
    /// Rotation in degrees.
    pub rotation: Option<f32>,
    /// Insert Aki Left, thousandths of an em.
    pub aki_left: Option<f32>,
    /// Insert Aki Right, thousandths of an em.
    pub aki_right: Option<f32>,
    /// Tsume, percent.
    pub tsume: Option<f32>,
    /// Horizontal diacritic position, thousandths of an em.
    pub diacritic_x: Option<f32>,
    /// Vertical diacritic position, thousandths of an em.
    pub diacritic_y: Option<f32>,
    /// The pair kerning Metrics or Optical gives at the caret; typing a number in the kerning field sets `Manual`.
    pub auto_kerning: Option<f32>,
    /// The anti-aliasing method.
    pub anti_alias: Option<TextAntiAlias>,
    /// The Set Font Height Reference value.
    pub font_height: Option<FontHeightReference>,
    /// Extra space after each character, in points; `None` while the selection mixes it.
    pub char_spacing: Option<f32>,
}

/// A variation axis's tag and its value.
type AxisValue = ([u8; 4], f32);

impl CharacterValues {
    /// The axis values of the face, none while the selection has no variable axes.
    fn held_axes(&self) -> &[AxisValue] {
        self.axes.as_deref().unwrap_or_default()
    }

    /// The axes two selections share with equal values, and the tags of the others: an axis mixes where either
    /// selection mixes it or the values differ.
    fn mix_axes(&self, other: &Self) -> (Option<Vec<AxisValue>>, Vec<[u8; 4]>) {
        let shared: Vec<AxisValue> = self
            .held_axes()
            .iter()
            .filter(|(tag, value)| !self.mixed_axes.contains(tag) && !other.mixed_axes.contains(tag) && other.held_axes().contains(&(*tag, *value)))
            .copied()
            .collect();
        let mut mixed: Vec<[u8; 4]> = Vec::new();
        let tags = self.held_axes().iter().map(|(tag, _)| tag).chain(other.held_axes().iter().map(|(tag, _)| tag));
        for tag in tags.chain(&self.mixed_axes).chain(&other.mixed_axes) {
            if !mixed.contains(tag) && !shared.iter().any(|(shared, _)| shared == tag) {
                mixed.push(*tag);
            }
        }
        let axes = if self.axes.is_none() && other.axes.is_none() { None } else { Some(shared) };
        (axes, mixed)
    }

    /// The values of two selections together: each field stays where both are equal and becomes `None` where they
    /// differ.
    pub fn mixed_with(&self, other: &Self) -> Self {
        let (axes, mixed_axes) = self.mix_axes(other);
        CharacterValues {
            family: agree(&self.family, &other.family),
            style: agree(&self.style, &other.style),
            size: agree(&self.size, &other.size),
            leading: agree(&self.leading, &other.leading),
            kerning: agree(&self.kerning, &other.kerning),
            tracking: agree(&self.tracking, &other.tracking),
            vertical_scale: agree(&self.vertical_scale, &other.vertical_scale),
            horizontal_scale: agree(&self.horizontal_scale, &other.horizontal_scale),
            baseline_shift: agree(&self.baseline_shift, &other.baseline_shift),
            color: agree(&self.color, &other.color),
            faux_bold: agree(&self.faux_bold, &other.faux_bold),
            faux_italic: agree(&self.faux_italic, &other.faux_italic),
            caps: agree(&self.caps, &other.caps),
            position: agree(&self.position, &other.position),
            underline: agree(&self.underline, &other.underline),
            strikethrough: agree(&self.strikethrough, &other.strikethrough),
            opentype: self.opentype.iter().copied().filter(|tag| other.opentype.contains(tag)).collect(),
            language: agree(&self.language, &other.language),
            axes,
            mixed_axes,
            rotation: agree(&self.rotation, &other.rotation),
            aki_left: agree(&self.aki_left, &other.aki_left),
            aki_right: agree(&self.aki_right, &other.aki_right),
            tsume: agree(&self.tsume, &other.tsume),
            diacritic_x: agree(&self.diacritic_x, &other.diacritic_x),
            diacritic_y: agree(&self.diacritic_y, &other.diacritic_y),
            auto_kerning: agree(&self.auto_kerning, &other.auto_kerning),
            anti_alias: agree(&self.anti_alias, &other.anti_alias),
            font_height: agree(&self.font_height, &other.font_height),
            char_spacing: agree(&self.char_spacing, &other.char_spacing),
        }
    }
}

/// One change from the Character controls.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CharacterChange {
    /// Family.
    Family(String),
    /// A style of the current family; the app keeps the family.
    Style(String),
    /// Size in points.
    Size(f32),
    /// Leading.
    Leading(Leading),
    /// Kerning.
    Kerning(Kerning),
    /// Tracking in thousandths of an em.
    Tracking(f32),
    /// Percent, 100 is unscaled.
    VerticalScale(f32),
    /// Percent, 100 is unscaled.
    HorizontalScale(f32),
    /// Baseline shift in points.
    BaselineShift(f32),
    /// Faux bold.
    FauxBold(bool),
    /// Faux italic.
    FauxItalic(bool),
    /// All Caps and Small Caps are one choice: pressing the pressed one returns `Normal`.
    Caps(Caps),
    /// Superscript and Subscript, the same way.
    Position(BaselinePosition),
    /// Underline.
    Underline(bool),
    /// Strikethrough.
    Strikethrough(bool),
    /// An OpenType feature switched on or off.
    #[non_exhaustive]
    OpenType {
        /// The feature's four-letter tag, such as "liga".
        tag: &'static str,
        /// On.
        on: bool,
    },
    /// Language.
    Language(String),
    /// The variable-font axis values, by axis tag.
    Axes(Vec<([u8; 4], f32)>),
    /// Rotation in degrees.
    Rotation(f32),
    /// Anti-aliasing.
    AntiAlias(TextAntiAlias),
    /// Set Font Height Reference.
    FontHeight(FontHeightReference),
    /// Insert Aki Left, thousandths of an em.
    AkiLeft(f32),
    /// Insert Aki Right, thousandths of an em.
    AkiRight(f32),
    /// Tsume, percent.
    Tsume(f32),
    /// Horizontal diacritic position, thousandths of an em.
    DiacriticX(f32),
    /// Vertical diacritic position, thousandths of an em.
    DiacriticY(f32),
    /// Extra space after each character, in points.
    CharSpacing(f32),
}

/// What the Character controls did this pass.
#[non_exhaustive]
pub struct CharacterControlsOutput {
    /// The controls' response.
    pub response: Response,
    /// The edit events of this pass, in the order of the controls (usually one; a preset pick and a toggle in the
    /// same pass are two). A scrub or a typing run in a field is a gesture (`Begin`, `Update`, then `Commit`, or
    /// `Cancel` on Esc): the app merges its steps into one history step. A preset, toggle or select is a `Set`. The
    /// app applies them in order.
    pub edits: Vec<EditEvent<CharacterChange>>,
    /// What this pass did to the live preview of the selected text: the family under the pointer or the keyboard row
    /// of the open font list (only with `font-picker`).
    pub preview_family: Preview<String>,
    /// The family the pointer rested on in the open font list long enough for the app to load its preview.
    pub rested_family: Option<String>,
    /// The colour swatch was clicked: the app opens its colour picker for this request (key [`COLOR_KEY`]) and sets
    /// the colour. Set for the one pass of the click.
    pub requests: HostRequests,
    /// The family `FontPicker`'s id this pass, which `FrameOutput::ended_previews` names when the controls stop
    /// showing with a preview.
    pub picker: Option<egui::Id>,
}

impl Outcome for CharacterControlsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The Character panel's controls, in Photoshop's rows: family; style; size and leading; kerning and tracking;
/// vertical and horizontal scale; baseline shift ([`CharacterRows::BASELINE`]) and colour; the style toggles
/// ([`CharacterRows::STYLE_TOGGLES`]: Faux Bold, Faux Italic, All Caps, Small Caps, then Superscript and Subscript with
/// [`CharacterRows::SUPERSCRIPT`], Underline, Strikethrough); the OpenType buttons; the language when
/// [`languages`](Self::languages) is given. [`rows`](Self::rows) turns these and the other groups (rotation,
/// anti-aliasing, font height reference, East Asian and Middle Eastern fields) on or off;
/// [`CharacterRows::PROPERTIES`] keeps family, style, the axes, size, leading, kerning and tracking.
///
/// ```ignore
/// let out = CharacterControls::new("Character", &values, &families, &styles).unit(DocUnit::points()).show(ui);
/// for edit in out.edits { gestures.handle(&mut doc, edit, apply_to_selection) }
/// ```
///
/// - Fields fill two columns where they fit and stack in narrower panels. A mark serves as a label (PhotoCraft's marks: "tT", "A↕", "VA",
///   "↕T", "↔T", "Aª"); a drag on the mark scrubs the value. Size, leading, kerning and tracking have a preset menu in
///   the field (presets are shortcuts, not limits: any value in range can be typed). Ranges in points: size
///   0.1–1296, leading 0.1–5000, baseline shift −1296–1296; tracking and kerning −1000–10000; scales 0–1000 %. The
///   leading menu starts with an Auto row ([`Leading::Auto`]); while leading is Auto the field shows the font size
///   × 120 %. The kerning menu starts with Metrics and Optical; while either is the kerning the field shows its name,
///   and the name typed sets it.
/// - Family: craft-font-manager's `FontPicker` with `font-picker` and `font_picker`, else a
///   `Select` of `families`.
/// - Toggles are toggle buttons in one row, pressed while on; Tab reaches each.
/// - A mixed value shows an empty field or an unpressed toggle; typing or pressing sets it for the whole selection.
#[must_use = "show the controls with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct CharacterControls<'a> {
    label: Label,
    values: &'a CharacterValues,
    families: &'a [String],
    styles: &'a [String],
    /// The unit the length fields show (Preferences › Units & Rulers › Type) and what converts it: the document's
    /// resolution for pixels. Values stay in points. Default points.
    #[prop]
    unit: DocUnit,
    /// Shows the Superscript and Subscript toggles (Photoshop; off by default, PhotoCraft has no superscript).
    #[prop]
    superscript: bool,
    /// Shows the Language select over `(tag, name)` pairs (Photoshop's bottom row).
    #[prop]
    languages: Option<&'a [(String, String)]>,
    /// The family field is craft-font-manager's font picker over `source`: the catalog, the app's stars, recents and
    /// filters, the preview text and the served families.
    #[cfg(feature = "font-picker")]
    #[prop(name = "font_picker")]
    picker: Option<FontPickerSource<'a>>,
    /// Shows the variable-font axis sliders of `axes` under the style row.
    #[cfg(feature = "font-picker")]
    #[prop]
    axes: Option<FaceAxes<'a>>,
    /// How `axes` shows: the sliders under the style row, or `AxesButton` under it.
    #[cfg(feature = "font-picker")]
    #[prop]
    axes_layout: AxesLayout,
    /// The row groups shown.
    #[prop]
    rows: CharacterRows,
    /// The methods the Anti-aliasing select offers (with `CharacterRows::ANTI_ALIAS`).
    #[prop]
    anti_alias_methods: &'a [TextAntiAlias],
}

impl<'a> CharacterControls<'a> {
    /// Controls named `label` for `values`, with the installed `families` and the current family's `styles`.
    pub fn new(label: impl Into<Label>, values: &'a CharacterValues, families: &'a [String], styles: &'a [String]) -> Self {
        CharacterControls {
            label: label.into(),
            values,
            families,
            styles,
            unit: DocUnit::points(),
            superscript: false,
            languages: None,
            #[cfg(feature = "font-picker")]
            picker: None,
            #[cfg(feature = "font-picker")]
            axes: None,
            #[cfg(feature = "font-picker")]
            axes_layout: AxesLayout::default(),
            rows: CharacterRows::default(),
            anti_alias_methods: &TextAntiAlias::BAR,
        }
    }

    /// Shows the controls and returns the changes the user made.
    pub fn show(self, ui: &mut Ui) -> CharacterControlsOutput {
        let values = self.values;
        let (families, styles, languages) = (self.families, self.styles, self.languages);
        let unit = self.unit;
        let (rows, anti_alias_methods) = (self.rows, self.anti_alias_methods);
        #[cfg(feature = "font-picker")]
        let axes_layout = self.axes_layout;
        #[cfg(feature = "font-picker")]
        let mut picker = self.picker;
        #[cfg(feature = "font-picker")]
        let axes = self.axes;
        let env = Env::of(ui);
        let words = env.words::<CharacterWords>();
        let units = env.units();
        let lengths = Lengths { units: &units, unit };

        let size_presets = size_menu(&units, unit);
        let leading_presets = leading_menu(&words, values.leading, &size_presets);
        let tracking_presets = number_presets(&units, TRACKING_PRESETS.iter().copied());
        let kerning_presets: Vec<Entry<NumberPick>> = [
            Entry::new(NumberPick::Choice(METRICS_ROW), words.metrics.clone()).checked(values.kerning == Some(Kerning::Metrics)),
            Entry::new(NumberPick::Choice(OPTICAL_ROW), words.optical.clone()).checked(values.kerning == Some(Kerning::Optical)),
            Entry::separator(),
        ]
        .into_iter()
        .chain(KERNING_PRESETS.iter().map(|&p| Entry::new(NumberPick::Number(f64::from(p)), units.number(f64::from(p), 0..=PRESET_DECIMALS))))
        .collect();

        let style_entries = string_entries(styles.iter().map(|name| (name.clone(), name.clone())));
        let language_entries = languages.map(|languages| string_entries(languages.iter().cloned()));

        let kerning_shown = values.kerning.map(|kerning| match kerning {
            Kerning::Manual(value) => NamedOr::Number(f64::from(value)),
            Kerning::Metrics => NamedOr::Named(KerningName::Metrics),
            Kerning::Optical => NamedOr::Named(KerningName::Optical),
        });

        let mut edits: Vec<EditEvent<CharacterChange>> = Vec::new();
        let mut preview_family = Preview::Unchanged;
        let mut rested_family = None;
        let mut requests = HostRequests::new();
        let mut picker_id = None;
        let cells: Vec<Cell> = Cell::ALL.into_iter().filter(|cell| cell.shown(rows)).collect();

        // A short field needs room for its value, plus space for the mark and preset button.
        let tokens = &env.tokens;
        let field_width = tokens.size.length.short + 2.0 * tokens.size.icon.md;
        let pair_width = 2.0 * field_width + tokens.space.gap.row;
        let columns = if ui.available_width() < pair_width { &TWO_COLUMNS[..1] } else { &TWO_COLUMNS };
        let shown = Stack::new().label(self.label).show(ui, |ui, _| {
            #[cfg(feature = "font-picker")]
            let family = family_field_keyed(ui, &words, values, families, Some(keys::FAMILY), None, Some(Sizing::FillWidth), picker.as_mut());
            #[cfg(not(feature = "font-picker"))]
            let family = family_field_keyed(ui, &words, values, families, Some(keys::FAMILY), None, Some(Sizing::FillWidth));
            preview_family = family.preview;
            rested_family = family.rested;
            picker_id = family.picker;
            edits.extend(family.edits);
            edits.extend(select_row(
                ui,
                &words.style,
                hidden_side(),
                mixed(values.style.clone()),
                &style_entries,
                Some(Sizing::FillWidth),
                CharacterChange::Style,
            ));
            #[cfg(feature = "font-picker")]
            if let Some(axes) = axes {
                let held = values.axes.clone().unwrap_or_default();
                let edit = match axes_layout {
                    AxesLayout::Sliders => {
                        let mut sliders = AxisSliders::new(words.axes.clone(), axes.axes, Bind::given(held))
                            .instances(axes.instances)
                            .italic(axes.italic)
                            .mixed(&values.mixed_axes);
                        if let Some(weight) = axes.weight {
                            sliders = sliders.weight(weight);
                        }
                        sliders.show(ui).edit
                    }
                    AxesLayout::Button => {
                        let mut button = AxesButton::new(words.variable_font.clone(), axes.axes, Bind::given(held))
                            .instances(axes.instances)
                            .italic(axes.italic)
                            .mixed(&values.mixed_axes);
                        if let Some(weight) = axes.weight {
                            button = button.weight(weight);
                        }
                        button.show(ui).edit
                    }
                };
                edits.extend(edit.into_iter().map(|event| event.map(CharacterChange::Axes)));
            }

            let mut draw = |ui: &mut Ui, cell: Cell| {
                if let Some(spec) = cell.plain(&words, values) {
                    let presets = number_presets(&units, std::iter::once(spec.presets));
                    let row = PlainRow {
                        label: spec.label,
                        side: mark(GlyphLabel::text(spec.mark), None, FieldLayout::Inside),
                        value: plain(spec.value),
                        range: spec.range,
                        percent: spec.percent,
                        presets: &presets,
                        disabled: None,
                    };
                    let (event, _) = lengths.plain(ui, row);
                    edits.extend(number_change(event, spec.change));
                    return;
                }
                match cell {
                    Cell::Size => {
                        let size = LengthRow {
                            label: words.size.clone(),
                            side: mark(GlyphLabel::text("tT"), None, FieldLayout::Inside),
                            value: values.size,
                            range: SIZE_RANGE,
                            presets: &size_presets,
                            disabled: None,
                        };
                        let (event, _) = lengths.row_keyed(ui, size, Some(keys::SIZE));
                        edits.extend(number_change(event, CharacterChange::Size));
                    }
                    Cell::Leading => {
                        let side = mark(GlyphLabel::text("A↕"), Some(words.leading_hint.clone()), FieldLayout::Inside);
                        edits.extend(leading_row_keyed(
                            &lengths,
                            ui,
                            &words,
                            side,
                            values.leading,
                            values.size,
                            &leading_presets,
                            Some(keys::LEADING),
                        ));
                    }
                    Cell::FontHeight => {
                        let entries = font_height_entries(&words);
                        edits.extend(select_row(
                            ui,
                            &words.font_height,
                            hidden_side(),
                            mixed(values.font_height),
                            &entries,
                            Some(Sizing::FillWidth),
                            CharacterChange::FontHeight,
                        ));
                    }
                    Cell::Kerning => {
                        // PLAN: `NumberField::named` has no per-value display text, so the field shows the name
                        // (Metrics, Optical) and `auto_kerning` is not shown.
                        let event = lengths.kerning(
                            ui,
                            words.kerning.clone(),
                            mark(GlyphLabel::icon(IconId::new("text_cursor")), Some(words.kerning_hint.clone()), FieldLayout::Inside),
                            kerning_shown,
                            &kerning_presets,
                            Some(keys::KERNING),
                        );
                        edits.extend(event.map(|value| CharacterChange::Kerning(kerning_of(value))));
                    }
                    Cell::Tracking => {
                        let tracking = PlainRow {
                            label: words.tracking.clone(),
                            side: mark(GlyphLabel::text("VA"), Some(words.tracking_hint.clone()), FieldLayout::Inside),
                            value: plain(values.tracking),
                            range: TRACKING_RANGE,
                            percent: false,
                            presets: &tracking_presets,
                            disabled: None,
                        };
                        let (event, _) = lengths.plain_keyed(ui, tracking, Some(keys::TRACKING));
                        edits.extend(number_change(event, CharacterChange::Tracking));
                    }
                    Cell::BaselineShift => {
                        let shift = LengthRow {
                            label: words.baseline_shift.clone(),
                            side: mark(GlyphLabel::text("Aª"), None, FieldLayout::Inside),
                            value: values.baseline_shift,
                            range: SHIFT_RANGE,
                            presets: &[],
                            disabled: None,
                        };
                        let (event, _) = lengths.row(ui, shift);
                        edits.extend(number_change(event, CharacterChange::BaselineShift));
                    }
                    Cell::Color => {
                        let color = mixed(values.color.map(Srgba::from));
                        let side = SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::text(words.color.clone())));
                        let well = Field::new(words.text_color.clone()).side(side).show(ui, |ui, _| {
                            Swatch::new(words.text_color.clone(), color).picker(COLOR_KEY).sizing(Sizing::FillWidth).show(ui).pick_color
                        });
                        if let Some(request) = well.inner.flatten() {
                            requests.request_color(request);
                        }
                    }
                    Cell::AntiAlias => {
                        let option_words = env.words::<TypeOptionWords>();
                        let entries: Vec<Entry<TextAntiAlias>> =
                            anti_alias_methods.iter().map(|&method| Entry::new(method, method.label(&option_words).clone())).collect();
                        edits.extend(select_row(
                            ui,
                            &words.anti_alias,
                            hidden_side(),
                            mixed(values.anti_alias),
                            &entries,
                            Some(Sizing::FillWidth),
                            CharacterChange::AntiAlias,
                        ));
                    }
                    // The plain number cells are drawn above.
                    Cell::VerticalScale
                    | Cell::HorizontalScale
                    | Cell::Rotation
                    | Cell::AkiLeft
                    | Cell::AkiRight
                    | Cell::Tsume
                    | Cell::DiacriticX
                    | Cell::DiacriticY => {}
                }
            };
            for run in cells.chunk_by(|a, b| !a.full() && !b.full()) {
                match run {
                    [cell] if cell.full() => draw(ui, *cell),
                    _ => {
                        Grid::new(columns).fill_width().show(ui, |ui, _| {
                            for &cell in run {
                                draw(ui, cell);
                            }
                        });
                    }
                }
            }

            if rows.contains(CharacterRows::STYLE_TOGGLES) {
                let caps = values.caps;
                let position = values.position;
                let mut toggles: Vec<Toggle<CharacterChange>> = Vec::new();
                if rows.contains(CharacterRows::FAUX) {
                    toggles.push(Toggle::new(&words.faux_bold, "text_bold", values.faux_bold, Box::new(CharacterChange::FauxBold)));
                    toggles.push(Toggle::new(&words.faux_italic, "text_italic", values.faux_italic, Box::new(CharacterChange::FauxItalic)));
                }
                toggles.push(Toggle::new(
                    &words.all_caps,
                    "text_all_caps",
                    caps.map(|caps| caps == Caps::AllCaps),
                    Box::new(|on| CharacterChange::Caps(if on { Caps::AllCaps } else { Caps::Normal })),
                ));
                toggles.push(Toggle::new(
                    &words.small_caps,
                    "text_small_caps",
                    caps.map(|caps| caps == Caps::SmallCaps),
                    Box::new(|on| CharacterChange::Caps(if on { Caps::SmallCaps } else { Caps::Normal })),
                ));
                if rows.contains(CharacterRows::SUPERSCRIPT) {
                    toggles.push(Toggle::new(
                        &words.superscript,
                        "text_superscript",
                        position.map(|position| position == BaselinePosition::Superscript),
                        Box::new(|on| CharacterChange::Position(if on { BaselinePosition::Superscript } else { BaselinePosition::Normal })),
                    ));
                    toggles.push(Toggle::new(
                        &words.subscript,
                        "text_subscript",
                        position.map(|position| position == BaselinePosition::Subscript),
                        Box::new(|on| CharacterChange::Position(if on { BaselinePosition::Subscript } else { BaselinePosition::Normal })),
                    ));
                }
                toggles.push(Toggle::new(&words.underline, "text_underline", values.underline, Box::new(CharacterChange::Underline)));
                toggles.push(Toggle::new(&words.strikethrough, "text_strikethrough", values.strikethrough, Box::new(CharacterChange::Strikethrough)));
                edits.extend(toggle_row(ui, None, &toggles));
            }

            if rows.contains(CharacterRows::OPENTYPE) {
                let names = [
                    &words.standard_ligatures,
                    &words.contextual_alternates,
                    &words.discretionary_ligatures,
                    &words.swash,
                    &words.stylistic_alternates,
                    &words.titling_alternates,
                    &words.ordinals,
                    &words.fractions,
                ];
                let features: Vec<Toggle<CharacterChange>> = OPENTYPE_FEATURES
                    .iter()
                    .zip(OPENTYPE_ICONS)
                    .zip(names)
                    .map(|((&tag, icon), name)| {
                        Toggle::new(name, icon, Some(values.opentype.contains(&tag)), Box::new(move |on| CharacterChange::OpenType { tag, on }))
                    })
                    .collect();
                edits.extend(toggle_row(ui, None, &features));
            }

            if let Some(entries) = &language_entries {
                edits.extend(select_row(
                    ui,
                    &words.language,
                    hidden_side(),
                    mixed(values.language.clone()),
                    entries,
                    Some(Sizing::FillWidth),
                    CharacterChange::Language,
                ));
            }
        });
        CharacterControlsOutput { response: shown.output, edits, preview_family, rested_family, requests, picker: picker_id }
    }
}

/// The OpenType buttons' icons in [`OPENTYPE_FEATURES`] order. The sets have no OpenType marks: the three ligature
/// features share the ligature icon, swash and stylistic alternates use the variable-font and similar-font icons,
/// titling the type icon, ordinals the superscript icon and fractions the slash.
const OPENTYPE_ICONS: [&str; 8] = ["ligature", "ligature", "ligature", "font_variable", "font_similar", "text", "text_superscript", "slash"];
