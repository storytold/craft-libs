//! `PageRangeField`: Acrobat's Pages selected field, a text field taking "1-3, 5" with a menu button of the app's
//! page choices, showing the pages back in the same format; and `PageRangeOptions`, the All pages / Pages from-to / Subset rows of
//! Acrobat's Page Range Options.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use craft_behavior::{Codec, NotAValue, NumberCodec};
use craft_ui::buttons::IconButton;
use craft_ui::choice::{Radio, RadioGroup};
use craft_ui::contract::{
    Bind, EditResponse, Entry, FieldLayout, LabelKind, LabelScheme, NoValue, Outcome, Pattern, Props as _, Show as _, ShowContainer as _, SideLabel,
    Trigger, Validity,
};
use craft_ui::display::Text;
use craft_ui::form::Field;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::menu::Menu;
use craft_ui::patterns::form_dialogs::{DialogFields, FieldValue, Param};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label, Message, Units};
use craft_ui::text::{NumberField, TextField};
use egui::{Response, Ui};

use crate::range::{PageRange, PageRangeError, PageScope, Subset, format_pages, parse_pages};

craft_ui::words! {
    /// The words of the page range field.
    pub struct PageRangeFieldWords {
        selected_pages: "pageRangeField.selectedPages" = "Selected pages";
        select_pages: "pageRangeField.selectPages" = "Select pages";
        /// `{page}` is the text that is not a page.
        not_a_page: template "pageRangeField.notAPage" = "\"{page}\" is not a page number or label";
        /// `{n}` is the page asked for, `{count}` the number of pages.
        out_of_range: template "pageRangeField.outOfRange" = "page {n} is out of range (1\u{2013}{count})";
        too_long: "pageRangeField.tooLong" = "Too many pages";
    }
}

craft_ui::words! {
    /// The words of the page range options.
    pub struct PageRangeOptionsWords {
        page_range: "pageRangeOptions.pageRange" = "Page range";
        all_pages: "pageRangeOptions.allPages" = "All pages";
        /// `{page}` is the current page's number.
        current_page: template "pageRangeOptions.currentPage" = "Current page ({page})";
        selection: "pageRangeOptions.selection" = "Selected pages";
        pages_from: "pageRangeOptions.pagesFrom" = "Pages from";
        to: "pageRangeOptions.to" = "to";
        /// After the last page field of a document dialog; `{count}` is the number of pages.
        of_count: template "pageRangeOptions.ofCount" = "of {count}";
        subset: "pageRangeOptions.subset" = "Subset";
        even_and_odd: "pageRangeOptions.evenAndOdd" = "Even and Odd Pages";
        all_in_range: "pageRangeOptions.allInRange" = "All pages in range";
        even_only: "pageRangeOptions.evenOnly" = "Even pages only";
        odd_only: "pageRangeOptions.oddOnly" = "Odd pages only";
    }
}

impl PageRangeFieldWords {
    /// The message for `error`, with the numbers in the user's digits.
    pub fn error(&self, error: &PageRangeError, units: &Units) -> Label {
        let whole = |n: usize| units.number(n as f64, 0..=0);
        match error {
            PageRangeError::NotAPage(token) => self.not_a_page.fill(&[("page", token.as_str())]).into(),
            PageRangeError::OutOfRange { page, count } => {
                self.out_of_range.fill(&[("n", whole(*page).as_str()), ("count", whole(*count).as_str())]).into()
            }
            PageRangeError::TooLong => self.too_long.clone(),
        }
    }
}

/// The pages a field's text stands for, or why it stands for none: what [`PageRangeCodec`] reads text into, so the
/// field's check can refuse the text with the error's message.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct PageSelection {
    /// The pages (0-based) the text names; empty while `error` is set.
    pub pages: Vec<usize>,
    /// Why the text names no pages.
    pub error: Option<PageRangeError>,
}

impl PageSelection {
    /// The selection of `pages`, which no error refuses.
    pub fn of(pages: &[usize]) -> Self {
        PageSelection { pages: pages.to_vec(), error: None }
    }
}

/// The text form of a page selection in a document of `count` pages: "1-3, 5" (1-based).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageRangeCodec {
    count: usize,
}

impl PageRangeCodec {
    /// A codec for a document of `count` pages.
    pub const fn new(count: usize) -> Self {
        PageRangeCodec { count }
    }
}

impl Codec for PageRangeCodec {
    type Value = PageSelection;

    fn format(&self, v: &PageSelection) -> String {
        format_pages(&v.pages)
    }

