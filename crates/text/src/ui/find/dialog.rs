//! The Find and Replace dialog: tabs, search options, Replace and Replace All.

use std::borrow::Cow;
use std::cell::Cell;

use craft_behavior::{SelectionChange, keyed};
use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, CollectionEvent, Entry, Props as _, Show as _, ShowCollection as _, ShowContainer as _, Trigger, clicked};
use craft_ui::dialog::{
    ButtonPlace, Dialog, DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id, serde_fields, set_serde_field,
};
use craft_ui::display::Text;
use craft_ui::form::{Field as FormField, Help};
use craft_ui::menu::Menu;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::tabs::{Tab, TabKind, TabStrip};
use craft_ui::text::TextField;
use egui::{Context, Ui};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::FindLabels;
use crate::find::FindOptions;

/// Which tab of Find and Replace is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum FindTab {
    /// Find.
    #[default]
    Find,
    /// Replace.
    Replace,
    /// Go To.
    GoTo,
}

/// A check box the host adds to the search options (Search Backwards, Check Hidden Layers).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct FindCheck {
    /// The key the host reads the check's value by.
    pub key: String,
    /// The check box's label.
    pub label: Label,
    /// Whether the check is on.
    pub on: bool,
}

impl FindCheck {
    /// A check box `key` named `label`, `on` or off.
    pub fn new(key: impl Into<String>, label: impl Into<Label>, on: bool) -> Self {
        FindCheck { key: key.into(), label: label.into(), on }
    }
}

/// How the Replace tab arranges its buttons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ReplaceButtons {
    /// One Replace that replaces and finds the next.
    #[default]
    Combined,
    /// Replace (replaces only) and Replace & Find.
    Separate,
}

/// A request the dialog makes while open; the app runs it and reports back with [`FindReplaceDialog::set_message`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FindRequest {
    /// Find the next match.
    #[non_exhaustive]
    FindNext {
        /// The Find what text.
        query: String,
        /// How the Find what text is read.
        options: FindOptions,
    },
    /// Replace the current match, then find the next when `find_next`.
    #[non_exhaustive]
    Replace {
        /// The Find what text.
        query: String,
        /// The Replace with text.
        with: String,
        /// How the Find what text is read.
        options: FindOptions,
        /// `true`: replace the current match, then find the next; `false`: replace only.
        find_next: bool,
    },
    /// Replace every match.
    #[non_exhaustive]
    ReplaceAll {
        /// The Find what text.
        query: String,
        /// The Replace with text.
        with: String,
        /// How the Find what text is read.
        options: FindOptions,
    },
    /// Word's Reading Highlight: highlight every match (`on`) or clear the highlight.
    #[non_exhaustive]
    Highlight {
        /// The Find what text.
        query: String,
        /// How the Find what text is read.
        options: FindOptions,
        /// `true` to highlight every match, `false` to clear the highlight.
        on: bool,
    },
    /// Go To: the kind's index in [`FindReplaceDialog::go_to_kinds`] and the typed target.
    #[non_exhaustive]
    GoTo {
        /// Index into the Go To kinds.
        kind: usize,
        /// The typed target, such as a page number.
        target: String,
    },
    /// Next or Previous on an empty Go To target of a [`GoToTarget::Typed`] kind that steps.
    #[non_exhaustive]
    GoToStep {
        /// Index into the Go To kinds.
        kind: usize,
        /// `true` for Next, `false` for Previous.
        forward: bool,
    },
}

/// One Go To kind and its target.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct GoToKind {
    /// The kind's name in the Go to what list.
    pub label: String,
    /// What the target field is.
    pub target: GoToTarget,
}

impl GoToKind {
    /// A kind whose target is typed, with `hint` as the field's help line; with `steps`, an empty field turns the
    /// accept button into Next and adds Previous.
    pub fn typed(label: impl Into<String>, hint: impl Into<Label>, steps: bool) -> Self {
        GoToKind { label: label.into(), target: GoToTarget::Typed { hint: hint.into(), steps } }
    }

