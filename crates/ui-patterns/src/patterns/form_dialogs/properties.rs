//! `PropertiesDialog`: an object's properties as tabs of label and value rows, with text, number and check rows to
//! edit and read-only rows to read.

use super::{DialogFields, Param, ParamDialog, ParamRowsOutput, ParamTab, SharedRows};
use crate::contract::{Bind, NoValue, Outcome, Show as _};
use crate::display::Text;
use crate::runtime::env::Label;
use crate::runtime::overlay::CloseReason;
use egui::{Response, Ui};
use std::borrow::Cow;

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// One row of a properties tab.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PropertyRow {
    /// A label and its text: read-only text the user selects and copies.
    #[non_exhaustive]
    Value {
        /// The row's label.
        label: Label,
        /// The text shown.
        text: Label,
    },
    /// A parameter row: text, number or check box, edited in the dialog unless it is read-only.
    Field(Box<Param>),
    /// A heading over the rows that follow, up to the next heading.
    Heading(Label),
    /// An empty row that separates the rows around it.
    Space,
}

impl PropertyRow {
    /// A read-only row.
    pub fn value(label: impl Into<Label>, text: impl Into<Label>) -> Self {
        PropertyRow::Value { label: label.into(), text: text.into() }
    }

    /// A row for `param`.
    pub fn field(param: Param) -> Self {
        PropertyRow::Field(Box::new(param))
    }

    /// An empty row between two groups of rows.
    pub fn space() -> Self {
        PropertyRow::Space
    }

    /// A heading.
    pub fn heading(label: impl Into<Label>) -> Self {
        PropertyRow::Heading(label.into())
    }
}

/// A tab of a properties dialog.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PropertiesTab {
    /// The tab's title.
    pub title: Label,
    /// The rows under it.
    pub rows: Vec<PropertyRow>,
    /// A note shown above the rows of a read-only tab.
    pub note: Option<Label>,
    /// Whether the rows show their values without controls.
    pub read_only: bool,
}

impl PropertiesTab {
    /// A tab titled `title` with `rows`.
    pub fn new(title: impl Into<Label>, rows: Vec<PropertyRow>) -> Self {
        PropertiesTab { title: title.into(), rows, note: None, read_only: false }
    }

    /// The tab as a read-only one, with `note` above its rows.
    #[must_use]
    pub fn read_only(self, note: Option<Label>) -> Self {
        PropertiesTab { note, read_only: true, ..self }
    }

    /// The tab as a tab of a [`ParamDialog`]; `at` keys the rows that hold no value of their own.
    fn param_tab(&self, at: usize) -> ParamTab {
        let tab = ParamTab::new(self.title.clone(), params(&self.rows, &format!("properties.{at}")));
        if self.read_only { tab.read_only(self.note.clone()) } else { tab }
    }
}

/// The parameters `rows` are: a value is a read-only text row, a heading is a section. The rows that hold no value of
/// their own are keyed `{prefix}.{row}`.
fn params(rows: &[PropertyRow], prefix: &str) -> Vec<Param> {
    rows.iter()
        .enumerate()
        .map(|(row, entry)| match entry {
            PropertyRow::Value { label, text } => Param::read_only_text(format!("{prefix}.{row}"), label.clone(), text.clone()),
            PropertyRow::Field(param) => (**param).clone(),
            PropertyRow::Heading(label) => Param::section(label.clone()),
            PropertyRow::Space => Param::custom(format!("{prefix}.{row}"), Label::Empty, |ui, _, _| Text::new(Label::Empty).show(ui)),
        })
        .collect()
}

/// What a properties dialog reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PropertiesOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The answer: at OK the edited values; empty when the dialog is read-only, whichever way it closed.
    pub result: Option<DialogFields>,
    /// The rows' output of this pass; `None` when the dialog is closed.
    pub rows: Option<ParamRowsOutput>,
    /// The id of the extra footer button or custom row button pressed this pass (`PropertiesDialog::extra_buttons`).
    pub action: Option<String>,
}

