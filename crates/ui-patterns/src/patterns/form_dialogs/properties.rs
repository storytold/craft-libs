//! `PropertiesDialog`: an object's properties as tabs of label and value rows, with text, number and check rows to
//! edit and read-only rows to read.

use super::{DialogFields, FormDialog, HostRequests, Param, ParamRows, ParamRowsOutput, open_fields};
use crate::contract::{Bind, CollectionEvent, NoValue, Outcome, Remember as _, Show as _, ShowCollection as _};
use crate::dialog::{DialogId, DialogResult, DialogStack, HeldAnswer, HeldOk, HoldsOk, StackDialog, dialog_id};
use crate::display::Text;
use crate::runtime::env::{Env, Label};
use crate::runtime::overlay::CloseReason;
use crate::runtime::requests::{ColorRequest, HostAnswer};
use crate::runtime::words::CommonWords;
use crate::sections::{DialogRole, FooterButton};
use crate::tabs::{Tab, TabKind, TabStrip};
use craft_behavior::{DisplayOrder, ItemSource, SelectionChange};
use craft_theme::Srgba;
use egui::{Context, Response, Ui};
use serde_json::Value;
use std::borrow::Cow;
use std::cell::RefCell;

#[cfg(feature = "testing")]
mod stories;

craft_ui::words! {
    /// The words of the properties dialog beside the common OK and Cancel.
    pub struct PropertiesWords {
        reset_tab: "properties.resetTab" = "Reset Panel";
    }
}

/// How a properties dialog arranges its tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TabLayout {
    /// A tab strip above the rows.
    #[default]
    Strip,
    /// The tabs as a list of sections at the start edge.
    List,
}

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
    /// The tab's id for automation (the `tab` field) and for the host.
    pub key: Option<String>,
    /// The footer's Reset Panel button is enabled while this tab shows.
    pub resettable: bool,
}

impl PropertiesTab {
    /// A tab titled `title` with `rows`.
    pub fn new(title: impl Into<Label>, rows: Vec<PropertyRow>) -> Self {
        PropertiesTab { title: title.into(), rows, key: None, resettable: false }
    }

    /// Names the tab for automation and for the host.
    #[must_use]
    pub fn key(self, key: impl Into<String>) -> Self {
        let _ = (self, key);
        todo!("ty.X75")
    }

    /// Enables the footer's Reset Panel button while this tab shows; a press puts the tab's keys back to their rows'
    /// defaults.
    #[must_use]
    pub fn resettable(self) -> Self {
        let _ = self;
        todo!("ty.X76")
    }