    /// A kind whose target is picked from `names`; with none, `empty` shows as the help line and Go To is disabled.
    pub fn named(label: impl Into<String>, names: Vec<String>, empty: impl Into<Label>) -> Self {
        GoToKind { label: label.into(), target: GoToTarget::Named { names, empty: empty.into() } }
    }
}

/// What a Go To kind's target is.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum GoToTarget {
    /// A text field with `hint` as its help line; with `steps`, an empty field turns the accept button into Next and
    /// adds Previous.
    #[non_exhaustive]
    Typed {
        /// The help line.
        hint: Label,
        /// Whether an empty target steps.
        steps: bool,
    },
    /// A `Select` over `names`; with none, `empty` shows as the help line and Go To is disabled.
    #[non_exhaustive]
    Named {
        /// The names to pick from.
        names: Vec<String>,
        /// The help line when there are no names.
        empty: Label,
    },
}

impl FindRequest {
    /// Find the next match of `query`.
    pub fn find_next(query: impl Into<String>, options: FindOptions) -> Self {
        FindRequest::FindNext { query: query.into(), options }
    }

    /// Replace the current match of `query` with `with`.
    pub fn replace(query: impl Into<String>, with: impl Into<String>, options: FindOptions) -> Self {
        FindRequest::Replace { query: query.into(), with: with.into(), options, find_next: true }
    }

    /// Replace the current match of `query` with `with` and stop there.
    pub fn replace_only(query: impl Into<String>, with: impl Into<String>, options: FindOptions) -> Self {
        FindRequest::Replace { query: query.into(), with: with.into(), options, find_next: false }
    }

    /// Replace every match of `query` with `with`.
    pub fn replace_all(query: impl Into<String>, with: impl Into<String>, options: FindOptions) -> Self {
        FindRequest::ReplaceAll { query: query.into(), with: with.into(), options }
    }

    /// Highlight every match of `query` (`on`) or clear the highlight.
    pub fn highlight(query: impl Into<String>, options: FindOptions, on: bool) -> Self {
        FindRequest::Highlight { query: query.into(), options, on }
    }

    /// Go To the `target` of the Go To kind at index `kind`.
    pub fn go_to(kind: usize, target: impl Into<String>) -> Self {
        FindRequest::GoTo { kind, target: target.into() }
    }

    /// Step Go To to the next (`forward`) or previous target of the kind at index `kind`.
    pub fn go_to_step(kind: usize, forward: bool) -> Self {
        FindRequest::GoToStep { kind, forward }
    }
}

/// The two text fields of the Find and Replace tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Find,
    With,
}

/// A press of the dialog's footer buttons other than Find Next and Close.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Press {
    Replace,
    /// Replace & Find, on the Separate arrangement.
    ReplaceFind,
    ReplaceAll,
    Highlight(bool),
    /// PowerPoint's Replace… on the Find tab.
    ShowReplace,
    /// Previous on an empty target of a stepping Go To kind.
    Previous,
}

/// The dialog's values as automation reads and sets them, besides the search options and the host's checks.
/// `message` is the app's report and is read only.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FindFields {
    tab: FindTab,
    find: String,
    replace: String,
    more: bool,
    go_to_kind: usize,
    go_to_target: String,
    message: String,
}

