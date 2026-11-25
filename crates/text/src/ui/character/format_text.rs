//! [`FormatText`]: the Format Text panel, a character and paragraph editor in five rows.

use std::ops::RangeInclusive;

use crate::ui::length::{DocUnit, shown};
use craft_behavior::{EditEvent, IconId};
use craft_theme::Srgba;
use craft_ui::buttons::{IconButton, PopupMark};
use craft_ui::choice::Swatch;
use craft_ui::contract::{
    Bind, Entry, FieldLayout, Gap, GlyphLabel, LabelKind, NoValue, Outcome, PopupKind, Props as _, Show as _, ShowContainer as _, SideLabel, Sizing,
    Trigger, clicked,
};
use craft_ui::display::Divider;
use craft_ui::form::Field;
use craft_ui::layout_components::{Grid, Row, Stack};
use craft_ui::menu::Menu;
use craft_ui::patterns::form_dialogs::HostRequests;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label, Units};
use egui::{Response, Ui};

#[cfg(feature = "font-picker")]
use super::FontPickerSource;
use super::paragraph::SPACE_RANGE;
use super::rows::{
    LengthRow, Lengths, PlainRow, Toggle, disabled_for, family_field, length_menu, mark, mixed, number_change, number_presets, picked_event, plain,
    toggle_row,
};
use super::{
    BaselinePosition, COLOR_KEY, CharacterChange, CharacterValues, CharacterWords, LineSpacing, PRESET_DECIMALS, ParagraphChange,
    ParagraphDialogWords, ParagraphValues, ParagraphWords, TWO_COLUMNS, TextAlign,
};

craft_ui::words! {
    /// The Format Text panel's words, English by default. The size, colour, underline, superscript, subscript and
    /// scale controls take their names from [`CharacterWords`], the alignment menu from [`ParagraphWords`].
    #[non_exhaustive]
    pub struct FormatTextWords {
        bulleted_list: "formatText.bulletedList" = "Bulleted list";
        numbered_list: "formatText.numberedList" = "Numbered list";
        line_spacing: "formatText.lineSpacing" = "Line spacing";
        paragraph_spacing: "formatText.paragraphSpacing" = "Paragraph spacing";
        char_spacing: "formatText.charSpacing" = "Character spacing";

        bold: "formatText.bold" = "Bold";
        italic: "formatText.italic" = "Italic";
    }
}

/// A control of the Format Text panel, named to disable it with a reason through
/// [`FormatText::unsupported`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FormatControl {
    /// The font family.
    Font,
    /// The font size.
    Size,
    /// The text colour.
    Color,
    /// The bulleted list button.
    BulletedList,
    /// The numbered list button.
    NumberedList,
    /// The alignment buttons.
    Align,
    /// Bold.
    Bold,
    /// Italic.
    Italic,
    /// Underline.
    Underline,
    /// Superscript.
    Superscript,
    /// Subscript.
    Subscript,
    /// The line spacing.
    LineSpacing,
    /// The space before and after a paragraph.
    ParagraphSpacing,
    /// The horizontal scale.
    HorizontalScale,
    /// The character spacing.
    CharSpacing,
}

/// A kind of list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ListKind {
    /// A bulleted list.
    Bulleted,
    /// A numbered list.
    Numbered,
}

/// One change from the Format Text panel.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum FormatTextChange {
    /// A character change; a style row pick is `Character(Family(..))` then `Character(Style(..))`.
    Character(CharacterChange),
    /// The key of a host row picked in the font picker.
    HostFont(String),
    /// A paragraph change.
    Paragraph(ParagraphChange),
    /// A line spacing.
    LineSpacing(LineSpacing),
    /// A list button.
    List(ListKind),
}

/// What the Format Text panel did this pass.
#[non_exhaustive]
pub struct FormatTextOutput {
    /// The panel's response.
    pub response: Response,
    /// The edit events of this pass, in the order of the controls; a scrub or a typing run is one gesture.
    pub edits: Vec<EditEvent<FormatTextChange>>,
    /// The colour swatch was clicked: the app opens its colour picker for this request.
    pub requests: HostRequests,
}

