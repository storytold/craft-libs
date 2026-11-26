//! `TypeBar`: the type tool's options bar, the family, style, size, alignment, colour and panels buttons in one
//! strip.

use craft_behavior::{EditEvent, IconId, Preview};
use craft_theme::Srgba;
use craft_ui::buttons::{IconButton, ToggleButton};
use craft_ui::choice::Swatch;
use craft_ui::contract::{
    Bind, Entry, FieldLayout, Gap, GlyphLabel, LabelKind, NoValue, Outcome, Props as _, Show as _, ShowContainer as _, SideLabel, clicked,
};
use craft_ui::display::Divider;
use craft_ui::form::Field;
use craft_ui::layout_components::Row;
use craft_ui::patterns::form_dialogs::HostRequests;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::Env;
use egui::{Response, Ui};

use crate::ui::character::paragraph::{align_toggles, direction_toggles};
use crate::ui::character::rows::{
    LengthRow, Lengths, family_field, hidden_side, mark, mixed, number_change, select_row, size_menu, string_entries, toggle_row,
};
use crate::ui::character::{
    COLOR_KEY, CharacterChange, CharacterValues, CharacterWords, ParagraphChange, ParagraphValues, ParagraphWords, SIZE_RANGE,
};
use crate::ui::length::DocUnit;
use crate::ui::type_options::{TextAntiAlias, TextOrientation, TypeOptionWords};

#[cfg(feature = "font-picker")]
use crate::ui::character::FontPickerSource;

#[cfg(feature = "suite")]
pub(crate) mod stories;

craft_ui::words! {
    /// The type bar's tooltips, English by default (PhotoCraft's). Family, alignment and direction names are
    /// [`CharacterWords`] and [`ParagraphWords`].
    #[non_exhaustive]
    pub struct TypeBarWords {
        orientation: "typeBar.orientation" = "Toggle text orientation";
        style: "typeBar.style" = "Set font style";
        size: "typeBar.size" = "Set font size";
        warp: "typeBar.warp" = "Create warped text";
        panels: "typeBar.panels" = "Toggle Character and Paragraph panels";
    }
}

/// One change from the type bar.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TypeBarChange {
    /// A character field.
    Character(CharacterChange),
    /// A paragraph field.
    Paragraph(ParagraphChange),
    /// The anti-aliasing method.
    AntiAlias(TextAntiAlias),
    /// The text orientation.
    Orientation(TextOrientation),
}

/// What the type bar did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct TypeBarOutput {
    /// The bar's response.
    pub response: Response,
    /// The edit events of this pass.
    pub edits: Vec<EditEvent<TypeBarChange>>,
    /// What this pass did to the live preview of the selected text: the family under the pointer or the keyboard row
    /// of the open font list.
    pub preview_family: Preview<String>,
    /// The family the pointer rested on in the open font list.
    pub rested_family: Option<String>,
    /// The colour well was clicked.
    pub requests: HostRequests,
    /// The Warp Text button was pressed.
    pub warp: bool,
    /// The Character and Paragraph panels button was pressed.
    pub panels: bool,
}

impl Outcome for TypeBarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The options bar of the type tool: the family and style, the size, the anti-aliasing menu, the alignment buttons,
/// the colour well, Warp Text and the panels button. Show it as children of the app's `Bar::new(label,
/// BarKind::Options)`. The alignment buttons show the vertical glyphs when `orientation` is `Some(Vertical)`.
#[must_use = "show the bar with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct TypeBar<'a> {
    character: &'a CharacterValues,
    paragraph: &'a ParagraphValues,
    families: &'a [String],
    styles: &'a [String],
    /// The unit the length fields show. Default points.
    #[prop]
    unit: DocUnit,
    /// The text's orientation; `None` while the selection mixes.
    #[prop]
    orientation: Option<TextOrientation>,
    /// The anti-aliasing method; `None` while the selection mixes.
    #[prop]
    anti_alias: Option<TextAntiAlias>,
    /// The methods the anti-aliasing menu offers.
    #[prop]
    anti_alias_methods: &'a [TextAntiAlias],
    /// Warp Text is available.
    #[prop]
    can_warp: bool,
    /// The family field is craft-font-manager's font picker over `source`.
    #[cfg(feature = "font-picker")]
    #[prop(name = "font_picker")]
    picker: Option<FontPickerSource<'a>>,
}

impl<'a> TypeBar<'a> {
    /// The bar over `character` and `paragraph` values, with the installed `families` and the family's `styles`.
    pub fn new(character: &'a CharacterValues, paragraph: &'a ParagraphValues, families: &'a [String], styles: &'a [String]) -> Self {
        TypeBar {
            character,
            paragraph,
            families,
            styles,
            unit: DocUnit::points(),
            orientation: None,
            anti_alias: None,
            anti_alias_methods: &TextAntiAlias::BAR,
            can_warp: false,
            #[cfg(feature = "font-picker")]
            picker: None,
        }
    }