/// Find and Replace, modeless on the dialog stack (the document stays editable under it).
///
/// - Word ([`word`](Self::word) on): Find, Replace and Go To tabs; Match case, Whole words and Use wildcards
///   (wildcards grey out the other two, as Word); Search Options with Match prefix, Match suffix, Ignore
///   punctuation, Ignore white-space, Regular expressions and the Special menu of `^` codes, inserted into the field
///   that last had the keyboard.
/// - PowerPoint (off): Find what, Replace with after Replace…, Match case, Find whole words only; Find Next,
///   Replace, Replace All, Close.
/// - Enter in Find what finds the next match and keeps the keyboard there; the status line shows the app's message
///   ("All done. We made 3 replacements.").
#[derive(Debug, craft_ui::contract::Pattern)]
pub struct FindReplaceDialog {
    query: String,
    with: String,
    options: FindOptions,
    tab: FindTab,
    word: bool,
    more: bool,
    go_to_kinds: Vec<GoToKind>,
    highlighted: bool,
    message: String,
    request: Option<FindRequest>,
    /// The field that last had the keyboard: where Special inserts.
    last_field: Field,
    /// A code the Special menu chose, written into its field in the next pass.
    insert: Option<(Field, String)>,
    go_to_kind: usize,
    go_to_target: String,
    /// The host's check boxes, drawn after the layout's own options.
    checks: Vec<FindCheck>,
    /// The arrangement of the Replace buttons.
    replace_buttons: ReplaceButtons,
    /// Whether a match is current; the Replace buttons are enabled only while it is.
    matched: bool,
}

impl FindReplaceDialog {
    /// A dialog opened on `tab` with `query` in Find what.
    pub fn new(query: impl Into<String>, tab: FindTab) -> Self {
        FindReplaceDialog {
            query: query.into(),
            with: String::new(),
            options: FindOptions::default(),
            tab,
            word: false,
            more: false,
            go_to_kinds: Vec::new(),
            highlighted: false,
            message: String::new(),
            request: None,
            last_field: Field::Find,
            insert: None,
            go_to_kind: 0,
            go_to_target: String::new(),
            checks: Vec::new(),
            replace_buttons: ReplaceButtons::default(),
            matched: true,
        }
    }

    /// Selects the Word layout (Find, Replace and Go To tabs, more search options) when `on`, or the PowerPoint layout
    /// when off.
    #[must_use]
    pub fn word(mut self, on: bool) -> Self {
        self.word = on;
        self
    }

    /// Sets the starting options for Match case, Whole words and the rest. An option that only Search Options shows
    /// (Match prefix, Match suffix, Ignore punctuation, Ignore white-space, Regular expressions) opens Search Options,
    /// so an option that is on is never hidden.
    #[must_use]
    pub fn options(mut self, options: FindOptions) -> Self {
        self.options = options;
        self.more |= options.prefix || options.suffix || options.ignore_punctuation || options.ignore_space || options.regex;
        self
    }

    /// Shows `tab` in the open dialog, and puts `query` in Find what when given (Find or Replace asked again with a
    /// selection). Replace with, the options, Search Options and the Go To target stay as they are.
    pub fn show_tab(&mut self, tab: FindTab, query: Option<String>) {
        self.tab = tab;
        if let Some(query) = query {
            self.query = query;
        }
    }

    /// Replaces Go To's kinds in the open dialog (the document's bookmarks changed, say); the chosen kind and target
    /// stay.
    pub fn set_go_to_kinds(&mut self, kinds: Vec<GoToKind>) {
        self.go_to_kinds = kinds;
    }

    /// Go To's kinds (Page, Section, Line, Bookmark, Comment, …), the app's.
    #[must_use]
    pub fn go_to_kinds(mut self, kinds: Vec<GoToKind>) -> Self {
        self.go_to_kinds = kinds;
        self
    }

    /// The host's check boxes, drawn after the layout's own options, in order, in both layouts.
    #[must_use]
    pub fn checks(mut self, checks: Vec<FindCheck>) -> Self {
        self.checks = checks;
        self
    }

    /// The current value of the host check `key`.
    pub fn checked(&self, key: &str) -> Option<bool> {
        self.checks.iter().find(|check| check.key == key).map(|check| check.on)
    }

    /// The arrangement of the Replace buttons on the Replace tab, both layouts.
    #[must_use]
    pub fn replace_buttons(mut self, buttons: ReplaceButtons) -> Self {
        self.replace_buttons = buttons;
        self
    }

    /// Opens with `with` in Replace with (the host's last search).
    #[must_use]
    pub fn replace_with(mut self, with: impl Into<String>) -> Self {
        self.with = with.into();
        self
    }