impl Outcome for FormatTextOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The line spacings the menu offers by default: the text's own, then the common multiples.
const LINE_PRESETS: [LineSpacing; 5] =
    [LineSpacing::Auto, LineSpacing::Multiple(1.0), LineSpacing::Multiple(1.15), LineSpacing::Multiple(1.5), LineSpacing::Multiple(2.0)];

/// The Format Text panel pattern: five rows (font; size, colour, list buttons, alignment; bold, italic, underline,
/// superscript, subscript; line and paragraph spacing; scale and character spacing). A control in `unsupported` is
/// disabled with its reason. It keeps no state between passes: each pass reports only the edits of its own input.
///
/// - Font: the family field the Character controls show: craft-font-manager's `FontPicker` with
///   `font-picker` and `font_picker` (with the host's rows), else a `Select` of `families`. A family pick is
///   `Character(Family(..))`, a style row pick `Character(Family(..))` then `Character(Style(..))`, and a host row
///   pick `HostFont(key)`.
/// - Size, paragraph spacing and character spacing are lengths in points; horizontal scale is a percentage. A scrub
///   or a typing run is one gesture (`Begin`, `Update`, then `Commit`, or `Cancel` on Esc); a preset, toggle, list
///   button, alignment or line spacing pick is a `Set`.
/// - The colour swatch asks the app for a colour through [`FormatTextOutput::requests`] (key [`COLOR_KEY`]).
/// - Line spacing lists `line_presets`, and the current spacing after them when it is none of them; a mixed one shows
///   empty.
#[must_use = "show the controls with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct FormatText<'a> {
    label: Label,
    character: &'a CharacterValues,
    paragraph: &'a ParagraphValues,
    families: &'a [String],
    /// The family field is craft-font-manager's font picker over the source, with the host's rows.
    #[cfg(feature = "font-picker")]
    #[prop(name = "font_picker")]
    picker: Option<FontPickerSource<'a>>,
    /// The paragraphs' line spacing; `None` while they mix it.
    #[prop]
    line_spacing: Option<LineSpacing>,
    /// The line spacings the line spacing menu offers.
    #[prop]
    line_presets: &'a [LineSpacing],
    /// The font sizes the size menu offers, in points.
    #[prop]
    size_presets: &'a [f32],
    /// The range of the size field, in points.
    #[prop]
    size_range: RangeInclusive<f32>,
    /// The horizontal scales the scale menu offers, in percent.
    #[prop]
    scale_presets: &'a [f32],
    /// The range of the horizontal scale field, in percent.
    #[prop]
    scale_range: RangeInclusive<f32>,
    /// The character spacings the character spacing menu offers, in points.
    #[prop]
    char_spacing_presets: &'a [f32],
    /// The range of the character spacing field, in points.
    #[prop]
    char_spacing_range: RangeInclusive<f32>,
    /// The paragraph spacings the paragraph spacing menu offers, in points; none by default.
    #[prop]
    paragraph_spacing_presets: &'a [f32],
    /// The range of the paragraph spacing field, in points; the Paragraph dialog's by default.
    #[prop]
    paragraph_spacing_range: RangeInclusive<f32>,
    /// Controls the host cannot apply, each disabled with its reason.
    #[prop]
    unsupported: &'a [(FormatControl, Label)],
}

impl<'a> FormatText<'a> {
    /// A Format Text panel named `label` for the `character` and `paragraph` values, with the installed `families`.
    pub fn new(label: impl Into<Label>, character: &'a CharacterValues, paragraph: &'a ParagraphValues, families: &'a [String]) -> Self {
        FormatText {
            label: label.into(),
            character,
            paragraph,
            families,
            #[cfg(feature = "font-picker")]
            picker: None,
            line_spacing: None,
            line_presets: &LINE_PRESETS,
            size_presets: &super::SIZE_PRESETS,
            size_range: 0.1..=1296.0,
            scale_presets: &[],
            scale_range: 0.0..=1000.0,
            char_spacing_presets: &[],
            char_spacing_range: -1000.0..=10000.0,
            paragraph_spacing_presets: &[],
            paragraph_spacing_range: *SPACE_RANGE.start() as f32..=*SPACE_RANGE.end() as f32,
            unsupported: &[],
        }
    }

