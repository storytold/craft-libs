//! The row model the host builds each frame, what the host implements, and what the window reports.

use std::ops::RangeInclusive;

use craft_theme::{Rgba, Srgba};
use egui::{Context, Response, Ui};

use crate::contract::{Entry, NoValue, Outcome};
use crate::runtime::env::UnitKind;
use crate::runtime::requests::ColorRequest;

/// A setting's value as it crosses the boundary between the window and the host.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PrefValue {
    /// A check box or switch.
    Bool(bool),
    /// An integer field.
    Int(i64),
    /// A number field.
    Float(f64),
    /// A text field.
    Text(String),
    /// The id of the chosen option.
    Choice(String),
    /// A colour.
    Color(Rgba),
}

/// One page of the page list.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PageSpec<'a> {
    /// Stable id: `"general"`.
    pub id: &'a str,
    /// The translated name in the page list and over the page.
    pub title: &'a str,
    /// The page's groups; one untitled group for a plain page.
    pub groups: Vec<GroupSpec<'a>>,
    /// A note under the last group.
    pub footnote: Option<&'a str>,
    /// Extra search text of the page, for a custom page whose controls the search can't see.
    pub keywords: &'a str,
    /// The host draws the whole page ([`PreferencesModel::custom_page`]).
    pub custom: bool,
}

impl<'a> PageSpec<'a> {
    /// A plain page of one untitled group.
    pub fn new(id: &'a str, title: &'a str, rows: Vec<RowSpec<'a>>) -> Self {
        Self { id, title, groups: vec![GroupSpec::new(None, rows)], footnote: None, keywords: "", custom: false }
    }

    /// A page the host draws.
    pub fn custom(id: &'a str, title: &'a str) -> Self {
        Self { id, title, groups: Vec::new(), footnote: None, keywords: "", custom: true }
    }

    /// Sets the note shown under the last group.
    #[must_use]
    pub fn footnote(mut self, footnote: &'a str) -> Self {
        self.footnote = Some(footnote);
        self
    }

    /// Sets extra search text for the page.
    #[must_use]
    pub fn keywords(mut self, keywords: &'a str) -> Self {
        self.keywords = keywords;
        self
    }

    /// Whether the page has anything to show: a custom page, or a visible row.
    pub fn has_content(&self) -> bool {
        self.custom || self.groups.iter().any(|g| g.rows.iter().any(|r| r.visible))
    }
}

/// How a group lays out its rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Columns {
    /// Labels in a column, controls beside them.
    #[default]
    Labelled,
    /// Check boxes start at the group's left edge, under the label column; the other rows keep their labels in it.
    /// Without this, a check box sits in the control column.
    Checks,
}

/// A group of rows: a titled box, or the page itself when untitled.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct GroupSpec<'a> {
    /// The translated title of the box; `None` for the page's untitled group.
    pub title: Option<&'a str>,
    /// How the rows are laid out.
    pub columns: Columns,
    /// The group's rows, in order.
    pub rows: Vec<RowSpec<'a>>,
}

impl<'a> GroupSpec<'a> {
    /// A group with [`Columns::Labelled`] layout.
    pub fn new(title: Option<&'a str>, rows: Vec<RowSpec<'a>>) -> Self {
        Self { title, columns: Columns::Labelled, rows }
    }

    /// Switches the group to [`Columns::Checks`] layout.
    #[must_use]
    pub fn checks(mut self) -> Self {
        self.columns = Columns::Checks;
        self
    }
}

/// One setting.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RowSpec<'a> {
    /// The host's path, unique: `"general.language"`.
    pub key: &'a str,
    /// Translated; also the row's accessible name.
    pub label: &'a str,
    /// Extra search text: the key, the English label, synonyms. A [`RowKind::Custom`] row, which the search can't look
    /// into, lists the terms of its control here.
    pub keywords: &'a str,
    /// Help under the control: "Applies at next launch." A line break starts a new paragraph. The search matches it.
    pub help: Option<&'a str>,
    /// False for hidden rows and rows hidden by another value.
    pub visible: bool,
    /// False: the control is drawn disabled.
    pub enabled: bool,
    /// The step of a [`RowKind::Float`] row (arrow keys, wheel, drag); without one the step comes from the range and
    /// decimals. Ignored by other kinds.
    pub step: Option<f64>,
    /// The control and the value it shows.
    pub kind: RowKind<'a>,
}

impl<'a> RowSpec<'a> {
    /// A visible, enabled row with no keywords or help.
    pub fn new(key: &'a str, label: &'a str, kind: RowKind<'a>) -> Self {
        Self { key, label, keywords: "", help: None, visible: true, enabled: true, step: None, kind }
    }

    /// Sets extra search text for the row.
    #[must_use]
    pub fn keywords(mut self, keywords: &'a str) -> Self {
        self.keywords = keywords;
        self
    }

    /// Sets the help text under the control.
    #[must_use]
    pub fn help(mut self, help: &'a str) -> Self {
        self.help = Some(help);
        self
    }

    /// Shows or hides the row.
    #[must_use]
    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    /// Enables or disables the control.
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Sets the step of a [`RowKind::Float`] row.
    #[must_use]
    pub fn step(mut self, step: f64) -> Self {
        self.step = Some(step);
        self
    }
}