    /// Whether a match is current; the Replace buttons are enabled only while it is. Starts `true`.
    pub fn set_matched(&mut self, matched: bool) {
        self.matched = matched;
    }

    /// The search options the shown layout has, by their serde names: PowerPoint's layout has Match case and Whole
    /// words only.
    fn option_fields(&self) -> Map<String, Value> {
        let Ok(Value::Object(mut fields)) = serde_json::to_value(self.options) else { return Map::new() };
        if !self.word {
            fields.retain(|name, _| matches!(name.as_str(), "matchCase" | "wholeWord"));
        }
        fields
    }

    /// The Find tab opens with Reading Highlight on (Clear highlight enabled); the app passes its session state.
    #[must_use]
    pub fn highlighted(mut self, on: bool) -> Self {
        self.highlighted = on;
        self
    }

    /// The request the user made since the last call, if any.
    pub fn take_request(&mut self) -> Option<FindRequest> {
        self.request.take()
    }

    fn fields(&self) -> FindFields {
        FindFields {
            tab: self.tab,
            find: self.query.clone(),
            replace: self.with.clone(),
            more: self.more,
            go_to_kind: self.go_to_kind,
            go_to_target: self.go_to_target.clone(),
            message: self.message.clone(),
        }
    }

    /// Takes every value of `fields` but the message, which only the app sets.
    fn apply(&mut self, fields: FindFields) {
        self.tab = fields.tab;
        self.query = fields.find;
        self.with = fields.replace;
        self.more = fields.more;
        self.go_to_kind = fields.go_to_kind;
        self.go_to_target = fields.go_to_target;
    }

    /// The status line under the options, the last line of the body.
    pub fn set_message(&mut self, message: impl Into<String>) {
        self.message = message.into();
    }
}

