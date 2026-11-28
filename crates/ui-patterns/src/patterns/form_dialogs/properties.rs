//! `PropertiesDialog`: an object's properties as tabs of label and value rows, with text, number and check rows to
//! edit and read-only rows to read.

use super::dialog::{ResetFooter, ResetPick, apply_answers, set_param_field};
use super::form::{read_bind, write_bind};
use super::param::fill_defaults;
use super::{DialogFields, FormDialog, HostRequests, Param, ParamDialog, ParamRows, ParamRowsOutput, ParamTab, SectionList, SharedRows, open_fields};
use crate::buttons::Button;
use crate::contract::{Bind, CollectionEvent, NoValue, Outcome, Props as _, Remember as _, Show as _, ShowCollection as _, clicked};
use crate::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, HoldsOk, StackDialog, dialog_id};
use crate::display::Text;
use crate::list::ListRow;
use crate::notices::{Notice, NoticeLayout};
use crate::runtime::env::{Env, Label};
use crate::runtime::overlay::CloseReason;
use crate::runtime::requests::{ColorRequest, HostAnswer};
use crate::runtime::words::CommonWords;
use crate::sections::{DialogRole, FooterButton};
use crate::tabs::{Tab, TabKind, TabStrip};
use craft_behavior::{DisplayOrder, ItemSource, SelectionChange};
use craft_theme::variant::Tone;
use egui::{Context, Response, Ui};
use serde_json::Value;
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[cfg(feature = "suite")]
pub(crate) mod stories;

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
    /// A note shown above the rows of a read-only tab.
    pub note: Option<Label>,
    /// Whether the rows show their values without controls.
    pub read_only: bool,
}

impl PropertiesTab {
    /// A tab titled `title` with `rows`.
    pub fn new(title: impl Into<Label>, rows: Vec<PropertyRow>) -> Self {
        PropertiesTab { title: title.into(), rows, key: None, resettable: false, note: None, read_only: false }
    }

    /// Names the tab for automation and for the host.
    #[must_use]
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Enables the footer's Reset Panel button while this tab shows; a press puts the tab's keys back to their rows'
    /// defaults.
    #[must_use]
    pub fn resettable(mut self) -> Self {
        self.resettable = true;
        self
    }