    /// The parameters the rows are: a value is a read-only text row, a heading is a section.
    fn params(&self, at: usize) -> Vec<Param> {
        self.rows
            .iter()
            .enumerate()
            .map(|(row, entry)| match entry {
                PropertyRow::Value { label, text } => Param::read_only_text(format!("properties.{at}.{row}"), label.clone(), text.clone()),
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
    /// Reset Panel was pressed this pass.
    pub tab_reset: bool,
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
    /// How the tabs are arranged.
    #[prop]
    tab_layout: TabLayout,
    /// The shown tab's position when the host keeps it; without it the dialog keeps it while open.
    #[prop]
    tab: Option<Bind<'a, usize>>,
    /// The host's answers to the requests of earlier passes; the dialog writes them into the shown tab's values first.
    #[prop]
    answers: Vec<HostAnswer>,
    /// The Preview check box; the host previews while it is on.
    #[prop]
    preview: Option<Bind<'a, bool>>,
    /// Whether OK is enabled.
    #[prop]
    #[allow(dead_code)] // P3(type-w6): ty.X82
    ok_enabled: bool,
    /// Why OK is disabled: the tooltip of the disabled button.
    #[prop]
    #[allow(dead_code)] // P3(type-w6): ty.X82
    ok_reason: Option<Label>,
}

impl<'a> PropertiesDialog<'a> {
    /// A dialog titled `title` over `tabs`, editing `values`.
    pub fn new(title: impl Into<Label>, tabs: impl Into<Cow<'a, [PropertiesTab]>>, values: Bind<'a, DialogFields>) -> Self {
        PropertiesDialog {
            title: title.into(),
            tabs: tabs.into(),
            values,
            read_only: false,
            open: None,
            tab_layout: TabLayout::default(),
            tab: None,
            answers: Vec::new(),
            preview: None,
            ok_enabled: true,
            ok_reason: None,
        }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> PropertiesOutput {
        let PropertiesDialog { title, tabs, values, read_only, open, tab_layout: _, tab: _, answers: _, preview: _, ok_enabled: _, ok_reason: _ } =
            self;
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
                    if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event // P3(mechanical): dp.C12
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
        PropertiesOutput { response: shown.response, closed: shown.closed, result, rows: rows.into_inner(), tab_reset: false }
    }

    /// Puts the keys of `tab` (the tab at position `at`) back to their rows' defaults, as the Reset Panel button does;
    /// a key whose row has no default is removed.
    pub fn reset_tab(tab: &PropertiesTab, at: usize, fields: &mut DialogFields) {
        let _ = (tab, at, fields);
        todo!("ty.X78")
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
    /// The colour requests the rows made, and the host's answers to them.
    #[allow(dead_code)] // P3(type-w6): fl.F36
    requests: HostRequests,
    /// The tab the host shows, bound to the dialog's `tab` prop.
    #[allow(dead_code)] // P3(type-w6): fl.F36
    tab: usize,
    /// The OK the host holds back until it answers `answer_ok`, and its refusal.
    #[allow(dead_code)] // P3(type-w6): ty.X82
    held: HeldAnswer<DialogFields>,
}

impl HostedPropertiesDialog {
    /// A dialog for `key` opening with `fields`; `build` makes it over the values.
    pub fn new(
        key: impl Into<String>,
        fields: DialogFields,
        build: impl for<'b> Fn(Bind<'b, DialogFields>) -> PropertiesDialog<'b> + Send + 'static,
    ) -> Self {
        let key = key.into();
        HostedPropertiesDialog {
            id: dialog_id(key.as_str()),
            key,
            fields,
            build: Box::new(build),
            requests: HostRequests::new(),
            tab: 0,
            held: HeldAnswer::new(),
        }
    }

    /// The values the dialog shows.
    pub fn fields(&self) -> &DialogFields {
        &self.fields
    }

    /// Makes OK file its answer for the host to take instead of closing: the host checks it and answers with
    /// [`answer_ok`](HostedPropertiesDialog::answer_ok). OK stays disabled until then.
    #[must_use]
    pub fn held_ok(self) -> Self {
        let _ = self;
        todo!("ty.X82")
    }

    /// [`held_ok`](HostedPropertiesDialog::held_ok) for a dialog already open.
    pub fn hold_ok(&mut self) {
        let _ = self;
        todo!("ty.X82")
    }

    /// The OK filed by a held dialog, once.
    pub fn take_ok(&mut self) -> Option<DialogFields> {
        let _ = self;
        todo!("ty.X82")
    }

    /// The host's answer to the held OK: `Ok` closes the dialog with the held answer in the next pass; `Err` enables
    /// OK again and shows the label under the rows until a value changes.
    pub fn answer_ok(&mut self, answer: Result<(), Label>) {
        let _ = (self, answer);
        todo!("ty.X82")
    }

    /// `Some`: the host refuses the current values, and OK stays off with the reason shown under the rows until the
    /// values change; `None` ends the refusal.
    pub fn refuse(&mut self, reason: Option<Label>) {
        let _ = (self, reason);
        todo!("ty.X82")
    }

    /// The colour row whose button was pressed, once.
    pub fn pick_color(&mut self) -> Option<ColorRequest> {
        let _ = self;
        todo!("fl.F36")
    }

    /// Answers the colour request `key`: the dialog writes the colour into its values in its next pass.
    pub fn answer_color(&mut self, key: impl Into<String>, color: Srgba) {
        let _ = (self, key, color);
        todo!("fl.F36")
    }

    /// Drops the pending colour request.
    pub fn cancel_color(&mut self) {
        let _ = self;
        todo!("fl.F36")
    }

    /// The dialog has the Preview check box, starting on.
    #[must_use]
    pub fn previews(self) -> Self {
        let _ = self;
        todo!("ty.X79")
    }

    /// Whether the dialog previews and its Preview check box is on.
    pub fn is_previewing(&self) -> bool {
        let _ = self;
        todo!("ty.X80")
    }

    /// The position of the shown tab, which the host binds.
    pub fn tab(&self) -> usize {
        let _ = self;
        todo!("ty.X81")
    }

    /// Whether Reset Panel is enabled: the shown tab is resettable.
    pub fn can_reset_tab(&self) -> bool {
        let _ = self;
        todo!("ty.X81")
    }

    /// Runs Reset Panel on the shown tab, as the button does; whether it changed a value.
    pub fn reset_tab(&mut self) -> bool {
        let _ = self;
        todo!("ty.X81")
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

    /// The values as [`DialogFields`] serializes them, plus `"tab"` (the shown tab's key) and `"preview"`.
    fn json_fields(&self) -> Option<Value> {
        todo!("ty.X82")
    }

    /// Takes `tab` (a key), `preview` (a bool), else a param field over every tab's params; inactive and unknown fields
    /// are refused.
    fn set_json_field(&mut self, ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        let _ = (ctx, field, value);
        todo!("ty.X82")
    }

    /// The values, as OK answers them (empty values when read-only).
    fn confirm(&mut self, ctx: &Context) -> Option<DialogFields> {
        let _ = ctx;
        todo!("ty.X82")
    }

    /// Drops the pending colour requests.
    fn cancelled(&mut self, ctx: &Context) {
        let _ = ctx;
        todo!("ty.X82")
    }

    /// Where the held OK stands.
    fn held_state(&self) -> Option<HeldOk> {
        todo!("ty.X82")
    }
}

impl HoldsOk for HostedPropertiesDialog {
    type Answer = DialogFields;

    fn held_ok(self) -> Self {
        HostedPropertiesDialog::held_ok(self)
    }

    fn take_ok(&mut self) -> Option<DialogFields> {
        HostedPropertiesDialog::take_ok(self)
    }

    fn answer_ok(&mut self, answer: Result<(), Label>) {
        HostedPropertiesDialog::answer_ok(self, answer);
    }
}