impl StackDialog for FindReplaceDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id("find-replace")
    }

    fn modeless(&self) -> bool {
        true
    }

    /// The dialog's fields, the search options the shown layout has by their serde names, and each host check by its key.
    fn json_fields(&self) -> Option<Value> {
        let Some(Value::Object(mut fields)) = serde_fields(&self.fields()) else { return None };
        fields.extend(self.option_fields());
        fields.extend(self.checks.iter().map(|check| (check.key.clone(), Value::Bool(check.on))));
        Some(Value::Object(fields))
    }

    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        if field == "message" {
            return Err(FieldRefusal::ReadOnly.message(field));
        }
        if let Some(check) = self.checks.iter_mut().find(|check| check.key == field) {
            check.on = value.as_bool().ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
            return Ok(());
        }
        if self.option_fields().contains_key(field) {
            return set_serde_field(&mut self.options, field, value);
        }
        let mut fields = self.fields();
        set_serde_field(&mut fields, field, value)?;
        self.apply(fields);
        Ok(())
    }

    fn show(&mut self, ui: &mut Ui, _stack: &DialogStack) -> Option<DialogResult<()>> {
        let words = Env::of(ui).words::<FindLabels>();
        let (word, tab) = (self.word, self.tab);
        let mut query = std::mem::take(&mut self.query);
        let mut with = std::mem::take(&mut self.with);
        let mut options = self.options;
        let mut more = self.more;
        let mut last_field = self.last_field;
        let mut target = std::mem::take(&mut self.go_to_target);
        let mut next_tab = tab;
        let mut picked_code: Option<(Field, String)> = None;
        let mut submitted = false;
        let pending = self.insert.take();
        let kinds = self.go_to_kinds.clone();
        let message = self.message.clone();
        let matched = self.matched;
        let separate = self.replace_buttons == ReplaceButtons::Separate;
        let mut checks = std::mem::take(&mut self.checks);
        let highlighted = self.highlighted;
        let kind_at = self.go_to_kind.min(kinds.len().saturating_sub(1));
        let mut go_kind = kind_at;
        let go_target = kinds.get(kind_at).map(|kind| &kind.target);
        // An empty target of a stepping kind steps: Next takes the accept button, Previous sits beside it.
        let stepping = matches!(go_target, Some(GoToTarget::Typed { steps: true, .. })) && target.trim().is_empty();

        let accept_label = match tab {
            FindTab::GoTo if stepping => words.next.clone(),
            FindTab::GoTo => words.go_to.clone(),
            _ => words.find_next.clone(),
        };
        let extras: Vec<(Label, Press)> = match (tab, word) {
            (FindTab::Replace, _) => {
                let mut extras = vec![(words.replace.clone(), Press::Replace)];
                if separate {
                    extras.push((words.replace_find.clone(), Press::ReplaceFind));
                }
                extras.push((words.replace_all.clone(), Press::ReplaceAll));
                extras
            }
            (FindTab::Find, true) => {
                vec![(words.highlight_all.clone(), Press::Highlight(true)), (words.clear_highlight.clone(), Press::Highlight(false))]
            }
            (FindTab::Find, false) => vec![(words.replace_more.clone(), Press::ShowReplace)],
            (FindTab::GoTo, _) if stepping => vec![(words.previous.clone(), Press::Previous)],
            (FindTab::GoTo, _) => Vec::new(),
        };
        let accept_enabled = match tab {
            FindTab::GoTo => match go_target {
                None => false,
                Some(GoToTarget::Named { names, .. }) => !names.is_empty(),
                Some(_) => true,
            },
            _ => !query.is_empty(),
        };
        // The Find tab's accept button follows the Find what text as the body left it, not as the last pass did.
        let ready = Cell::new(accept_enabled);
        let mut buttons = vec![FooterButton::new(accept_label, DialogRole::Accept).stays_open(true).enabled(accept_enabled)];
        buttons.extend(extras.iter().map(|(label, press)| {
            let needs_match = matches!(press, Press::Replace | Press::ReplaceFind);
            let enabled = (matched || !needs_match) && (highlighted || *press != Press::Highlight(false));
            FooterButton::new(label.clone(), DialogRole::Other).stays_open(true).enabled(enabled)
        }));
        buttons.push(FooterButton::new(words.close_button.clone(), DialogRole::Cancel));
        let close_at = buttons.len() - 1;

        let shown = Dialog::new(words.title.clone())
            .modeless()
            .button_place(ButtonPlace::Side)
            .buttons(buttons)
            .settle_buttons(|_, buttons| {
                if let Some(accept) = buttons.get_mut(0) {
                    accept.set_enabled(ready.get());
                }
            })
            .show(ui, |ui, _| {
                if word {
                    let tabs = [
                        (FindTab::Find, words.find_tab.as_str().to_owned()),
                        (FindTab::Replace, words.replace_tab.as_str().to_owned()),
                        (FindTab::GoTo, words.go_to_tab.as_str().to_owned()),
                    ];
                    let source = keyed(&tabs[..], |tab: &(FindTab, String)| tab.0).with_text(|tab| Cow::Borrowed(tab.1.as_str()));
                    let strip = TabStrip::new(words.title.clone()).kind(TabKind::Text).shown(tab).natural_height(true);
                    let out = strip.show_items(ui, &source, &mut |key, _| {
                        Tab::new(tabs.iter().find(|tab| tab.0 == *key).map_or_else(String::new, |tab| tab.1.clone()))
                    });
                    for event in out.response.events {
                        if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
                            && let Some(chosen) = keys.last()
                        {
                            next_tab = *chosen;
                        }
                    }
                }
                if tab == FindTab::GoTo {
                    Text::new(words.go_to_what.clone()).show(ui);
                    let entries: Vec<Entry<usize>> = kinds.iter().enumerate().map(|(at, kind)| Entry::new(at, kind.label.clone())).collect();
                    let edit = Select::new(words.go_to_what.clone(), Bind::given(kind_at), &entries).show(ui);
                    if let Some(value) = edit.events.value() {
                        go_kind = *value;
                    }
                    if let Some(kind) = kinds.get(kind_at) {
                        match &kind.target {
                            GoToTarget::Typed { hint, .. } => {
                                let mut field = FormField::new(Label::from(kind.label.clone())).id_salt("go-to-target");
                                if !hint.as_str().is_empty() {
                                    field = field.help(Help::Text(hint.clone()));
                                }
                                field.show(ui, |ui, _| {
                                    let edit = TextField::new(Label::from(kind.label.clone()), Bind::given(target.clone())).show(ui);
                                    if let Some(value) = edit.events.value() {
                                        target.clone_from(value);
                                    }
                                });
                            }
                            GoToTarget::Named { names, empty } => match names.iter().position(|name| *name == target).or(names.first().map(|_| 0)) {
                                None => {
                                    FormField::new(Label::from(kind.label.clone())).id_salt("go-to-target").help(Help::Text(empty.clone())).show(
                                        ui,
                                        |ui, _| {
                                            TextField::new(Label::from(kind.label.clone()), Bind::given(String::new())).enabled(false).show(ui);
                                        },
                                    );
                                }
                                Some(at) => {
                                    // The field always names one of the names, so Go To has a target before the user picks.
                                    target.clone_from(&names[at]);
                                    let entries: Vec<Entry<usize>> =
                                        names.iter().enumerate().map(|(at, name)| Entry::new(at, name.clone())).collect();
                                    let edit = Select::new(Label::from(kind.label.clone()), Bind::given(at), &entries).show(ui);
                                    if let Some(&picked) = edit.events.value()
                                        && let Some(name) = names.get(picked)
                                    {
                                        target.clone_from(name);
                                    }
                                }
                            },
                        }
                    }
                    if !message.is_empty() {
                        Text::new(message.clone()).show(ui);
                    }
                    return;
                }

                Text::new(words.find_what.clone()).show(ui);
                let mut field = TextField::new(words.find_what.clone(), Bind::given(query.clone()));
                if let Some((Field::Find, code)) = &pending {
                    field = field.replace_selection(code.clone());
                }
                let edit = field.show(ui);
                if edit.response.has_focus() {
                    last_field = Field::Find;
                }
                if let Some(value) = edit.events.value() {
                    query.clone_from(value);
                }
                ready.set(!query.is_empty());
                submitted |= edit.submitted;
                if tab == FindTab::Replace {
                    Text::new(words.replace_with.clone()).show(ui);
                    let mut field = TextField::new(words.replace_with.clone(), Bind::given(with.clone()));
                    if let Some((Field::With, code)) = &pending {
                        field = field.replace_selection(code.clone());
                    }
                    let edit = field.show(ui);
                    if edit.response.has_focus() {
                        last_field = Field::With;
                    }
                    if let Some(value) = edit.events.value() {
                        with.clone_from(value);
                    }
                }

                let check = |ui: &mut Ui, label: &Label, value: &mut bool, enabled: bool| {
                    Checkbox::new(Bind::bound(value)).label(label.clone()).enabled(enabled).show(ui);
                };
                'options: {
                    if !word {
                        check(ui, &words.match_case.clone(), &mut options.match_case, true);
                        check(ui, &words.whole_words.clone(), &mut options.whole_word, true);
                        break 'options;
                    }
                    if clicked(&Button::new(words.search_options.clone()).show(ui)) {
                        more = !more;
                    }
                    if !more {
                        break 'options;
                    }
                    // Word greys the case, word and ignore options out while wildcards are on.
                    let free = !options.wildcards || options.regex;
                    let no_regex = !options.regex;
                    check(ui, &words.match_case.clone(), &mut options.match_case, free);
                    check(ui, &words.whole_words.clone(), &mut options.whole_word, free);
                    check(ui, &words.use_wildcards.clone(), &mut options.wildcards, no_regex);
                    check(ui, &words.match_prefix.clone(), &mut options.prefix, free);
                    check(ui, &words.match_suffix.clone(), &mut options.suffix, free);
                    check(ui, &words.ignore_punctuation.clone(), &mut options.ignore_punctuation, free);
                    check(ui, &words.ignore_space.clone(), &mut options.ignore_space, free);
                    check(ui, &words.regular_expressions.clone(), &mut options.regex, true);
                    let entries = special_entries(&words, tab == FindTab::Replace, options.wildcards);
                    let button = Button::new(words.special.clone()).show(ui);
                    if let Some(code) = Menu::new(words.special.clone()).opened_by(&button, Trigger::Press).pick(ui, &entries).picked {
                        let field = if last_field == Field::With && tab == FindTab::Replace { Field::With } else { Field::Find };
                        picked_code = Some((field, code.to_owned()));
                    }
                }
                if !word || more {
                    for host in &mut checks {
                        Checkbox::new(Bind::bound(&mut host.on)).label(host.label.clone()).show(ui);
                    }
                }
                if !message.is_empty() {
                    Text::new(message.clone()).show(ui);
                }
            });

        self.query = query;
        self.with = with;
        self.options = options;
        self.more = more;
        self.last_field = last_field;
        if go_kind != kind_at {
            target.clear();
            self.message.clear();
        }
        self.go_to_kind = go_kind;
        self.go_to_target = target;
        self.tab = next_tab;
        self.insert = picked_code;
        self.checks = checks;

        let picked = shown.output.picked;
        if picked == Some(0) || (submitted && tab != FindTab::GoTo) {
            // The target as the body left it decides whether Go To steps.
            let steps_now = matches!(go_target, Some(GoToTarget::Typed { steps: true, .. })) && self.go_to_target.trim().is_empty();
            self.request = match tab {
                FindTab::GoTo if kinds.is_empty() => None,
                FindTab::GoTo if steps_now => Some(FindRequest::go_to_step(kind_at, true)),
                FindTab::GoTo => Some(FindRequest::go_to(kind_at, self.go_to_target.clone())),
                _ if self.query.is_empty() => None,
                _ => Some(FindRequest::FindNext { query: self.query.clone(), options: self.options }),
            };
        }
        if let Some(index) = picked.and_then(|at| at.checked_sub(1))
            && let Some((_, which)) = extras.get(index)
        {
            let (query, with, options) = (self.query.clone(), self.with.clone(), self.options);
            match which {
                Press::ShowReplace => self.tab = FindTab::Replace,
                Press::Highlight(on) => {
                    self.highlighted = *on;
                    self.request = Some(FindRequest::Highlight { query, options, on: *on });
                }
                Press::Previous => self.request = Some(FindRequest::go_to_step(kind_at, false)),
                Press::Replace if !query.is_empty() && separate => self.request = Some(FindRequest::replace_only(query, with, options)),
                Press::Replace | Press::ReplaceFind if !query.is_empty() => self.request = Some(FindRequest::replace(query, with, options)),
                Press::ReplaceAll if !query.is_empty() => self.request = Some(FindRequest::replace_all(query, with, options)),
                Press::Replace | Press::ReplaceFind | Press::ReplaceAll => {}
            }
        }
        (picked == Some(close_at) || shown.output.closed.is_some()).then_some(DialogResult::Ok(()))
    }
}

