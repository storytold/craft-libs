//! `TextStyleOptions`: the body of the Character Style and Paragraph Style options dialogs, a name and one include
//! check with a field per row.

use craft_behavior::{EditEvent, IconId, Mixed};
use craft_theme::Srgba;
use craft_ui::buttons::IconButton;
use craft_ui::choice::{Checkbox, Swatch};
use craft_ui::contract::{
    Bind, Entry, FieldLayout, GlyphLabel, NoValue, Outcome, Props as _, Show as _, ShowContainer as _, SideLabel, Sizing, clicked,
};
use craft_ui::display::Text;
use craft_ui::form::Field;
use craft_ui::layout_components::{Grid, Row, Stack};
use craft_ui::patterns::form_dialogs::HostRequests;
use craft_ui::runtime::env::{Env, Label, WordTemplate};
use craft_ui::text::TextField;
use egui::{Response, Ui};

use super::paragraph::{INDENT_RANGE, SPACE_RANGE};
use super::rows::{
    LengthRow, Lengths, PlainRow, Toggle, family_field, hidden_side, leading_menu, leading_row, mark, mixed, number_change, number_presets, plain,
    select_row, size_menu, string_entries, toggle_row,
};
use super::{
    CharacterChange, CharacterValues, CharacterWords, ParagraphChange, ParagraphValues, ParagraphWords, SHIFT_RANGE, SIZE_RANGE, TRACKING_PRESETS,
    TRACKING_RANGE, TWO_COLUMNS, TextAlign,
};
use crate::ui::length::DocUnit;

#[cfg(feature = "font-picker")]
use super::FontPickerSource;

#[cfg(feature = "suite")]
pub(crate) mod stories;

craft_ui::words! {
    /// The words of the style options editor, English by default: the three short alignment names its rows show. The
    /// name field, the include checks and the close button are words the contract row does not list but the editor
    /// needs. The other labels come from the Character and Paragraph words.
    #[non_exhaustive]
    pub struct TextStyleOptionsWords {
        left: "styleOptions.left" = "Left";
        center: "styleOptions.center" = "Center";
        right: "styleOptions.right" = "Right";
        name: "styleOptions.name" = "Style name";
        include: template "styleOptions.include" = "Include {attribute}";
        close: "styleOptions.close" = "Close";
    }
}

/// The key of the colour request the colour well makes.
pub const STYLE_COLOR_KEY: &str = "textStyle.color";

/// An attribute a style may define; the rows of the options editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StyleAttribute {
    /// Family.
    Font,
    /// Size.
    Size,
    /// Leading.
    Leading,
    /// Tracking.
    Tracking,
    /// Baseline shift.
    BaselineShift,
    /// Colour.
    Color,
    /// Faux bold.
    FauxBold,
    /// Faux italic.
    FauxItalic,
    /// Underline.
    Underline,
    /// Strikethrough.
    Strikethrough,
    /// Alignment.
    Align,
    /// First-line indent.
    FirstLineIndent,
    /// Start indent.
    StartIndent,
    /// End indent.
    EndIndent,
    /// Space before.
    SpaceBefore,
    /// Space after.
    SpaceAfter,
    /// Hyphenate.
    Hyphenate,
}

impl StyleAttribute {
    /// The rows of a character style.
    pub const CHARACTER: [StyleAttribute; 10] = [
        StyleAttribute::Font,
        StyleAttribute::Size,
        StyleAttribute::Leading,
        StyleAttribute::Tracking,
        StyleAttribute::BaselineShift,
        StyleAttribute::Color,
        StyleAttribute::FauxBold,
        StyleAttribute::FauxItalic,
        StyleAttribute::Underline,
        StyleAttribute::Strikethrough,
    ];

    /// The rows of a paragraph style.
    pub const PARAGRAPH: [StyleAttribute; 7] = [
        StyleAttribute::Align,
        StyleAttribute::FirstLineIndent,
        StyleAttribute::StartIndent,
        StyleAttribute::EndIndent,
        StyleAttribute::SpaceBefore,
        StyleAttribute::SpaceAfter,
        StyleAttribute::Hyphenate,
    ];
}

/// The values the options editor shows: the style's name, its attribute values, and which attributes it defines.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct StyleOptionValues {
    /// The style's name.
    pub name: String,
    /// The character attributes (a paragraph style's character part too).
    pub character: CharacterValues,
    /// The paragraph attributes; `None` for a character style.
    pub paragraph: Option<ParagraphValues>,
    /// The attributes the style defines; the others are inherited.
    pub included: Vec<StyleAttribute>,
}

