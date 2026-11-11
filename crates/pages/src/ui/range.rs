//! `PageRangeField`: Acrobat's Pages selected field, a text field taking "1-3, 5" with a menu button of page presets,
//! showing the pages back in the same format; and `PageRangeOptions`, the All pages / Pages from-to / Subset rows of
//! Acrobat's Page Range Options.

#[cfg(feature = "screen-testing")]
mod stories;

use craft_behavior::{Codec, NotAValue, NumberCodec};
use craft_ui::buttons::IconButton;
use craft_ui::choice::{Radio, RadioGroup};
use craft_ui::contract::{
    Bind, EditResponse, Entry, FieldLayout, LabelKind, NoValue, Outcome, Pattern, Props as _, Show as _, ShowContainer as _, SideLabel, Trigger,
    Validity,
};
use craft_ui::display::Text;
use craft_ui::form::Field;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::menu::Menu;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label, Message, Units};
use craft_ui::text::{NumberField, TextField};
use egui::{Response, Ui};

use crate::range::{PageRange, PageRangeError, Subset, format_pages, parse_pages};

craft_ui::words! {
    /// The words of the page range field.
    pub struct PageRangeFieldWords {
        selected_pages: "pageRangeField.selectedPages" = "Selected pages";
        select_pages: "pageRangeField.selectPages" = "Select pages";
        all_pages: "pageRangeField.allPages" = "All pages";
        current_page: "pageRangeField.currentPage" = "Current page";
        selected_preset: "pageRangeField.selectedPreset" = "Selected pages";
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
        pages_from: "pageRangeOptions.pagesFrom" = "Pages from";
        to: "pageRangeOptions.to" = "to";
        subset: "pageRangeOptions.subset" = "Subset";
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

/// A text field over page numbers with a menu button of presets.
///
/// - The field shows `pages` formatted ([`format_pages`]); with none, the current page is its placeholder.
/// - Typed text applies on Enter or when the field loses focus: text that reads reports the pages and shows them back
///   formatted; text that does not stays, with the error under the field. Escape puts the pages' text back.
/// - The button opens the presets: All pages, Current page and the document's selected pages; Down opens it from the
///   button. A pick reports like typed pages.
#[must_use = "show the field with `show`"]
#[derive(Pattern)]
pub struct PageRangeField<'a> {
    label: Label,
    pages: &'a [usize],
    selected: &'a [usize],
    count: usize,
    current: usize,
}

impl<'a> PageRangeField<'a> {
    /// The field named `label` over `pages` (0-based) in a document of `count` pages. `selected` are the pages the
    /// Selected pages preset picks and `current` (0-based) the page the Current page preset picks.
    pub fn new(label: impl Into<Label>, pages: &'a [usize], selected: &'a [usize], count: usize, current: usize) -> Self {
        PageRangeField { label: label.into(), pages, selected, count, current }
    }

    /// Shows the field.
    pub fn show(self, ui: &mut Ui) -> PageRangeFieldOutput {
        let PageRangeField { label, pages, selected, count, current } = self;
        let env = Env::of(ui);
        let words = env.words::<PageRangeFieldWords>();
        let units = env.units();
        let refuse = {
            let (words, units) = (words.clone(), units.clone());
            move |value: &PageSelection| match &value.error {
                Some(error) => Validity::Refuse(Message::new(words.error(error, &units))),
                None => Validity::Ok,
            }
        };
        let presets = [
            Entry::new((0..count).collect::<Vec<usize>>(), words.all_pages.clone()),
            Entry::new(vec![current.min(count.saturating_sub(1))], words.current_page.clone()),
            Entry::new(selected.to_vec(), words.selected_preset.clone()).enabled(!selected.is_empty()),
        ];
        let mut picked = None;
        let mut response = None;
        Field::new(label).side(SideLabel::new().kind(LabelKind::Hidden)).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                let value = Bind::given(PageSelection::of(pages)).check(&refuse);
                let field = TextField::with_codec(words.selected_pages.clone(), value, PageRangeCodec::new(count)).show(ui);
                if let Some(typed) = field.value().filter(|typed| typed.error.is_none()) {
                    picked = Some(typed.pages.clone());
                }
                response = Some(field.response.clone());
                let button =
                    IconButton::new(craft_ui::controls::disclosure::DisclosureGlyph::menu_button()).label(words.select_pages.clone()).show(ui);
                let menu = Menu::new(words.select_pages.clone()).opened_by(&button, Trigger::Press).pick(ui, &presets);
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

/// The All pages / Pages from [ ] to [ ] / Subset [ ] rows over a dialog's [`PageRange`]: a radio group for All pages
/// and the from-to range, whole-number fields for From and To (1 to the page count, off with All pages) and a list for
/// the subset.
#[must_use = "show the options with `show`"]
#[derive(Pattern)]
pub struct PageRangeOptions {
    label: Label,
    range: PageRange,
    count: usize,
}

impl PageRangeOptions {
    /// The options named `label` over `range` in a document of `count` pages.
    pub fn new(label: impl Into<Label>, range: PageRange, count: usize) -> Self {
        PageRangeOptions { label: label.into(), range, count }
    }

    /// Shows the options.
    pub fn show(self, ui: &mut Ui) -> PageRangeOptionsOutput {
        let PageRangeOptions { label, range, count } = self;
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
        let mut from = None;
        let mut to = None;
        let group = RadioGroup::new(label, Bind::given(range.all)).show(ui, |ui, cx| {
            Stack::new().show(ui, |ui, _| {
                cx.labelled(ui, "all", words.all_pages.clone(), after.clone(), Radio::new(true));
                // Two themed fields don't fit beside the radio in a narrow panel: "to" and its field wrap together.
                Row::new().wrap(true).show(ui, |ui, _| {
                    cx.labelled(ui, "range", words.pages_from.clone(), after.clone(), Radio::new(false));
                    let field = |label: &Label, shown: usize| {
                        NumberField::new(label.clone(), Bind::given(Some(shown as f64))).codec(numbers.clone()).enabled(!range.all)
                    };
                    from = whole(&field(&words.pages_from, shown_from).show(ui).edit, shown_from);
                    Row::new().show(ui, |ui, _| {
                        Text::new(words.to.clone()).show(ui);
                        to = whole(&field(&words.to, shown_to).show(ui).edit, shown_to);
                    });
                });
            });
        });
        if let Some(all) = group.output.value().copied() {
            next.all = all;
        }
        if let Some(value) = from {
            next.from = value;
            next.to = next.to.map(|to| to.max(value));
        }
        if let Some(value) = to {
            next.to = Some(value);
            next.from = next.from.min(value);
        }
        let entries = [
            Entry::new(Subset::All, words.all_in_range.clone()),
            Entry::new(Subset::Even, words.even_only.clone()),
            Entry::new(Subset::Odd, words.odd_only.clone()),
        ];
        Field::new(words.subset.clone()).show(ui, |ui, _| {
            let picked = Select::new(words.subset.clone(), Bind::given(range.subset), &entries).show(ui);
            if let Some(subset) = picked.value().copied() {
                next.subset = subset;
            }
        });
        PageRangeOptionsOutput { response: group.output.response.clone(), changed: (next != range).then_some(next) }
    }
}