    /// Shows the bar and returns the changes the user made. The bar keeps no state: each pass reports only the
    /// edits of its own input.
    pub fn show(self, ui: &mut Ui) -> TypeBarOutput {
        let character = self.character;
        let paragraph = self.paragraph;
        let (families, styles, unit) = (self.families, self.styles, self.unit);
        let (orientation, anti_alias, methods, can_warp) = (self.orientation, self.anti_alias, self.anti_alias_methods, self.can_warp);
        #[cfg(feature = "font-picker")]
        let mut picker = self.picker;
        let env = Env::of(ui);
        let bar_words = env.words::<TypeBarWords>();
        let character_words = env.words::<CharacterWords>();
        let paragraph_words = env.words::<ParagraphWords>();
        let option_words = env.words::<TypeOptionWords>();
        let units = env.units();
        let lengths = Lengths { units: &units, unit };
        let vertical = orientation == Some(TextOrientation::Vertical);

        let style_entries = string_entries(styles.iter().map(|name| (name.clone(), name.clone())));
        let size_presets = size_menu(&units, unit);
        let method_entries: Vec<Entry<TextAntiAlias>> =
            methods.iter().map(|&method| Entry::new(method, method.label(&option_words).clone())).collect();
        let aligns = align_toggles(paragraph, &paragraph_words, vertical);
        let directions = direction_toggles(paragraph, &paragraph_words);

        let mut edits: Vec<EditEvent<TypeBarChange>> = Vec::new();
        let mut preview_family = Preview::Unchanged;
        let mut rested_family = None;
        let mut requests = HostRequests::new();
        let (mut warp, mut panels) = (false, false);

        let row = Row::new().gap(Gap::Item).show(ui, |ui, _| {
            let shown_vertical = orientation.map(|o| o == TextOrientation::Vertical);
            let toggle = ToggleButton::new(bar_words.orientation.clone(), Bind::given(mixed(shown_vertical)))
                .icon(IconId::new("text_cursor"))
                .icon_only()
                .show(ui);
            edits
                .extend(toggle.events.map(|on| TypeBarChange::Orientation(if on { TextOrientation::Vertical } else { TextOrientation::Horizontal })));

            #[cfg(feature = "font-picker")]
            let family = family_field(ui, &character_words, character, families, None, None, picker.as_mut());
            #[cfg(not(feature = "font-picker"))]
            let family = family_field(ui, &character_words, character, families, None, None);
            preview_family = family.preview;
            rested_family = family.rested;
            edits.extend(family.edits.into_iter().map(|event| event.map(TypeBarChange::Character)));

            let style = select_row(ui, &bar_words.style, hidden_side(), mixed(character.style.clone()), &style_entries, None, CharacterChange::Style);
            edits.extend(style.into_iter().map(|event| event.map(TypeBarChange::Character)));

            // The size glyph the Character panel's size row shows.
            let side = mark(GlyphLabel::text("tT"), None, FieldLayout::Inside);
            let size =
                LengthRow { label: bar_words.size.clone(), side, value: character.size, range: SIZE_RANGE, presets: &size_presets, disabled: None };
            let (event, _) = lengths.row(ui, size);
            edits.extend(number_change(event, CharacterChange::Size).into_iter().map(|event| event.map(TypeBarChange::Character)));

            let method = Field::new(option_words.anti_alias.clone()).side(hidden_side()).show(ui, |ui, _| {
                Select::new(option_words.anti_alias.clone(), Bind::given(mixed(anti_alias)), &method_entries).placeholder("—").show(ui).events
            });
            edits.extend(method.inner.unwrap_or_default().map(TypeBarChange::AntiAlias));
            Divider::vertical().show(ui);

            edits.extend(toggle_row(ui, None, &aligns).into_iter().map(|event| event.map(TypeBarChange::Paragraph)));
            Divider::vertical().show(ui);
            edits.extend(toggle_row(ui, None, &directions).into_iter().map(|event| event.map(TypeBarChange::Paragraph)));
            Divider::vertical().show(ui);

            let color = mixed(character.color.map(Srgba::from));
            let side = SideLabel::new().kind(LabelKind::Hidden).layout(FieldLayout::Beside);
            let well = Field::new(character_words.text_color.clone())
                .side(side)
                .show(ui, |ui, _| Swatch::new(character_words.text_color.clone(), color).picker(COLOR_KEY).show(ui).pick_color);
            if let Some(request) = well.inner.flatten() {
                requests.request_color(request);
            }

            let button = IconButton::new(IconId::new("text_warp")).label(bar_words.warp.clone()).enabled(can_warp).show(ui);
            warp = clicked(&button);
            let button = IconButton::new(IconId::new("panels_top_left")).label(bar_words.panels.clone()).show(ui);
            panels = clicked(&button);
        });
        TypeBarOutput { response: row.output.response().clone(), edits, preview_family, rested_family, requests, warp, panels }
    }
}