    fn parse(&self, text: &str, _: &PageSelection) -> Result<PageSelection, NotAValue> {
        Ok(match parse_pages(text, self.count) {
            Ok(pages) => PageSelection::of(&pages),
            Err(error) => PageSelection { error: Some(error), ..PageSelection::default() },
        })
    }
}

/// What the page range field reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PageRangeFieldOutput {
    /// The text field's response.
    pub response: Response,
    /// The pages (0-based) the user typed or picked in this pass.
    pub picked: Option<Vec<usize>>,
}

impl Outcome for PageRangeFieldOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A text field over page numbers with a menu button of the app's page choices.
///
/// - The field shows `pages` formatted ([`format_pages`]); with none, the current page's number is its placeholder.
/// - Typed text applies on Enter or when the field loses focus: text that reads reports the pages and shows them back
///   formatted; text that does not stays, with the error under the field. Escape puts the pages' text back.
/// - The button opens the app's entries ([`menu`](PageRangeField::menu)), each naming its pages; Down opens it from
///   the button. A pick reports like typed pages. A field given no entries has no button.
#[must_use = "show the field with `show`"]
#[derive(Pattern)]
pub struct PageRangeField<'a> {
    label: Label,
    pages: &'a [usize],
    count: usize,
    current: usize,
    menu: &'a [Entry<Vec<usize>>],
}

impl<'a> PageRangeField<'a> {
    /// The field named `label` over `pages` (0-based) in a document of `count` pages, `current` (0-based) the page
    /// the placeholder names.
    pub fn new(label: impl Into<Label>, pages: &'a [usize], count: usize, current: usize) -> Self {
        PageRangeField { label: label.into(), pages, count, current, menu: &[] }
    }

    /// The entries of the menu button, each with the pages (0-based) it picks (All pages, Odd pages only).
    pub fn menu(mut self, entries: &'a [Entry<Vec<usize>>]) -> Self {
        self.menu = entries;
        self
    }

    /// Shows the field.
    pub fn show(self, ui: &mut Ui) -> PageRangeFieldOutput {
        let PageRangeField { label, pages, count, current, menu } = self;
        let env = Env::of(ui);
        let words = env.words::<PageRangeFieldWords>();
        let units = env.units();
        let placeholder = units.number((current.min(count.saturating_sub(1)) + 1) as f64, 0..=0);
        let mut picked = None;
        let mut response = None;
        Field::new(label).side(SideLabel::new().kind(LabelKind::Hidden)).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                let value =
                    Bind::given(PageSelection::of(pages)).check_with((words.clone(), units.clone()), |(words, units), value| match &value.error {
                        Some(error) => Validity::Refuse(Message::new(words.error(error, units))),
                        None => Validity::Ok,
                    });
                let field = TextField::with_codec(words.selected_pages.clone(), value, PageRangeCodec::new(count))
                    .placeholder(Label::from(placeholder.clone()))
                    .show(ui);
                if let Some(typed) = field.value().filter(|typed| typed.error.is_none()) {
                    picked = Some(typed.pages.clone());
                }
                response = Some(field.response.clone());
                if menu.is_empty() {
                    return;
                }
                let button =
                    IconButton::new(craft_ui::controls::disclosure::DisclosureGlyph::menu_button()).label(words.select_pages.clone()).show(ui);
                let menu = Menu::new(words.select_pages.clone()).opened_by(&button, Trigger::Press).pick(ui, menu);
                if let Some(pages) = menu.picked {
                    picked = Some(pages);
                }
            });
        });
        PageRangeFieldOutput { response: response.unwrap_or_else(|| ui.response()), picked }
    }
}

/// What the page range options report.
#[derive(Debug)]
#[non_exhaustive]
pub struct PageRangeOptionsOutput {
    /// The response of the rows.
    pub response: Response,
    /// The range after a change in this pass.
    pub changed: Option<PageRange>,
}

impl Outcome for PageRangeOptionsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The All pages / Pages from [ ] to [ ] / Subset [ ] rows over a dialog's [`PageRange`]: a radio group over the
/// range's [`PageScope`], whole-number fields for From and To (1 to the page count, on only for the from-to range) and
/// a list for the subset. [`PageRangeChoices`] adds Current page and Selected pages, or leaves out the subset.
#[must_use = "show the options with `show`"]
#[derive(Pattern)]
pub struct PageRangeOptions {
    label: Label,
    range: PageRange,
    count: usize,
    choices: PageRangeChoices,
    current: usize,
}