/// A Special menu item: the word that names it and the code it inserts.
type Code = (fn(&FindLabels) -> &str, &'static str);

/// Find what without wildcards.
const FIND_CODES: [Code; 19] = [
    (|w| w.paragraph_mark.as_str(), "^p"),
    (|w| w.tab_character.as_str(), "^t"),
    (|w| w.any_character.as_str(), "^?"),
    (|w| w.any_digit.as_str(), "^#"),
    (|w| w.any_letter.as_str(), "^$"),
    (|w| w.caret_character.as_str(), "^^"),
    (|w| w.column_break.as_str(), "^n"),
    (|w| w.em_dash.as_str(), "^+"),
    (|w| w.en_dash.as_str(), "^="),
    (|w| w.endnote_mark.as_str(), "^e"),
    (|w| w.footnote_mark.as_str(), "^f"),
    (|w| w.graphic.as_str(), "^g"),
    (|w| w.line_break.as_str(), "^l"),
    (|w| w.page_break.as_str(), "^m"),
    (|w| w.nonbreaking_hyphen.as_str(), "^~"),
    (|w| w.nonbreaking_space.as_str(), "^s"),
    (|w| w.optional_hyphen.as_str(), "^-"),
    (|w| w.section_break.as_str(), "^b"),
    (|w| w.white_space.as_str(), "^w"),
];

/// Find what with wildcards: the wildcard expressions and the codes that still apply.
const FIND_WILDCARD_CODES: [Code; 23] = [
    (|w| w.paragraph_mark.as_str(), "^p"),
    (|w| w.tab_character.as_str(), "^t"),
    (|w| w.any_single_character.as_str(), "?"),
    (|w| w.any_run.as_str(), "*"),
    (|w| w.any_digit.as_str(), "[0-9]"),
    (|w| w.any_letter.as_str(), "[a-zA-Z]"),
    (|w| w.word_start.as_str(), "<"),
    (|w| w.word_end.as_str(), ">"),
    (|w| w.caret_character.as_str(), "^^"),
    (|w| w.column_break.as_str(), "^n"),
    (|w| w.em_dash.as_str(), "^+"),
    (|w| w.en_dash.as_str(), "^="),
    (|w| w.endnote_mark.as_str(), "^e"),
    (|w| w.footnote_mark.as_str(), "^f"),
    (|w| w.graphic.as_str(), "^g"),
    (|w| w.line_break.as_str(), "^l"),
    (|w| w.page_break.as_str(), "^m"),
    (|w| w.nonbreaking_hyphen.as_str(), "^~"),
    (|w| w.nonbreaking_space.as_str(), "^s"),
    (|w| w.optional_hyphen.as_str(), "^-"),
    (|w| w.section_break.as_str(), "^b"),
    (|w| w.white_space.as_str(), "^w"),
    (|w| w.field_code.as_str(), "^d"),
];

/// Replace with.
const REPLACE_CODES: [Code; 15] = [
    (|w| w.paragraph_mark.as_str(), "^p"),
    (|w| w.tab_character.as_str(), "^t"),
    (|w| w.caret_character.as_str(), "^^"),
    (|w| w.column_break.as_str(), "^n"),
    (|w| w.em_dash.as_str(), "^+"),
    (|w| w.en_dash.as_str(), "^="),
    (|w| w.graphic.as_str(), "^g"),
    (|w| w.line_break.as_str(), "^l"),
    (|w| w.page_break.as_str(), "^m"),
    (|w| w.nonbreaking_hyphen.as_str(), "^~"),
    (|w| w.nonbreaking_space.as_str(), "^s"),
    (|w| w.optional_hyphen.as_str(), "^-"),
    (|w| w.section_break.as_str(), "^b"),
    (|w| w.found_text.as_str(), "^&"),
    (|w| w.clipboard_contents.as_str(), "^c"),
];

/// With wildcards on, Replace with starts with the back-reference to the Find what expression.
const FIND_EXPRESSION: Code = (|w| w.find_expression.as_str(), "\\1");

/// The Special menu: the `^` codes of Find what, or of Replace with, by whether wildcards are on.
fn special_entries(words: &FindLabels, replacing: bool, wildcards: bool) -> Vec<Entry<&'static str>> {
    let table: Vec<&Code> = match (replacing, wildcards) {
        (false, false) => FIND_CODES.iter().collect(),
        (false, true) => FIND_WILDCARD_CODES.iter().collect(),
        (true, false) => REPLACE_CODES.iter().collect(),
        (true, true) => std::iter::once(&FIND_EXPRESSION).chain(REPLACE_CODES.iter()).collect(),
    };
    table.into_iter().map(|(name, code)| Entry::new(*code, words.special_item.fill(&[("name", name(words)), ("code", *code)]))).collect()
}