/// What a row edits, with its value.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum RowKind<'a> {
    /// A check box.
    Bool(bool),
    /// One of `options`; each entry's value is the option's id.
    #[non_exhaustive]
    Choice {
        /// The id of the chosen option.
        value: &'a str,
        /// The options, labelled in the host's language.
        options: Vec<Entry<&'a str>>,
        /// What each option's id counts. The row then labels the options itself, spelling each id as a number of
        /// this unit in the user's language.
        unit: Option<UnitKind>,
    },
    /// A swatch and a hex field. A press on the swatch reports [`PreferencesOutput::pick_color`]; the host opens its
    /// colour picker.
    Color(Rgba),
    /// An integer field.
    #[non_exhaustive]
    Int {
        /// The current value.
        value: i64,
        /// The values the field accepts.
        range: RangeInclusive<i64>,
        /// What the number counts, shown after the field in the user's language.
        unit: Option<UnitKind>,
    },
    /// A number field.
    #[non_exhaustive]
    Float {
        /// The current value.
        value: f64,
        /// The values the field accepts.
        range: RangeInclusive<f64>,
        /// How many decimal places the field shows.
        decimals: u8,
        /// What the number counts, shown after the field in the user's language.
        unit: Option<UnitKind>,
    },
    /// A text field.
    Text(&'a str),
    /// The host draws the control ([`PreferencesModel::custom_row`]).
    Custom,
}

impl<'a> RowKind<'a> {
    /// A choice of `options`, with `value` chosen.
    pub fn choice(value: &'a str, options: Vec<Entry<&'a str>>) -> Self {
        RowKind::Choice { value, options, unit: None }
    }

    /// An integer field limited to `range`.
    pub fn int(value: i64, range: RangeInclusive<i64>) -> Self {
        RowKind::Int { value, range, unit: None }
    }

    /// A number field limited to `range`, showing `decimals` decimal places.
    pub fn float(value: f64, range: RangeInclusive<f64>, decimals: u8) -> Self {
        RowKind::Float { value, range, decimals, unit: None }
    }

    /// What an integer or number field counts, shown after it, or what the ids of a choice count (the choice labels its
    /// options with them); other kinds ignore it.
    #[must_use]
    pub fn counting(mut self, kind: UnitKind) -> Self {
        if let RowKind::Int { unit, .. } | RowKind::Float { unit, .. } | RowKind::Choice { unit, .. } = &mut self {
            *unit = Some(kind);
        }
        self
    }
}

/// How the window was answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Closed {
    /// OK: the host applied the draft.
    Ok,
    /// Cancel, Esc or the window's close: the host dropped the draft.
    Cancel,
}

/// What the window reports for one pass.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host closes the window when it is answered"]
pub struct PreferencesOutput {
    /// The window's response.
    pub response: Response,
    /// Set on the pass the window is answered; the host closes it.
    pub closed: Option<Closed>,
    /// The colour row whose swatch was pressed, for this pass only: the host opens its colour dialog and answers with
    /// [`PreferencesModel::set_color`] on the request's key.
    pub pick_color: Option<ColorRequest>,
    /// Apply ran this pass and succeeded (OK also applies, and closes).
    pub applied: bool,
    /// The id of the page shown, `None` when the search lists no page.
    pub page: Option<String>,
}

impl PreferencesOutput {
    /// The output of a pass in which nothing was answered, picked or applied.
    pub fn new(response: Response) -> Self {
        Self { response, closed: None, pick_color: None, applied: false, page: None }
    }
}

impl Outcome for PreferencesOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The app's side of the preferences window. The model keeps the saved settings and the draft; the window shows the
/// pages [`pages`](PreferencesModel::pages) returns, filters them and reports edits through
/// [`set`](PreferencesModel::set).
///
/// The window reads its words and numbers from the scope's language. To preview a language while the window is open,
/// install the host with the draft's language; the window follows on the next pass. [`cancel`](PreferencesModel::cancel)
/// installs the saved language again.
pub trait PreferencesModel {
    /// The pages, built from the draft.
    fn pages(&self) -> Vec<PageSpec<'_>>;
    /// A row changed in the draft. A cancelled gesture reports the value from before it.
    fn set(&mut self, key: &str, value: PrefValue);
    /// The colour picker's answer to a [`PreferencesOutput::pick_color`] request: the colour row `key` changes in the
    /// draft.
    fn set_color(&mut self, key: &str, color: Srgba) {
        self.set(key, PrefValue::Color(color.into()));
    }
    /// Draws a [`RowKind::Custom`] row's control.
    fn custom_row(&mut self, ui: &mut Ui, key: &str);
    /// Draws a [`PageSpec::custom`] page.
    fn custom_page(&mut self, ui: &mut Ui, page: &str);
    /// Whether the draft differs from the saved settings: enables Apply.
    fn differs(&self) -> bool;
    /// Whether OK may close the window.
    fn can_ok(&self) -> bool {
        true
    }
    /// Commits the draft. `Err` keeps it and the window open and shows the message.
    fn apply(&mut self) -> Result<(), String>;
    /// Drops the draft and undoes previews (the language).
    fn cancel(&mut self);
    /// Resets the page's rows in the draft to their defaults.
    fn reset_page(&mut self, page: &str);
    /// Draws the dialogs the host opened from this window (a colour picker) inside the same viewport. Called last in
    /// every pass.
    fn nested(&mut self, _ctx: &Context) {}
    /// Whether a nested dialog is open: the window takes no keys then, and refuses OK.
    fn nested_open(&self) -> bool {
        false
    }
}