/// A named arrangement of the page-range rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PageRangeChoices {
    /// All pages, From and To, and the Subset.
    #[default]
    Acrobat,
    /// Acrobat's rows with Current page; it sets the range's scope to `PageScope::Current`.
    Current,
    /// Acrobat's rows with Current page and Selected pages.
    Selection,
    /// All pages and From and To, with no Subset.
    FromTo,
    /// All pages, Current page and From and To, with no Subset.
    CurrentFromTo,
    /// All pages, Selected pages and From and To, with no Subset (its own row, [`PageRangeOptions::subset_select`],
    /// stands elsewhere in the dialog).
    SelectedFromTo,
    /// Acrobat's rows on one line, as production's Background and Watermark dialogs show them: All pages, Pages from
    /// and to, then the Subset, with no page count after the range.
    Line,
}

impl PageRangeChoices {
    /// Whether the rows offer Current page.
    fn current(self) -> bool {
        matches!(self, PageRangeChoices::Current | PageRangeChoices::Selection | PageRangeChoices::CurrentFromTo)
    }

    /// Whether the rows offer Selected pages.
    fn selection(self) -> bool {
        matches!(self, PageRangeChoices::Selection | PageRangeChoices::SelectedFromTo)
    }

    /// Whether the rows end with the Subset list.
    fn subset(self) -> bool {
        matches!(self, PageRangeChoices::Acrobat | PageRangeChoices::Current | PageRangeChoices::Selection | PageRangeChoices::Line)
    }

    /// Whether every option stands on one line.
    fn line(self) -> bool {
        self == PageRangeChoices::Line
    }
}

impl PageRangeOptions {
    /// The options named `label` over `range` in a document of `count` pages.
    pub fn new(label: impl Into<Label>, range: PageRange, count: usize) -> Self {
        PageRangeOptions { label: label.into(), range, count, choices: PageRangeChoices::Acrobat, current: 0 }
    }

    /// The arrangement of the rows.
    pub fn choices(mut self, choices: PageRangeChoices) -> Self {
        self.choices = choices;
        self
    }

    /// The current page (0-based), which the Current page row names.
    pub fn current(mut self, current: usize) -> Self {
        self.current = current;
        self
    }

    /// The one page-range row for every `ParamDialog`: a custom row named `label` that keeps its [`PageRange`] under
    /// `key` as JSON, in a document of `count` pages with `current` the current page.
    ///
    /// The row opens on All pages. A value under `key` that is not a range shows as All pages until the user picks.
    pub fn param(key: impl Into<String>, label: impl Into<Label>, count: usize, current: usize, choices: PageRangeChoices) -> Param {
        let key = key.into();
        let label = label.into();
        let (row_key, row_label) = (key.clone(), label.clone());
        Param::custom(key, label, move |ui, fields, _| {
            let range = Self::value(fields, &row_key).unwrap_or_default();
            let shown = PageRangeOptions::new(row_label.clone(), range, count).choices(choices).current(current).show(ui);
            if let Some(next) = shown.changed {
                fields.set(row_key.clone(), Self::field_value(&next));
            }
            shown.response
        })
        .opens_with(Self::field_value(&PageRange::default()))
    }

    /// The range the row `key` holds in `fields`; `None` when absent or not a range.
    pub fn value(fields: &DialogFields, key: &str) -> Option<PageRange> {
        match fields.get(key)? {
            FieldValue::Json(value) => serde_json::from_value(value.clone()).ok(),
            _ => None,
        }
    }

    /// The Subset list alone, for a dialog row of its own (production's Rotate puts it first under "Rotate:"): it edits
    /// the subset of the range kept under `key` (see [`param`](Self::param)).
    pub fn subset_select(ui: &mut Ui, fields: &mut DialogFields, key: &str) -> Response {
        let words = Env::of(ui).words::<PageRangeOptionsWords>();
        let range = Self::value(fields, key).unwrap_or_default();
        let entries = [
            Entry::new(Subset::All, words.even_and_odd.clone()),
            Entry::new(Subset::Even, words.even_only.clone()),
            Entry::new(Subset::Odd, words.odd_only.clone()),
        ];
        let picked = Select::new(words.subset.clone(), Bind::given(range.subset), &entries).show(ui);
        if let Some(chosen) = picked.value().copied() {
            fields.set(key.to_owned(), Self::field_value(&PageRange { subset: chosen, ..range }));
        }
        picked.response
    }

    /// The row's stored form of `range`, for the row's `opens_with`.
    pub fn field_value(range: &PageRange) -> FieldValue {
        // A `PageRange` is plain fields and enums, which always serialize; `Null` would read back as no range.
        FieldValue::Json(serde_json::to_value(range).unwrap_or_default())
    }