    /// Shows the panel and returns the changes the user made.
    pub fn show(self, ui: &mut Ui) -> FormatTextOutput {
        let (character, paragraph, families) = (self.character, self.paragraph, self.families);
        let (line_spacing, unsupported) = (self.line_spacing, self.unsupported);
        #[cfg(feature = "font-picker")]
        let mut picker = self.picker;
        let env = Env::of(ui);
        let words = env.words::<FormatTextWords>();
        let cw = env.words::<CharacterWords>();
        let pw = env.words::<ParagraphWords>();
        let dialog_words = env.words::<ParagraphDialogWords>();
        let units = env.units();
        let unit = DocUnit::points();
        let lengths = Lengths { units: &units, unit };
        let reason = |control: FormatControl| unsupported.iter().find(|(c, _)| *c == control).map(|(_, reason)| reason);

        let size_presets = length_menu(&units, unit, self.size_presets);
        let char_spacing_presets = length_menu(&units, unit, self.char_spacing_presets);
        let paragraph_spacing_presets = length_menu(&units, unit, self.paragraph_spacing_presets);
        let paragraph_spacing_range = wide(&self.paragraph_spacing_range);
        let scale_presets = number_presets(&units, std::iter::once(self.scale_presets));
        let (size_range, scale_range, char_spacing_range) = (wide(&self.size_range), wide(&self.scale_range), wide(&self.char_spacing_range));

        let mut spacings = self.line_presets.to_vec();
        if let Some(current) = line_spacing
            && !spacings.contains(&current)
        {
            spacings.push(current);
        }
        let line_entries: Vec<Entry<LineSpacing>> =
            spacings.into_iter().map(|spacing| Entry::new(spacing, line_label(spacing, &dialog_words, &units, unit))).collect();

        let aligns = [
            (TextAlign::Left, "align_left", &pw.left),
            (TextAlign::Center, "align_center", &pw.center),
            (TextAlign::Right, "align_right", &pw.right),
            (TextAlign::JustifyLeft, "align_justify", &pw.justify_left),
        ];
        let align_entries: Vec<Entry<TextAlign>> = aligns
            .iter()
            .map(|&(align, icon, name)| Entry::new(align, name.clone()).icon(IconId::new(icon)).checked(paragraph.align == Some(align)))
            .collect();
        let align_icon = match paragraph.align {
            Some(TextAlign::Center) => "align_center",
            Some(TextAlign::Right) => "align_right",
            Some(TextAlign::JustifyLeft | TextAlign::JustifyCenter | TextAlign::JustifyRight | TextAlign::JustifyAll) => "align_justify",
            Some(TextAlign::Left) | None => "align_left",
        };

        let gate = |toggle: Toggle<FormatTextChange>, control: FormatControl| match reason(control) {
            Some(why) => toggle.disabled(why),
            None => toggle,
        };
        let position = character.position;
        let toggles = [
            gate(
                Toggle::new(&words.bold, "text_bold", character.faux_bold, Box::new(|on| FormatTextChange::Character(CharacterChange::FauxBold(on)))),
                FormatControl::Bold,
            ),
            gate(
                Toggle::new(
                    &words.italic,
                    "text_italic",
                    character.faux_italic,
                    Box::new(|on| FormatTextChange::Character(CharacterChange::FauxItalic(on))),
                ),
                FormatControl::Italic,
            ),
            gate(
                Toggle::new(
                    &cw.underline,
                    "text_underline",
                    character.underline,
                    Box::new(|on| FormatTextChange::Character(CharacterChange::Underline(on))),
                ),
                FormatControl::Underline,
            ),
            gate(
                Toggle::new(
                    &cw.superscript,
                    "text_superscript",
                    position.map(|position| position == BaselinePosition::Superscript),
                    Box::new(|on| {
                        FormatTextChange::Character(CharacterChange::Position(if on {
                            BaselinePosition::Superscript
                        } else {
                            BaselinePosition::Normal
                        }))
                    }),
                ),
                FormatControl::Superscript,
            ),
            gate(
                Toggle::new(
                    &cw.subscript,
                    "text_subscript",
                    position.map(|position| position == BaselinePosition::Subscript),
                    Box::new(|on| {
                        FormatTextChange::Character(CharacterChange::Position(if on {
                            BaselinePosition::Subscript
                        } else {
                            BaselinePosition::Normal
                        }))
                    }),
                ),
                FormatControl::Subscript,
            ),
        ];
        let lists = [
            (ListKind::Bulleted, "list", &words.bulleted_list, FormatControl::BulletedList),
            (ListKind::Numbered, "list_numbered", &words.numbered_list, FormatControl::NumberedList),
        ];

        let mut edits: Vec<EditEvent<FormatTextChange>> = Vec::new();
        let mut requests = HostRequests::new();
        let character_edits = |events: Vec<EditEvent<CharacterChange>>| events.into_iter().map(|event| event.map(FormatTextChange::Character));

        let panel = Stack::new().label(self.label).show(ui, |ui, _| {
            // The font, across the panel.
            #[cfg(feature = "font-picker")]
            let family = family_field(ui, &cw, character, families, reason(FormatControl::Font), Some(Sizing::FillWidth), picker.as_mut());
            #[cfg(not(feature = "font-picker"))]
            let family = family_field(ui, &cw, character, families, reason(FormatControl::Font), Some(Sizing::FillWidth));
            edits.extend(character_edits(family.edits));
            if let Some(key) = family.host_picked {
                edits.push(picked_event(FormatTextChange::HostFont(key), cw.family.as_str()));
            }

            // Size and colour, the list buttons, the alignment menu.
            Row::new().gap(Gap::Tight).show(ui, |ui, _| {
                let size = LengthRow {
                    label: cw.size.clone(),
                    // Production's size field carries no glyph: the room goes to its value ("Mixed", "14 pt").
                    side: SideLabel::new().kind(LabelKind::Hidden).layout(FieldLayout::Beside),
                    value: character.size,
                    range: size_range.clone(),
                    presets: &size_presets,
                    disabled: None,
                };
                let size = match reason(FormatControl::Size) {
                    Some(why) => size.disabled(why),
                    None => size,
                };
                let (event, _) = lengths.row(ui, size);
                edits.extend(character_edits(number_change(event, CharacterChange::Size).into_iter().collect()));

                let color = mixed(character.color.map(Srgba::from));
                let side = SideLabel::new().kind(LabelKind::Hidden).layout(FieldLayout::Beside);
                let well = Field::new(cw.text_color.clone()).side(side).sizing(Sizing::Content).show(ui, |ui, _| {
                    let swatch = Swatch::new(cw.text_color.clone(), color).picker(COLOR_KEY);
                    disabled_for(swatch, &cw.text_color, reason(FormatControl::Color)).show(ui).pick_color
                });
                if let Some(request) = well.inner.flatten() {
                    requests.request_color(request);
                }
                Divider::vertical().show(ui);

                for &(kind, icon, name, control) in &lists {
                    let button = disabled_for(IconButton::new(IconId::new(icon)).label(name.clone()), name, reason(control)).show(ui);
                    if clicked(&button) {
                        edits.push(picked_event(FormatTextChange::List(kind), name.as_str()));
                    }
                }

                let opener = IconButton::new(IconId::new(align_icon)).label(pw.alignment.clone()).popup(PopupKind::Menu).popup_mark(PopupMark::Menu);
                let opener = disabled_for(opener, &pw.alignment, reason(FormatControl::Align)).show(ui).response;
                let picked = Menu::new(pw.alignment.clone()).values().opened_by(&opener, Trigger::Press).pick(ui, &align_entries);
                if let Some(align) = picked.picked {
                    edits.push(picked_event(FormatTextChange::Paragraph(ParagraphChange::Align(align)), pw.alignment.as_str()));
                }
            });

            // Bold, italic, underline, superscript, subscript.
            edits.extend(toggle_row(ui, None, &toggles));

            // Line and paragraph spacing.
            Grid::new(&TWO_COLUMNS).fill_width().show(ui, |ui, _| {
                let side =
                    SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::text("A↕"))).layout(FieldLayout::Beside).tooltip(words.line_spacing.clone());
                let picked = Field::new(words.line_spacing.clone()).side(side).show(ui, |ui, _| {
                    let select = Select::new(words.line_spacing.clone(), Bind::given(mixed(line_spacing)), &line_entries)
                        .placeholder("—")
                        .sizing(Sizing::FillWidth);
                    disabled_for(select, &words.line_spacing, reason(FormatControl::LineSpacing)).show(ui).events
                });
                edits.extend(picked.inner.unwrap_or_default().map(FormatTextChange::LineSpacing));

                // The space between paragraphs, which a paragraph keeps after itself.
                let spacing = LengthRow {
                    label: words.paragraph_spacing.clone(),
                    side: mark(GlyphLabel::text("¶↓"), None, FieldLayout::Inside),
                    value: paragraph.space_after,
                    range: paragraph_spacing_range.clone(),
                    presets: &paragraph_spacing_presets,
                    disabled: None,
                };
                let spacing = match reason(FormatControl::ParagraphSpacing) {
                    Some(why) => spacing.disabled(why),
                    None => spacing,
                };
                let (event, _) = lengths.row(ui, spacing);
                edits.extend(number_change(event, ParagraphChange::SpaceAfter).into_iter().map(|event| event.map(FormatTextChange::Paragraph)));
            });

            // Horizontal scale and character spacing.
            Grid::new(&TWO_COLUMNS).fill_width().show(ui, |ui, _| {
                let scale = PlainRow {
                    label: cw.horizontal_scale.clone(),
                    side: mark(GlyphLabel::text("↔T"), None, FieldLayout::Inside),
                    value: plain(character.horizontal_scale),
                    range: scale_range.clone(),
                    percent: true,
                    presets: &scale_presets,
                    disabled: None,
                };
                let scale = match reason(FormatControl::HorizontalScale) {
                    Some(why) => scale.disabled(why),
                    None => scale,
                };
                let (event, _) = lengths.plain(ui, scale);
                edits.extend(character_edits(number_change(event, CharacterChange::HorizontalScale).into_iter().collect()));

                let spacing = LengthRow {
                    label: words.char_spacing.clone(),
                    side: mark(GlyphLabel::text("VA"), None, FieldLayout::Inside),
                    value: character.char_spacing,
                    range: char_spacing_range.clone(),
                    presets: &char_spacing_presets,
                    disabled: None,
                };
                let spacing = match reason(FormatControl::CharSpacing) {
                    Some(why) => spacing.disabled(why),
                    None => spacing,
                };
                let (event, _) = lengths.row(ui, spacing);
                edits.extend(character_edits(number_change(event, CharacterChange::CharSpacing).into_iter().collect()));
            });
        });
        FormatTextOutput { response: panel.output, edits, requests }
    }
}

/// A range in points as the number fields take it.
fn wide(range: &RangeInclusive<f32>) -> RangeInclusive<f64> {
    f64::from(*range.start())..=f64::from(*range.end())
}

/// The line spacing menu's row of `spacing`: Auto, a multiple as a bare number ("1.15"), or a distance after the
/// paragraph dialog's word for it.
fn line_label(spacing: LineSpacing, dialog: &ParagraphDialogWords, units: &Units, unit: DocUnit) -> Label {
    let length = |points: f32| units.text(unit.kind, shown(units, unit, f64::from(points)), 0..=PRESET_DECIMALS);
    match spacing {
        LineSpacing::Auto => dialog.auto.clone(),
        LineSpacing::Multiple(times) => units.number(f64::from(times), 0..=PRESET_DECIMALS).into(),
        LineSpacing::Exactly(points) => format!("{} {}", dialog.exactly.as_str(), length(points)).into(),
        LineSpacing::AtLeast(points) => format!("{} {}", dialog.at_least.as_str(), length(points)).into(),
    }
}

#[cfg(feature = "suite")]
pub(crate) mod stories;