    /// The parameters this tab's rows are; `at` keys the rows that hold no value of their own.
    fn params(&self, at: usize) -> Vec<Param> {
        params(&self.rows, &format!("properties.{at}"))
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
    /// Reset Panel was pressed this pass.
    pub tab_reset: bool,
    /// The answer OK filed this pass while `hold_ok` is on; the host accepts or refuses it.
    pub held: Option<DialogFields>,
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
/// - Tab: the `tab` binding says which tab shows; without it the shown tab is kept while the dialog is open; with
///   [`TabLayout::List`] the tabs stand as a list of sections at the start edge instead of a strip.
/// - Reset Panel: a footer button, enabled while the shown tab is [`resettable`](PropertiesTab::resettable); it puts
///   that tab's keys back to their rows' defaults, and Alt on Cancel still restores the values from when the dialog
///   opened.
/// - Held OK: with `hold_ok`, OK files its answer in the output's `held` and the dialog stays open until the host sets
///   `finish_held` or a `refusal`.
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
    ok_enabled: bool,
    /// Why OK is disabled: the tooltip of the disabled button.
    #[prop]
    ok_reason: Option<Label>,
    /// OK files its answer in the output's `held` instead of closing the dialog; the host answers it.
    #[prop]
    hold_ok: bool,
    /// The host accepted the held answer: the dialog closes with it.
    #[prop]
    finish_held: bool,
    /// The host refused the held answer for the reason, which shows under the rows until the values change.
    #[prop]
    refusal: Option<(Label, DialogFields)>,
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
            tab_layout: TabLayout::default(),
            tab: None,
            answers: Vec::new(),
            preview: None,
            ok_enabled: true,
            ok_reason: None,
            hold_ok: false,
            finish_held: false,
            refusal: None,
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
        let PropertiesDialog { key, title, tabs, values, read_only, open, tab, extra_buttons, shared, shared_rows, .. } = self;
        let tabs: Vec<ParamTab> = tabs.iter().enumerate().map(|(at, tab)| tab.param_tab(at)).collect();
        let extra_buttons = extra_buttons.into_iter().map(|(id, label)| (id, FooterButton::new(label, DialogRole::Other))).collect();
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
        let use_form_dialog = self.tab_layout == TabLayout::List
            || self.hold_ok
            || self.finish_held
            || self.refusal.is_some()
            || self.preview.is_some()
            || !self.answers.is_empty()
            || !self.ok_enabled
            || self.ok_reason.is_some()
            || self.tabs.iter().any(|tab| tab.resettable);
        if !use_form_dialog {
            let shown = self.into_param_dialog().show(ui);
            return PropertiesOutput {
                response: shown.response,
                closed: shown.closed,
                result: shown.result.map(|result| result.fields),
                rows: shown.rows,
                tab_reset: false,
                held: None,
                action: shown.action,
            };
        }
        let PropertiesDialog {
            title,
            tabs,
            mut values,
            read_only,
            open,
            tab_layout,
            tab: mut tab_bind,
            answers,
            preview,
            ok_enabled,
            ok_reason,
            hold_ok,
            finish_held,
            refusal,
            ..
        } = self;
        let env = Env::of(ui);
        let common = env.words::<CommonWords>();
        let words = env.words::<PropertiesWords>();
        let per_tab: Vec<Vec<Param>> = tabs.iter().enumerate().map(|(at, tab)| tab.params(at)).collect();
        let hold = hold_ok && !read_only;
        let bound_tab = tab_bind.as_mut().and_then(read_bind);
        let footer = ResetFooter::new(ui, None, !read_only);
        // The footer is a region of its own: the body tells it whether the shown tab resets, and it tells back a press.
        let (resettable, pressed) = (Rc::new(Cell::new(false)), Rc::new(Cell::new(false)));
        let mut dialog =
            FormDialog::new(title.clone(), values.reborrowed()).collection_body(true).ok_enabled(ok_enabled).accept_stays_open(hold).answers(answers);
        if let Some(reason) = ok_reason {
            dialog = dialog.ok_reason(reason);
        }
        if read_only {
            let ok = FooterButton::new(common.ok.clone(), DialogRole::Accept);
            dialog = dialog.alt_resets(false).shaped(move |dialog| dialog.buttons([ok]));
        } else if tabs.iter().any(|tab| tab.resettable) {
            let (on, pressed, label) = (Rc::clone(&resettable), Rc::clone(&pressed), words.reset_tab.clone());
            dialog = dialog.footer_extra(move |ui| {
                if clicked(&Button::new(label.clone()).enabled(on.get()).show(ui)) {
                    pressed.set(true);
                }
            });
        }
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        if let Some(preview) = preview {
            dialog = dialog.preview(Some(preview));
        }
        let rows = RefCell::new(None::<ParamRowsOutput>);
        let (shown_tab, mut latest) = (Cell::new(0), None::<DialogFields>);
        let shown = dialog.show(ui, |ui, form, fields| {
            if form.first() {
                let all: Vec<Param> = per_tab.iter().flatten().cloned().collect();
                open_fields(&all, fields);
            }
            let last = tabs.len().saturating_sub(1);
            let mut current = bound_tab.unwrap_or_else(|| form.memory::<OpenTab>().0).min(last);
            let answers = form.take_answers();
            if let Some(params) = per_tab.get(current) {
                apply_answers(params, fields, answers);
            }
            // A measuring run closes nothing: the real run of the same pass makes the close.
            if finish_held && !crate::runtime::measure::active(ui.ctx()) {
                form.request_close_as(CloseReason::Activated);
            }
            let mut show_rows = |ui: &mut Ui, current: usize| {
                if let Some(params) = per_tab.get(current) {
                    *rows.borrow_mut() = Some(ParamRows::new(title.clone(), params, Bind::bound(&mut *fields)).read_only(read_only).show(ui));
                }
            };
            let titles = Titles(&tabs);
            let title_of = |key: &usize| tabs.get(*key).map_or(Label::Empty, |tab| tab.title.clone());
            match (tabs.len() > 1, tab_layout) {
                (true, TabLayout::List) => {
                    let picked =
                        SectionList::new(title.clone(), &titles, current).show(ui, |key| ListRow::new(title_of(key)), |ui| show_rows(ui, current));
                    current = picked.map_or(current, |key| key.min(last));
                }
                (true, TabLayout::Strip) => {
                    let strip = TabStrip::new(title.clone())
                        .kind(TabKind::Text)
                        .shown(current)
                        .show_items(ui, &titles, &mut |key, _| Tab::new(title_of(key)));
                    for event in strip.response.events {
                        if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
                            && let Some(key) = keys.into_iter().last()
                        {
                            current = key.min(last);
                        }
                    }
                    show_rows(ui, current);
                }
                _ => show_rows(ui, current),
            }
            if let Some((reason, _)) = refusal.as_ref().filter(|(_, refused)| *refused == *fields) {
                Notice::new(Tone::Bad, reason.clone()).layout(NoticeLayout::Inline).show(ui);
            }
            form.memory::<OpenTab>().0 = current;
            shown_tab.set(current);
            resettable.set(tabs.get(current).is_some_and(|tab| tab.resettable));
            if hold {
                latest = Some(fields.clone());
            }
            true
        });
        if let Some(bind) = tab_bind.as_mut()
            && read_bind(bind) != Some(shown_tab.get())
        {
            write_bind(bind, shown_tab.get());
        }
        let tab_reset = pressed.get();
        if tab_reset && let Some(tab) = tabs.get(shown_tab.get()) {
            let mut fields = read_bind(&mut values).unwrap_or_default();
            Self::reset_tab(tab, shown_tab.get(), &mut fields);
            write_bind(&mut values, fields);
        }
        let result = match shown.closed {
            Some(_) if read_only => Some(DialogFields::new()),
            _ => shown.accepted,
        };
        let held = match (footer.pick(shown.picked), latest) {
            (Some(ResetPick::Accept), Some(fields)) if result.is_none() => Some(fields),
            _ => None,
        };
        PropertiesOutput { response: shown.response, closed: shown.closed, result, rows: rows.into_inner(), tab_reset, held, action: None }
    }

