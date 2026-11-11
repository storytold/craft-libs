//! `PropertiesDialog`: an object's properties as tabs of label and value rows, with text, number and check rows to
//! edit and read-only rows to read.

use super::{DialogFields, FormDialog, Param, ParamRows, ParamRowsOutput, open_fields};
use crate::contract::{Bind, CollectionEvent, NoValue, Outcome, Remember as _, Show as _, ShowCollection as _};
use crate::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use crate::display::Text;
use crate::runtime::env::{Env, Label};
use crate::runtime::overlay::CloseReason;
use crate::runtime::words::CommonWords;
use crate::sections::{DialogRole, FooterButton};
use crate::tabs::{Tab, TabKind, TabStrip};
use craft_behavior::{DisplayOrder, ItemSource, SelectionChange};
use egui::{Response, Ui};
use std::borrow::Cow;
use std::cell::RefCell;

#[cfg(feature = "testing")]
mod stories;

/// One row of a properties tab.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PropertyRow {
    /// A label and its text: read-only.
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
}

impl PropertiesTab {
    /// A tab titled `title` with `rows`.
    pub fn new(title: impl Into<Label>, rows: Vec<PropertyRow>) -> Self {
        PropertiesTab { title: title.into(), rows }
    }

    /// The parameters the rows are: a read-only row draws its text, a heading is a section.
    fn params(&self, at: usize) -> Vec<Param> {
        self.rows
            .iter()
            .enumerate()
            .map(|(row, entry)| match entry {
                PropertyRow::Value { label, text } => {
                    let text = text.clone();
                    Param::custom(format!("properties.{at}.{row}"), label.clone(), move |ui, _, _| Text::new(text.clone()).show(ui))
                }
                PropertyRow::Field(param) => (**param).clone(),
                PropertyRow::Heading(label) => Param::section(label.clone()),
                PropertyRow::Space => Param::custom(format!("properties.{at}.{row}"), Label::Empty, |ui, _, _| Text::new(Label::Empty).show(ui)),
            })
            .collect()
    }
}

/// The tab titles as an item source, keyed by position.
struct Titles<'t>(&'t [PropertiesTab]);

impl DisplayOrder<usize> for Titles<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|tab| Cow::Borrowed(tab.title.as_str()))
    }
}

impl ItemSource<usize> for Titles<'_> {}

/// The tab the dialog shows while it is open.
#[derive(Clone, Default)]
struct OpenTab(usize);

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
}

impl Outcome for PropertiesOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A properties dialog: a `TabStrip` over the tabs (none with one tab) and the rows of the shown tab as one
/// `ParamRows` over the values of all of them.
///
/// - Keys: Tab walks the rows of the shown tab, arrows on the strip switch tabs, Enter is OK, Escape cancels.
/// - Read-only: the dialog has one button, and every row shows its value without a control; whichever way it closes
///   it answers with empty values.
/// - Values: the `values` binding holds them; the first pass fills the rows' defaults for keys it lacks. Alt turns
///   Cancel into Reset, which restores the values from when the dialog opened.
/// - Tab: the shown tab is kept while the dialog is open.
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct PropertiesDialog<'a> {
    title: Label,
    tabs: Cow<'a, [PropertiesTab]>,
    values: Bind<'a, DialogFields>,
    /// Whether the rows only show their values.
    #[prop]
    read_only: bool,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
}

impl<'a> PropertiesDialog<'a> {
    /// A dialog titled `title` over `tabs`, editing `values`.
    pub fn new(title: impl Into<Label>, tabs: impl Into<Cow<'a, [PropertiesTab]>>, values: Bind<'a, DialogFields>) -> Self {
        PropertiesDialog { title: title.into(), tabs: tabs.into(), values, read_only: false, open: None }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> PropertiesOutput {
        let PropertiesDialog { title, tabs, values, read_only, open } = self;
        let env = Env::of(ui);
        let common = env.words::<CommonWords>();
        let per_tab: Vec<Vec<Param>> = tabs.iter().enumerate().map(|(at, tab)| tab.params(at)).collect();
        let mut dialog = FormDialog::new(title.clone(), values).collection_body(true);
        if read_only {
            let ok = FooterButton::new(common.ok.clone(), DialogRole::Accept);
            dialog = dialog.alt_resets(false).shaped(move |dialog| dialog.buttons([ok]));
        }
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        let rows = RefCell::new(None::<ParamRowsOutput>);
        let shown = dialog.show(ui, |ui, form, fields| {
            if form.first() {
                let all: Vec<Param> = per_tab.iter().flatten().cloned().collect();
                open_fields(&all, fields);
            }
            let last = tabs.len().saturating_sub(1);
            let mut current = form.memory::<OpenTab>().0.min(last);
            if tabs.len() > 1 {
                let strip = TabStrip::new(title.clone())
                    .kind(TabKind::Text)
                    .shown(current)
                    .show_items(ui, &Titles(&tabs), &mut |key, _| Tab::new(tabs[*key].title.clone()));
                for event in strip.response.events {
                    if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
                        && let Some(key) = keys.into_iter().last()
                    {
                        current = key.min(last);
                    }
                }
            }
            if let Some(params) = per_tab.get(current) {
                *rows.borrow_mut() = Some(ParamRows::new(title.clone(), params, Bind::bound(&mut *fields)).read_only(read_only).show(ui));
            }
            form.memory::<OpenTab>().0 = current;
            true
        });
        let result = match shown.closed {
            Some(_) if read_only => Some(DialogFields::new()),
            _ => shown.accepted,
        };
        PropertiesOutput { response: shown.response, closed: shown.closed, result, rows: rows.into_inner() }
    }
}

/// Builds the dialog over the host-owned values of a [`HostedPropertiesDialog`] for one pass.
type Build = dyn for<'b> Fn(Bind<'b, DialogFields>) -> PropertiesDialog<'b> + Send;

/// A hosted properties dialog: the `DialogHost` owns the values, and `build` makes the dialog over them each pass.
/// It answers with the edited values at OK, and with empty values when the dialog is read-only.
///
/// ```ignore
/// host.open(HostedPropertiesDialog::new("file.properties", DialogFields::new(), move |values| {
///     PropertiesDialog::new("Properties", tabs.clone(), values)
/// }));
/// ```
pub struct HostedPropertiesDialog {
    id: DialogId,
    key: String,
    fields: DialogFields,
    build: Box<Build>,
}

impl HostedPropertiesDialog {
    /// A dialog for `key` opening with `fields`; `build` makes it over the values.
    pub fn new(
        key: impl Into<String>,
        fields: DialogFields,
        build: impl for<'b> Fn(Bind<'b, DialogFields>) -> PropertiesDialog<'b> + Send + 'static,
    ) -> Self {
        let key = key.into();
        HostedPropertiesDialog { id: dialog_id(key.as_str()), key, fields, build: Box::new(build) }
    }

    /// The values the dialog shows.
    pub fn fields(&self) -> &DialogFields {
        &self.fields
    }
}

impl StackDialog for HostedPropertiesDialog {
    type Result = DialogFields;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<DialogFields>> {
        let shown = (self.build)(Bind::bound(&mut self.fields)).show(ui);
        match (shown.result, shown.closed) {
            (Some(result), _) => Some(DialogResult::Ok(result)),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn key(&self) -> Option<&str> {
        Some(&self.key)
    }
}
