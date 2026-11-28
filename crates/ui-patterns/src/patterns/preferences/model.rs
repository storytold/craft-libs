//! The row model the host builds each frame, what the host implements, and what the window reports.

use std::ops::RangeInclusive;

use craft_theme::{Rgba, Srgba};
use egui::{Response, Ui};

use crate::contract::{Entry, NoValue, Outcome};
use crate::runtime::env::{LengthUnits, UnitKind};
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

    /// A page the host draws ([`PreferencesModel::custom_page`]). Its [`group`](PageSpec::group)s follow the host's
    /// part, drawn and searched as a plain page's groups are.
    pub fn custom(id: &'a str, title: &'a str) -> Self {
        Self { id, title, groups: Vec::new(), footnote: None, keywords: "", custom: true }
    }

    /// Adds `group` under a custom page's own part, where the search sees its rows.
    #[must_use]
    pub fn group(mut self, group: GroupSpec<'a>) -> Self {
        self.groups.push(group);
        self
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

/// A group of rows: a titled box, or the page itself when untitled. Its rows share one label column; a check box row
/// starts in the control column when the group has rows labelled beside their controls, else at the row's start.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct GroupSpec<'a> {
    /// The translated title of the box; `None` for the page's untitled group.
    pub title: Option<&'a str>,
    /// The group's rows, in order.
    pub rows: Vec<RowSpec<'a>>,
}

impl<'a> GroupSpec<'a> {
    /// A group titled `title` holding `rows`.
    pub fn new(title: Option<&'a str>, rows: Vec<RowSpec<'a>>) -> Self {
        Self { title, rows }
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
    /// Extra search text: the key, the English label, synonyms. A [`RowKind::Custom`] or [`RowKind::CustomRows`] row,
    /// which the search can't look into, lists the terms of what the host draws here.
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
    /// A choice row: `Some(true)` draws its options as a segmented (pill) control, `Some(false)` as a drop-down; by
    /// default a drop-down, and on a card ([`PreferencesLayout::Cards`](super::PreferencesLayout::Cards)) a short
    /// choice of up to five options is segmented. Ignored by other kinds.
    pub pills: Option<bool>,
    /// The control and the value it shows.
    pub kind: RowKind<'a>,
}

impl<'a> RowSpec<'a> {
    /// A visible, enabled row with no keywords or help.
    pub fn new(key: &'a str, label: &'a str, kind: RowKind<'a>) -> Self {
        Self { key, label, keywords: "", help: None, visible: true, enabled: true, step: None, pills: None, kind }
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

    /// Chooses how a [`RowKind::Choice`] row draws its options: `true` as segments, `false` as a drop-down.
    #[must_use]
    pub fn pills(mut self, pills: bool) -> Self {
        self.pills = Some(pills);
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
        /// A translated unit after the number, for quantities with no `UnitKind`; `counting` wins when both are set.
        unit_text: Option<&'a str>,
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
        /// A translated unit after the number, for quantities with no `UnitKind`; `counting` wins when both are set.
        unit_text: Option<&'a str>,
    },
    /// A length: stored in `lengths.stored` and shown in `lengths.shown`; typed text accepts any length spelling.
    #[non_exhaustive]
    Length {
        /// The current value in the stored unit.
        value: f64,
        /// The values the field accepts, in the stored unit.
        range: RangeInclusive<f64>,
        /// The stored and the shown unit.
        lengths: LengthUnits,
    },
    /// A text field.
    Text(&'a str),
    /// The host draws the control ([`PreferencesModel::custom_row`]); the window labels it beside the control.
    Custom,
    /// The host draws whole rows, labels included ([`PreferencesModel::custom_row`]): the window adds no label, and
    /// shows the row's help under them and its enabled state on them. The row's label stays its search text and its
    /// group's name.
    CustomRows,
}

impl<'a> RowKind<'a> {
    /// A choice of `options`, with `value` chosen.
    pub fn choice(value: &'a str, options: Vec<Entry<&'a str>>) -> Self {
        RowKind::Choice { value, options, unit: None }
    }

    /// An integer field limited to `range`.
    pub fn int(value: i64, range: RangeInclusive<i64>) -> Self {
        RowKind::Int { value, range, unit: None, unit_text: None }
    }

    /// A number field limited to `range`, showing `decimals` decimal places.
    pub fn float(value: f64, range: RangeInclusive<f64>, decimals: u8) -> Self {
        RowKind::Float { value, range, decimals, unit: None, unit_text: None }
    }

    /// A length field limited to `range` in the stored unit, shown in the shown unit of `lengths`.
    pub fn length(value: f64, range: RangeInclusive<f64>, lengths: LengthUnits) -> Self {
        RowKind::Length { value, range, lengths }
    }

    /// A translated unit shown after an integer or number field, for quantities with no `UnitKind`; other kinds
    /// ignore it.
    #[must_use]
    pub fn unit_text(mut self, text: &'a str) -> Self {
        if let RowKind::Int { unit_text, .. } | RowKind::Float { unit_text, .. } = &mut self {
            *unit_text = Some(text);
        }
        self
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

/// The app's side of a preferences page. The page shows the pages [`pages`](PreferencesModel::pages) returns, filters
/// them and reports each edit through [`set`](PreferencesModel::set) in the pass it lands in.
///
/// A [`PreferencesPage`](super::PreferencesPage) needs only this trait: its edits apply at once. A
/// [`PreferencesWindow`](super::PreferencesWindow) also needs [`PreferencesDraft`]: there `set` edits a draft that OK
/// or Apply commits and Cancel drops.
///
/// The page reads its words and numbers from the scope's language. To preview a language while the window is open,
/// install the host with the draft's language; the page follows on the next pass. [`PreferencesDraft::cancel`]
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
    /// Draws a [`RowKind::Custom`] row's control, which the window labels from the row's label (an empty label shows
    /// an empty one), or a [`RowKind::CustomRows`] row's whole rows, labels included. Returns whether the row changed
    /// the draft this pass, so an edit made there clears a failed apply's message as any row's edit does.
    fn custom_row(&mut self, ui: &mut Ui, key: &str) -> bool;
    /// Draws a [`PageSpec::custom`] page, returning whether it changed the draft this pass.
    fn custom_page(&mut self, ui: &mut Ui, page: &str) -> bool;
    /// Resets the page's rows in the draft to their defaults.
    fn reset_page(&mut self, page: &str);
    /// Draws the dialogs the host opened from this window (a colour picker) with the body's `Ui`, in the window's own
    /// viewport. Called last in every pass.
    fn nested(&mut self, _ui: &mut Ui) {}
    /// Whether a nested dialog is open: the window takes no keys then, and refuses OK.
    fn nested_open(&self) -> bool {
        false
    }
}

/// The draft side of a [`PreferencesModel`]: what a [`PreferencesWindow`](super::PreferencesWindow) with OK, Cancel and
/// Apply needs beyond the rows. A [`PreferencesPage`](super::PreferencesPage) never calls it.
pub trait PreferencesDraft: PreferencesModel {
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
}