impl StyleOptionValues {
    /// A style named `name` with `character` values, no paragraph part and no attribute included.
    pub fn new(name: impl Into<String>, character: CharacterValues) -> Self {
        StyleOptionValues { name: name.into(), character, paragraph: None, included: Vec::new() }
    }

    /// Whether the style defines `attribute`.
    pub fn includes(&self, attribute: StyleAttribute) -> bool {
        self.included.contains(&attribute)
    }
}

/// One change from the options editor.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum StyleOptionsChange {
    /// The style's new name.
    Rename(String),
    /// An include check switched.
    #[non_exhaustive]
    Include {
        /// The attribute.
        attribute: StyleAttribute,
        /// Whether the style defines it now.
        on: bool,
    },
    /// A character field.
    Character(CharacterChange),
    /// A paragraph field.
    Paragraph(ParagraphChange),
}

impl StyleOptionsChange {
    /// The include check of `attribute` switched to `on`.
    pub fn include(attribute: StyleAttribute, on: bool) -> Self {
        StyleOptionsChange::Include { attribute, on }
    }
}

/// What the options editor did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct TextStyleOptionsOutput {
    /// The editor's response.
    pub response: Response,
    /// The edit events of this pass.
    pub edits: Vec<EditEvent<StyleOptionsChange>>,
    /// The colour well was clicked (key [`STYLE_COLOR_KEY`]).
    pub requests: HostRequests,
    /// The editor asks to close.
    pub closed: bool,
}

impl Outcome for TextStyleOptionsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The alignment select's rows: each alignment and the key its entry carries.
const ALIGNS: [(TextAlign, &str); 7] = [
    (TextAlign::Left, "left"),
    (TextAlign::Center, "center"),
    (TextAlign::Right, "right"),
    (TextAlign::JustifyLeft, "justify-left"),
    (TextAlign::JustifyCenter, "justify-center"),
    (TextAlign::JustifyRight, "justify-right"),
    (TextAlign::JustifyAll, "justify-all"),
];

/// The options of a character or paragraph style: a name field, then one include check and field per row of `rows`.
/// A row that is not included shows the inherited value and takes no input; every length row is in the type unit.
#[must_use = "show the editor with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct TextStyleOptions<'a> {
    label: Label,
    values: &'a StyleOptionValues,
    rows: &'a [StyleAttribute],
    families: &'a [String],
    styles: &'a [String],
    /// The unit the length fields show. Default points.
    #[prop]
    unit: DocUnit,
    /// The name field takes input.
    #[prop]
    renamable: bool,
    /// The family field is craft-font-manager's font picker over `source`.
    #[cfg(feature = "font-picker")]
    #[prop(name = "font_picker")]
    picker: Option<FontPickerSource<'a>>,
}

impl<'a> TextStyleOptions<'a> {
    /// The editor named `label` over `values`, with the `rows` to show, the installed `families` and the family's
    /// `styles`.
    pub fn new(
        label: impl Into<Label>,
        values: &'a StyleOptionValues,
        rows: &'a [StyleAttribute],
        families: &'a [String],
        styles: &'a [String],
    ) -> Self {
        TextStyleOptions {
            label: label.into(),
            values,
            rows,
            families,
            styles,
            unit: DocUnit::points(),
            renamable: true,
            #[cfg(feature = "font-picker")]
            picker: None,
        }
    }