    /// Puts the keys of `tab` (the tab at position `at`) back to their rows' defaults, as the Reset Panel button does;
    /// a key whose row has no default is removed.
    pub fn reset_tab(tab: &PropertiesTab, at: usize, fields: &mut DialogFields) {
        let params = tab.params(at);
        for param in &params {
            fields.remove(&param.key);
            if let Some(unit) = param.unit_key() {
                fields.remove(unit);
            }
        }
        fill_defaults(&params, fields);
        if let Some(key) = &tab.key {
            fields.mark_reset(key.clone());
        }
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
    requests: HostRequests,
    /// The tab the host shows, bound to the dialog's `tab` prop.
    tab: usize,
    /// Whether the dialog has the Preview check box ([`previews`](HostedPropertiesDialog::previews)).
    previews: bool,
    /// The Preview check box's value, bound to the dialog's `preview` prop so it is known before the first pass.
    preview_on: bool,
    /// Whether OK files its answer for the host instead of closing ([`held_ok`](HostedPropertiesDialog::held_ok)).
    hold: bool,
    /// The OK the host holds back until it answers `answer_ok`, and its refusal.
    held: HeldAnswer<DialogFields>,
    /// The host's refusal of the current values ([`refuse`](HostedPropertiesDialog::refuse)): the reason and the values
    /// it refused.
    refusal: Option<(Label, DialogFields)>,
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
            previews: false,
            preview_on: true,
            hold: false,
            held: HeldAnswer::new(),
            refusal: None,
        }
    }

    /// The tabs `build` makes over the current values.
    fn tabs(&self) -> Vec<PropertiesTab> {
        (self.build)(Bind::given(self.fields.clone())).tabs.into_owned()
    }

    /// The params of every tab, for the rows `set_json_field` may set.
    fn params(&self) -> Vec<Param> {
        self.tabs().iter().enumerate().flat_map(|(at, tab)| tab.params(at)).collect()
    }

    /// The values the dialog shows.
    pub fn fields(&self) -> &DialogFields {
        &self.fields
    }

    /// Makes OK file its answer for the host to take instead of closing: the host checks it and answers with
    /// [`answer_ok`](HostedPropertiesDialog::answer_ok). OK stays disabled until then.
    #[must_use]
    pub fn held_ok(mut self) -> Self {
        self.hold = true;
        self
    }

    /// [`held_ok`](HostedPropertiesDialog::held_ok) for a dialog already open.
    pub fn hold_ok(&mut self) {
        self.hold = true;
    }

    /// The OK filed by a held dialog, once.
    pub fn take_ok(&mut self) -> Option<DialogFields> {
        self.held.take()
    }

    /// The host's answer to the held OK: `Ok` closes the dialog with the held answer in the next pass; `Err` enables
    /// OK again and shows the label under the rows until a value changes. Without a held OK it does nothing.
    pub fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.held.answer(answer);
    }

    /// `Some`: the host refuses the current values, and OK stays off with the reason shown under the rows until the
    /// values change; `None` ends the refusal.
    pub fn refuse(&mut self, reason: Option<Label>) {
        self.refusal = reason.map(|reason| (reason, self.fields.clone()));
    }

    /// The dialog has the Preview check box, starting on.
    #[must_use]
    pub fn previews(mut self) -> Self {
        self.previews = true;
        self.preview_on = true;
        self
    }

    /// Whether the dialog previews and its Preview check box is on.
    pub fn is_previewing(&self) -> bool {
        self.previews && self.preview_on
    }

    /// The position of the shown tab, which the host binds.
    pub fn tab(&self) -> usize {
        self.tab
    }

    /// Whether Reset Panel is enabled: the shown tab is resettable.
    pub fn can_reset_tab(&self) -> bool {
        self.tabs().get(self.tab).is_some_and(|tab| tab.resettable)
    }

    /// Runs Reset Panel on the shown tab, as the button does; whether it changed a value.
    pub fn reset_tab(&mut self) -> bool {
        let tabs = self.tabs();
        let Some(tab) = tabs.get(self.tab).filter(|tab| tab.resettable) else { return false };
        let before = self.fields.clone();
        PropertiesDialog::reset_tab(tab, self.tab, &mut self.fields);
        self.fields != before
    }

    /// The key of the tab at `at`, else its position, as the `tab` field names it.
    fn tab_name(tabs: &[PropertiesTab], at: usize) -> Value {
        match tabs.get(at).and_then(|tab| tab.key.clone()) {
            Some(key) => Value::from(key),
            None => Value::from(at),
        }
    }
}