impl Outcome for PropertiesOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A properties dialog: a builder over a tabbed [`ParamDialog`] ([`into_param_dialog`](PropertiesDialog::into_param_dialog)),
/// a `TabStrip` over the tabs (none with one tab) and the rows of the shown tab over the values of all of them.
///
/// - Keys: Tab walks the rows of the shown tab, arrows on the strip switch tabs, Enter is OK, Escape cancels.
/// - Read-only: the dialog has one button, and every row shows its value without a control; whichever way it closes
///   it answers with empty values. A [`PropertiesTab::read_only`] tab shows its values only, under its note.
/// - Values: the `values` binding holds them; the first pass fills the rows' defaults for keys it lacks. Alt turns
///   Cancel into Reset, which restores the values from when the dialog opened.
/// - Shared rows: the [`shared`](PropertiesDialog::shared) rows show with every tab, where `shared_rows` puts them.
/// - Tab: the `tab` binding says which tab shows; without it the shown tab is kept while the dialog is open.
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct PropertiesDialog<'a> {
    key: String,
    title: Label,
    tabs: Cow<'a, [PropertiesTab]>,
    values: Bind<'a, DialogFields>,
    /// Whether the rows only show their values.
    #[prop]
    read_only: bool,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    /// Binds the shown tab to `tab`.
    #[prop]
    tab: Option<Bind<'a, usize>>,
    /// Extra footer buttons after OK and Cancel, each an id and a label; a press comes back as
    /// `PropertiesOutput::action`.
    #[prop]
    extra_buttons: Vec<(String, Label)>,
    /// The rows shown with every tab.
    #[prop]
    shared: Vec<PropertyRow>,
    /// Where the shared rows go: above the tab strip, or under the shown tab's rows.
    #[prop]
    shared_rows: SharedRows,
}

impl<'a> PropertiesDialog<'a> {
    /// A dialog answering for `key`, titled `title` over `tabs`, editing `values`.
    pub fn new(key: impl Into<String>, title: impl Into<Label>, tabs: impl Into<Cow<'a, [PropertiesTab]>>, values: Bind<'a, DialogFields>) -> Self {
        PropertiesDialog {
            key: key.into(),
            title: title.into(),
            tabs: tabs.into(),
            values,
            read_only: false,
            open: None,
            tab: None,
            extra_buttons: Vec::new(),
            shared: Vec::new(),
            shared_rows: SharedRows::default(),
        }
    }

    /// The dialog as a tabbed [`ParamDialog`] over the same values.
    ///
    /// A host owns a properties dialog as it owns any parameter dialog:
    ///
    /// ```ignore
    /// host.open(HostedParamDialog::new(ctx, "file.properties", DialogFields::new(), move |fields| {
    ///     PropertiesDialog::new("file.properties", "Properties", tabs.clone(), Bind::bound(fields)).into_param_dialog()
    /// }));
    /// ```
    pub fn into_param_dialog(self) -> ParamDialog<'a> {
        let PropertiesDialog { key, title, tabs, values, read_only, open, tab, extra_buttons, shared, shared_rows } = self;
        let tabs: Vec<ParamTab> = tabs.iter().enumerate().map(|(at, tab)| tab.param_tab(at)).collect();
        let mut dialog = ParamDialog::new(key, title, params(&shared, "properties.shared"))
            .tabs(tabs)
            .shared_rows(shared_rows)
            .read_only(read_only)
            .values(values)
            .extra_buttons(extra_buttons);
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        if let Some(tab) = tab {
            dialog = dialog.tab(tab);
        }
        dialog
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> PropertiesOutput {
        let shown = self.into_param_dialog().show(ui);
        PropertiesOutput {
            response: shown.response,
            closed: shown.closed,
            result: shown.result.map(|result| result.fields),
            rows: shown.rows,
            action: shown.action,
        }
    }
}