    /// Shows the editor and returns the changes the user made.
    pub fn show(self, ui: &mut Ui) -> TextStyleOptionsOutput {
        let (label, values, rows) = (self.label, self.values, self.rows);
        let (families, styles, unit, renamable) = (self.families, self.styles, self.unit, self.renamable);
        #[cfg(feature = "font-picker")]
        let mut picker = self.picker;
        let env = Env::of(ui);
        let words = env.words::<TextStyleOptionsWords>();
        let cw = env.words::<CharacterWords>();
        let pw = env.words::<ParagraphWords>();
        let units = env.units();
        let lengths = Lengths { units: &units, unit };

        let size_presets = size_menu(&units, unit);
        let leading_presets = leading_menu(&cw, values.character.leading, &size_presets);
        let tracking_presets = number_presets(&units, TRACKING_PRESETS.iter().copied());

        let style_entries = string_entries(styles.iter().map(|name| (name.clone(), name.clone())));
        let align_entries: Vec<Entry<String>> = ALIGNS
            .iter()
            .map(|&(align, key)| {
                let name = match align {
                    TextAlign::Left => &words.left,
                    TextAlign::Center => &words.center,
                    TextAlign::Right => &words.right,
                    TextAlign::JustifyLeft => &pw.justify_left,
                    TextAlign::JustifyCenter => &pw.justify_center,
                    TextAlign::JustifyRight => &pw.justify_right,
                    TextAlign::JustifyAll => &pw.justify_all,
                };
                Entry::new(key.to_owned(), name.as_str().to_owned())
            })
            .collect();
        let has_paragraph = values.paragraph.is_some();
        let paragraph = values.paragraph.clone().unwrap_or_default();
        let c = &values.character;

        let mut edits: Vec<EditEvent<StyleOptionsChange>> = Vec::new();
        let mut requests = HostRequests::new();
        let mut closed = false;

        let shown = Stack::new().label(label.clone()).show(ui, |ui, _| {
            Row::new().fill_width().show(ui, |ui, _| {
                Text::new(label.clone()).show(ui);
                let close = IconButton::new(IconId::new("close")).label(words.close.clone()).show(ui);
                closed = clicked(&close);
            });
            let renamed = Field::new(words.name.clone())
                .show(ui, |ui, _| TextField::new(words.name.clone(), Bind::given(values.name.clone())).enabled(renamable).show(ui).events);
            edits.extend(renamed.inner.unwrap_or_default().map(StyleOptionsChange::Rename));

            for &attribute in rows {
                if !has_paragraph && StyleAttribute::PARAGRAPH.contains(&attribute) {
                    continue;
                }
                let on = values.includes(attribute);
                let found = match attribute {
                    StyleAttribute::Font => include_row(ui, &words.include, attribute, on, |ui| {
                        #[cfg(feature = "font-picker")]
                        let family = family_field(ui, &cw, c, families, None, Some(Sizing::FillWidth), picker.as_mut());
                        #[cfg(not(feature = "font-picker"))]
                        let family = family_field(ui, &cw, c, families, None, Some(Sizing::FillWidth));
                        let mut found = character(family.edits);
                        found.extend(character(select_row(
                            ui,
                            &cw.style,
                            hidden_side(),
                            mixed(c.style.clone()),
                            &style_entries,
                            Some(Sizing::FillWidth),
                            CharacterChange::Style,
                        )));
                        found
                    }),
                    StyleAttribute::Size => include_row(ui, &words.include, attribute, on, |ui| {
                        let row = LengthRow {
                            label: cw.size.clone(),
                            side: text_side(&cw.size),
                            value: c.size,
                            range: SIZE_RANGE,
                            presets: &size_presets,
                            disabled: None,
                        };
                        let (event, _) = lengths.row(ui, row);
                        character(number_change(event, CharacterChange::Size))
                    }),
                    StyleAttribute::Leading => include_row(ui, &words.include, attribute, on, |ui| {
                        character(leading_row(&lengths, ui, &cw, text_side(&cw.leading), c.leading, c.size, &leading_presets))
                    }),
                    StyleAttribute::Tracking => include_row(ui, &words.include, attribute, on, |ui| {
                        let row = PlainRow {
                            label: cw.tracking.clone(),
                            side: text_side(&cw.tracking),
                            value: plain(c.tracking),
                            range: TRACKING_RANGE,
                            percent: false,
                            presets: &tracking_presets,
                            disabled: None,
                        };
                        let (event, _) = lengths.plain(ui, row);
                        character(number_change(event, CharacterChange::Tracking))
                    }),
                    StyleAttribute::BaselineShift => include_row(ui, &words.include, attribute, on, |ui| {
                        let row = LengthRow {
                            label: cw.baseline_shift.clone(),
                            side: text_side(&cw.baseline_shift),
                            value: c.baseline_shift,
                            range: SHIFT_RANGE,
                            presets: &[],
                            disabled: None,
                        };
                        let (event, _) = lengths.row(ui, row);
                        character(number_change(event, CharacterChange::BaselineShift))
                    }),
                    StyleAttribute::Color => include_row(ui, &words.include, attribute, on, |ui| {
                        let color = mixed(c.color.map(Srgba::from));
                        let well = Field::new(cw.text_color.clone()).side(hidden_side()).show(ui, |ui, _| {
                            Swatch::new(cw.text_color.clone(), color).picker(STYLE_COLOR_KEY).sizing(Sizing::FillWidth).show(ui).pick_color
                        });
                        if let Some(request) = well.inner.flatten() {
                            requests.request_color(request);
                        }
                        Vec::new()
                    }),
                    StyleAttribute::FauxBold => include_row(ui, &words.include, attribute, on, |ui| {
                        toggle_field(ui, &cw.faux_bold, "text_bold", c.faux_bold, CharacterChange::FauxBold)
                    }),
                    StyleAttribute::FauxItalic => include_row(ui, &words.include, attribute, on, |ui| {
                        toggle_field(ui, &cw.faux_italic, "text_italic", c.faux_italic, CharacterChange::FauxItalic)
                    }),
                    StyleAttribute::Underline => include_row(ui, &words.include, attribute, on, |ui| {
                        toggle_field(ui, &cw.underline, "text_underline", c.underline, CharacterChange::Underline)
                    }),
                    StyleAttribute::Strikethrough => include_row(ui, &words.include, attribute, on, |ui| {
                        toggle_field(ui, &cw.strikethrough, "text_strikethrough", c.strikethrough, CharacterChange::Strikethrough)
                    }),
                    StyleAttribute::Align => include_row(ui, &words.include, attribute, on, |ui| {
                        let key = paragraph.align.and_then(|align| ALIGNS.iter().find(|(a, _)| *a == align)).map(|(_, key)| (*key).to_owned());
                        // The entries come from ALIGNS, so every key is found.
                        let align_of = |key: String| {
                            ParagraphChange::Align(ALIGNS.iter().find(|(_, k)| *k == key).map_or_else(TextAlign::default, |(align, _)| *align))
                        };
                        paragraph_changes(select_row(ui, &pw.alignment, hidden_side(), mixed(key), &align_entries, Some(Sizing::FillWidth), align_of))
                    }),
                    StyleAttribute::FirstLineIndent => include_row(ui, &words.include, attribute, on, |ui| {
                        let row = LengthRow {
                            label: pw.first_line_indent.clone(),
                            side: text_side(&pw.first_line_indent),
                            value: paragraph.first_line_indent,
                            range: INDENT_RANGE,
                            presets: &[],
                            disabled: None,
                        };
                        let (event, _) = lengths.row(ui, row);
                        paragraph_changes(number_change(event, ParagraphChange::FirstLineIndent))
                    }),
                    StyleAttribute::StartIndent => include_row(ui, &words.include, attribute, on, |ui| {
                        let row = LengthRow {
                            label: pw.start_indent.clone(),
                            side: text_side(&pw.start_indent),
                            value: paragraph.start_indent,
                            range: INDENT_RANGE,
                            presets: &[],
                            disabled: None,
                        };
                        let (event, _) = lengths.row(ui, row);
                        paragraph_changes(number_change(event, ParagraphChange::StartIndent))
                    }),
                    StyleAttribute::EndIndent => include_row(ui, &words.include, attribute, on, |ui| {
                        let row = LengthRow {
                            label: pw.end_indent.clone(),
                            side: text_side(&pw.end_indent),
                            value: paragraph.end_indent,
                            range: INDENT_RANGE,
                            presets: &[],
                            disabled: None,
                        };
                        let (event, _) = lengths.row(ui, row);
                        paragraph_changes(number_change(event, ParagraphChange::EndIndent))
                    }),
                    StyleAttribute::SpaceBefore => include_row(ui, &words.include, attribute, on, |ui| {
                        let row = LengthRow {
                            label: pw.space_before.clone(),
                            side: text_side(&pw.space_before),
                            value: paragraph.space_before,
                            range: SPACE_RANGE,
                            presets: &[],
                            disabled: None,
                        };
                        let (event, _) = lengths.row(ui, row);
                        paragraph_changes(number_change(event, ParagraphChange::SpaceBefore))
                    }),
                    StyleAttribute::SpaceAfter => include_row(ui, &words.include, attribute, on, |ui| {
                        let row = LengthRow {
                            label: pw.space_after.clone(),
                            side: text_side(&pw.space_after),
                            value: paragraph.space_after,
                            range: SPACE_RANGE,
                            presets: &[],
                            disabled: None,
                        };
                        let (event, _) = lengths.row(ui, row);
                        paragraph_changes(number_change(event, ParagraphChange::SpaceAfter))
                    }),
                    StyleAttribute::Hyphenate => include_row(ui, &words.include, attribute, on, |ui| {
                        let out = Checkbox::new(Bind::given(mixed(paragraph.hyphenate))).label(pw.hyphenate.clone()).show(ui);
                        paragraph_changes(out.events.map(ParagraphChange::Hyphenate))
                    }),
                };
                edits.extend(found);
            }
        });
        TextStyleOptionsOutput { response: shown.output, edits, requests, closed }
    }
}