impl StackDialog for HostedPropertiesDialog {
    type Result = DialogFields;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<DialogFields>> {
        let measuring = crate::runtime::measure::active(ui.ctx());
        let answers = if measuring { Vec::new() } else { self.requests.take_answers() };
        let finish = self.held.is_accepted();
        let mut dialog =
            (self.build)(Bind::bound(&mut self.fields)).tab(Bind::bound(&mut self.tab)).answers(answers).hold_ok(self.hold).finish_held(finish);
        if self.previews {
            dialog = dialog.preview(Bind::bound(&mut self.preview_on));
        }
        // The host's refusal of the held OK leaves OK enabled; its refusal of the values does not.
        if let Some((reason, refused)) = self.held.refusal() {
            dialog = dialog.refusal((reason.clone(), refused.clone()));
        } else if let Some((reason, refused)) = &self.refusal {
            dialog = dialog.refusal((reason.clone(), refused.clone())).ok_enabled(false);
        }
        // OK waits for the host's answer: a second OK cannot file over the held one.
        if self.held.is_waiting() {
            dialog = dialog.ok_enabled(false);
        }
        let mut shown = dialog.show(ui);
        if let Some(rows) = shown.rows.as_mut() {
            self.requests.absorb(rows);
        }
        if self.held.refusal().is_some_and(|(_, refused)| *refused != self.fields) {
            self.held.clear_refusal();
        }
        if self.refusal.as_ref().is_some_and(|(_, refused)| *refused != self.fields) {
            self.refusal = None;
        }
        if let Some(answer) = shown.held {
            self.held.file(answer);
        }
        match (shown.result, shown.closed) {
            (Some(result), _) => Some(DialogResult::Ok(result)),
            (None, Some(CloseReason::Activated)) if finish => Some(self.held.finish().map_or(DialogResult::Cancel, DialogResult::Ok)),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn key(&self) -> Option<&str> {
        Some(&self.key)
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn take_color_request(&mut self) -> Option<ColorRequest> {
        self.requests.take_color()
    }

    // PLAN: fl.F36 names inherent `pick_color`/`answer_color`/`cancel_color`; they clash with co.K13's trait methods,
    // so these serve, and `cancelled` drops the pending request.
    fn answer_color(&mut self, key: &str, color: craft_theme::Srgba) {
        self.requests.answer_color(key, color);
    }

    /// The values as [`DialogFields`] serializes them, plus `"tab"` (the shown tab's key, else its position) and, with
    /// the Preview check box, `"preview"`.
    fn json_fields(&self) -> Option<Value> {
        let mut fields = serde_json::to_value(&self.fields).ok()?;
        let map = fields.as_object_mut()?;
        map.insert("tab".to_owned(), Self::tab_name(&self.tabs(), self.tab));
        if self.previews {
            map.insert("preview".to_owned(), Value::Bool(self.preview_on));
        }
        Some(fields)
    }

    /// Takes `tab` (a key, or a position), `preview` (a bool), else a param field over every tab's params; inactive and
    /// unknown fields are refused.
    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        match field {
            "tab" => {
                let tabs = self.tabs();
                let at = match &value {
                    Value::String(key) => tabs.iter().position(|tab| tab.key.as_deref() == Some(key.as_str())),
                    Value::Number(at) => at.as_u64().and_then(|at| usize::try_from(at).ok()).filter(|at| *at < tabs.len()),
                    _ => None,
                };
                self.tab = at.ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
                Ok(())
            }
            "preview" if self.previews => {
                self.preview_on = value.as_bool().ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
                Ok(())
            }
            _ => set_param_field(&self.params(), &mut self.fields, field, value),
        }
    }

    /// The values, as OK answers them (empty values when read-only); none while OK is disabled. A held dialog files the
    /// answer for the host, as its button does, and answers none; once the host accepted the held answer, it answers
    /// that.
    fn confirm(&mut self, _: &Context) -> Option<DialogFields> {
        if self.held.is_accepted() {
            return self.held.finish();
        }
        if self.held.is_waiting() || self.refusal.is_some() {
            return None;
        }
        let (enabled, read_only) = {
            let dialog = (self.build)(Bind::bound(&mut self.fields));
            (dialog.ok_enabled, dialog.read_only)
        };
        if !enabled {
            return None;
        }
        let answer = if read_only { DialogFields::new() } else { self.fields.clone() };
        if self.hold && !read_only {
            self.held.file(answer);
            return None;
        }
        Some(answer)
    }

    /// Drops the pending colour requests.
    fn cancelled(&mut self, _: &Context) {
        self.requests.cancel();
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

/// The position of the shown tab, kept in the dialog's memory while it is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct OpenTab(usize);

/// The tab positions as an `ItemSource` for `TabStrip` and `SectionList`.
struct Titles<'a>(&'a [PropertiesTab]);

impl DisplayOrder<usize> for Titles<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        Some(std::borrow::Cow::Borrowed(self.0[at].title.as_str()))
    }
}

impl ItemSource<usize> for Titles<'_> {}