    /// Shows the options.
    pub fn show(self, ui: &mut Ui) -> PageRangeOptionsOutput {
        let PageRangeOptions { label, range, count, choices, current } = self;
        let env = Env::of(ui);
        let words = env.words::<PageRangeOptionsWords>();
        let numbers = NumberCodec::new(env.units().reader(None), 0);
        let last = count.max(1);
        let shown_from = range.from.clamp(1, last);
        let shown_to = range.to.map_or(last, |to| to.clamp(1, last));
        let mut next = range;
        let after = SideLabel::new().layout(FieldLayout::After);
        // A page number past the document takes the nearest page, as Photoshop and Acrobat do; `whole` is the one place
        // that holds it, so the field has no range of its own.
        let whole = |value: &EditResponse<Option<f64>>, shown: usize| {
            value.value().copied().flatten().map(|v| (v.round() as usize).clamp(1, last)).filter(|v| *v != shown)
        };
        let page = env.format().integer(current.saturating_add(1) as u64);
        let current_page = Label::from(words.current_page.fill(&[("page", page.as_str())]));
        let mut from = None;
        let mut to = None;
        // A document dialog (`LabelScheme::Inline`) shows the options as production does: one wrapping line, the subset
        // at its end. Elsewhere the radio buttons stack and the subset is a row of its own.
        let inline = env.view().get::<LabelScheme>() == LabelScheme::Inline;
        let entries = [
            Entry::new(Subset::All, words.all_in_range.clone()),
            Entry::new(Subset::Even, words.even_only.clone()),
            Entry::new(Subset::Odd, words.odd_only.clone()),
        ];
        let mut subset = None;
        // One line ([`PageRangeChoices::Line`]): the subset ends the line in a document dialog and elsewhere, and the
        // range shows no page count.
        let line = choices.line();
        let subset_inline = inline || line;
        let mut pick_subset = |ui: &mut Ui| {
            let picked = Select::new(words.subset.clone(), Bind::given(range.subset), &entries).show(ui);
            if let Some(chosen) = picked.value().copied() {
                subset = Some(chosen);
            }
        };
        let group = RadioGroup::new(label, Bind::given(range.scope)).show(ui, |ui, cx| {
            let mut body = |ui: &mut Ui| {
                cx.labelled(ui, "all", words.all_pages.clone(), after.clone(), Radio::new(PageScope::All));
                if choices.current() {
                    cx.labelled(ui, "current", current_page.clone(), after.clone(), Radio::new(PageScope::Current));
                }
                if choices.selection() {
                    cx.labelled(ui, "selection", words.selection.clone(), after.clone(), Radio::new(PageScope::Selection));
                }
                // Two themed fields don't fit beside the radio in a narrow panel: "to" and its field wrap together. One
                // line keeps them on it, as production's does: the dialog takes the line's width.
                Row::new().wrap(!line).show(ui, |ui, _| {
                    cx.labelled(ui, "range", words.pages_from.clone(), after.clone(), Radio::new(PageScope::Range));
                    let field = |label: &Label, shown: usize| {
                        NumberField::new(label.clone(), Bind::given(Some(shown as f64)))
                            .codec(numbers.clone())
                            .enabled(range.scope == PageScope::Range)
                    };
                    from = whole(&field(&words.pages_from, shown_from).show(ui).edit, shown_from);
                    Row::new().show(ui, |ui, _| {
                        Text::new(words.to.clone()).show(ui);
                        to = whole(&field(&words.to, shown_to).show(ui).edit, shown_to);
                        if inline && !line {
                            let count = env.format().integer(last as u64);
                            Text::new(words.of_count.fill(&[("count", count.as_str())])).show(ui);
                        }
                    });
                    if subset_inline && choices.subset() {
                        Row::new().show(ui, |ui, _| {
                            Text::new(words.subset.clone()).show(ui);
                            pick_subset(ui);
                        });
                    }
                });
            };
            // The radio buttons stack, in a document dialog as elsewhere: production's Pages group is a column.
            if line {
                Row::new().show(ui, |ui, _| body(ui));
            } else {
                Stack::new().show(ui, |ui, _| body(ui));
            }
        });
        if let Some(scope) = group.output.value().copied() {
            next.scope = scope;
        }
        if let Some(value) = from {
            next.from = value;
            next.to = next.to.map(|to| to.max(value));
        }
        if let Some(value) = to {
            next.to = Some(value);
        }
        if choices.subset() && !subset_inline {
            Field::new(words.subset.clone()).show(ui, |ui, _| pick_subset(ui));
        }
        if let Some(chosen) = subset {
            next.subset = chosen;
        }
        PageRangeOptionsOutput { response: group.output.response.clone(), changed: (next != range).then_some(next) }
    }
}