/// A row's side label: its name as text beside the field.
fn text_side(label: &Label) -> SideLabel {
    mark(GlyphLabel::text(label.clone()), None, FieldLayout::Beside)
}

/// The name of the row of `attribute`, which the include check's accessible name carries.
fn attribute_name(attribute: StyleAttribute, cw: &CharacterWords, pw: &ParagraphWords) -> Label {
    match attribute {
        StyleAttribute::Font => cw.family.clone(),
        StyleAttribute::Size => cw.size.clone(),
        StyleAttribute::Leading => cw.leading.clone(),
        StyleAttribute::Tracking => cw.tracking.clone(),
        StyleAttribute::BaselineShift => cw.baseline_shift.clone(),
        StyleAttribute::Color => cw.text_color.clone(),
        StyleAttribute::FauxBold => cw.faux_bold.clone(),
        StyleAttribute::FauxItalic => cw.faux_italic.clone(),
        StyleAttribute::Underline => cw.underline.clone(),
        StyleAttribute::Strikethrough => cw.strikethrough.clone(),
        StyleAttribute::Align => pw.alignment.clone(),
        StyleAttribute::FirstLineIndent => pw.first_line_indent.clone(),
        StyleAttribute::StartIndent => pw.start_indent.clone(),
        StyleAttribute::EndIndent => pw.end_indent.clone(),
        StyleAttribute::SpaceBefore => pw.space_before.clone(),
        StyleAttribute::SpaceAfter => pw.space_after.clone(),
        StyleAttribute::Hyphenate => pw.hyphenate.clone(),
    }
}

/// One row: the include check, named by the `include` template filled with the row's name, then the row's `field`. A
/// row that is not included shows the inherited value and takes no input; its check still does.
fn include_row(
    ui: &mut Ui,
    include: &WordTemplate,
    attribute: StyleAttribute,
    on: bool,
    mut field: impl FnMut(&mut Ui) -> Vec<EditEvent<StyleOptionsChange>>,
) -> Vec<EditEvent<StyleOptionsChange>> {
    let env = Env::of(ui);
    let name = attribute_name(attribute, &env.words::<CharacterWords>(), &env.words::<ParagraphWords>());
    let include = Label::from(include.fill(&[("attribute", name.as_str())]));
    let mut found = Vec::new();
    Grid::new(&TWO_COLUMNS).fill_width().show(ui, |ui, _| {
        let check = Field::new(include.clone()).side(hidden_side()).show(ui, |ui, _| Checkbox::new(Bind::given(Mixed::Same(on))).show(ui).events);
        found.extend(check.inner.unwrap_or_default().map(move |on| StyleOptionsChange::Include { attribute, on }));
        Stack::new().enabled(on).show(ui, |ui, _| found.extend(field(ui)));
    });
    found
}

/// A single toggle button as a row's field.
fn toggle_field(
    ui: &mut Ui,
    name: &Label,
    icon: &'static str,
    on: Option<bool>,
    change: fn(bool) -> CharacterChange,
) -> Vec<EditEvent<StyleOptionsChange>> {
    character(toggle_row(ui, None, &[Toggle::new(name, icon, on, Box::new(change))]))
}

/// Character edits as the editor's changes.
fn character(events: impl IntoIterator<Item = EditEvent<CharacterChange>>) -> Vec<EditEvent<StyleOptionsChange>> {
    events.into_iter().map(|event| event.map(StyleOptionsChange::Character)).collect()
}

/// Paragraph edits as the editor's changes.
fn paragraph_changes(events: impl IntoIterator<Item = EditEvent<ParagraphChange>>) -> Vec<EditEvent<StyleOptionsChange>> {
    events.into_iter().map(|event| event.map(StyleOptionsChange::Paragraph)).collect()
}
