//! The Find and Replace dialog: tabs, search options, Replace and Replace All.

use std::borrow::Cow;

use craft_behavior::{SelectionChange, keyed};
use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, CollectionEvent, Entry, Props as _, Show as _, ShowCollection as _, ShowContainer as _, Trigger, clicked};
use craft_ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft_ui::display::Text;
use craft_ui::layout_components::Stack;
use craft_ui::menu::Menu;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::tabs::{Tab, TabKind, TabStrip};
use craft_ui::text::TextField;
use egui::Ui;

use super::FindLabels;
use crate::find::FindOptions;

/// Which tab of Find and Replace is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
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
    /// Replace the current match, then find the next.
    #[non_exhaustive]
    Replace {
        /// The Find what text.
        query: String,
        /// The Replace with text.
        with: String,
        /// How the Find what text is read.
        options: FindOptions,
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
}

impl FindRequest {
    /// Find the next match of `query`.
    pub fn find_next(query: impl Into<String>, options: FindOptions) -> Self {
        FindRequest::FindNext { query: query.into(), options }
    }

    /// Replace the current match of `query` with `with`.
    pub fn replace(query: impl Into<String>, with: impl Into<String>, options: FindOptions) -> Self {
        FindRequest::Replace { query: query.into(), with: with.into(), options }
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
    ReplaceAll,
    Highlight(bool),
    /// PowerPoint's Replace… on the Find tab.
    ShowReplace,
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
    go_to_kinds: Vec<String>,
    message: String,
    request: Option<FindRequest>,
    /// The field that last had the keyboard: where Special inserts.
    last_field: Field,
    /// A code the Special menu chose, written into its field in the next pass.
    insert: Option<(Field, String)>,
    go_to_kind: usize,
    go_to_target: String,
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
            message: String::new(),
            request: None,
            last_field: Field::Find,
            insert: None,
            go_to_kind: 0,
            go_to_target: String::new(),
        }
    }

    /// Selects the Word layout (Find, Replace and Go To tabs, more search options) when `on`, or the PowerPoint layout
    /// when off.
    #[must_use]
    pub fn word(mut self, on: bool) -> Self {
        self.word = on;
        self
    }

    /// Sets the starting options for Match case, Whole words and the rest.
    #[must_use]
    pub fn options(mut self, options: FindOptions) -> Self {
        self.options = options;
        self
    }

    /// Go To's kinds (Page, Section, Line, Bookmark, Comment, …), the app's.
    #[must_use]
    pub fn go_to_kinds(mut self, kinds: Vec<String>) -> Self {
        self.go_to_kinds = kinds;
        self
    }

    /// The request the user made since the last call, if any.
    pub fn take_request(&mut self) -> Option<FindRequest> {
        self.request.take()
    }

    /// The status line under the buttons.
    pub fn set_message(&mut self, message: impl Into<String>) {
        self.message = message.into();
    }
}

impl StackDialog for FindReplaceDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id("find-replace")
    }

    fn show(&mut self, ui: &mut Ui, _stack: &DialogStack) -> Option<DialogResult<()>> {
        let words = Env::of(ui).words::<FindLabels>();
        let (word, tab) = (self.word, self.tab);
        let mut query = std::mem::take(&mut self.query);
        let mut with = std::mem::take(&mut self.with);
        let mut options = self.options;
        let mut more = self.more;
        let mut last_field = self.last_field;
        let mut go_kind = self.go_to_kind;
        let mut target = std::mem::take(&mut self.go_to_target);
        let mut next_tab = tab;
        let mut picked_code: Option<(Field, String)> = None;
        let mut submitted = false;
        let pending = self.insert.take();
        let kinds = self.go_to_kinds.clone();
        let message = self.message.clone();

        let accept_label = if tab == FindTab::GoTo { words.go_to.clone() } else { words.find_next.clone() };
        let extras: Vec<(Label, Press)> = match (tab, word) {
            (FindTab::Replace, _) => vec![(words.replace.clone(), Press::Replace), (words.replace_all.clone(), Press::ReplaceAll)],
            (FindTab::Find, true) => {
                vec![(words.highlight_all.clone(), Press::Highlight(true)), (words.clear_highlight.clone(), Press::Highlight(false))]
            }
            (FindTab::Find, false) => vec![(words.replace_more.clone(), Press::ShowReplace)],
            (FindTab::GoTo, _) => Vec::new(),
        };
        let accept_enabled = if tab == FindTab::GoTo { !kinds.is_empty() } else { !query.is_empty() };
        let mut buttons = vec![FooterButton::new(accept_label, DialogRole::Accept).stays_open(true).enabled(accept_enabled)];
        buttons.extend(extras.iter().map(|(label, _)| FooterButton::new(label.clone(), DialogRole::Other).stays_open(true)));
        buttons.push(FooterButton::new(words.close_button.clone(), DialogRole::Cancel));
        let close_at = buttons.len() - 1;

        let shown = Dialog::new(words.title.clone()).modeless().buttons(buttons).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
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
                        if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
                            && let Some(chosen) = keys.last()
                        {
                            next_tab = *chosen;
                        }
                    }
                }
                if tab == FindTab::GoTo {
                    Text::new(words.go_to_what.clone()).show(ui);
                    let entries: Vec<Entry<usize>> = kinds.iter().enumerate().map(|(at, kind)| Entry::new(at, kind.clone())).collect();
                    let edit = Select::new(words.go_to_what.clone(), Bind::given(go_kind.min(kinds.len().saturating_sub(1))), &entries).show(ui);
                    if let Some(value) = edit.events.value() {
                        go_kind = *value;
                    }
                    let edit = TextField::new(words.go_to.clone(), Bind::given(target.clone())).show(ui);
                    if let Some(value) = edit.events.value() {
                        target.clone_from(value);
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

                if !message.is_empty() {
                    Text::new(message.clone()).show(ui);
                }
                let check = |ui: &mut Ui, label: &Label, value: &mut bool, enabled: bool| {
                    Checkbox::new(Bind::bound(value)).label(label.clone()).enabled(enabled).show(ui);
                };
                if !word {
                    check(ui, &words.match_case.clone(), &mut options.match_case, true);
                    check(ui, &words.whole_words.clone(), &mut options.whole_word, true);
                    return;
                }
                if clicked(&Button::new(words.search_options.clone()).show(ui)) {
                    more = !more;
                }
                if !more {
                    return;
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
                let entries = special_entries(&words, tab == FindTab::Replace);
                let button = Button::new(words.special.clone()).show(ui);
                if let Some(code) = Menu::new(words.special.clone()).opened_by(&button, Trigger::Press).pick(ui, &entries).picked {
                    let field = if last_field == Field::With && tab == FindTab::Replace { Field::With } else { Field::Find };
                    picked_code = Some((field, code.to_owned()));
                }
            });
        });

        self.query = query;
        self.with = with;
        self.options = options;
        self.more = more;
        self.last_field = last_field;
        self.go_to_kind = go_kind;
        self.go_to_target = target;
        self.tab = next_tab;
        self.insert = picked_code;

        let picked = shown.output.picked;
        if picked == Some(0) || (submitted && tab != FindTab::GoTo) {
            self.request = match tab {
                FindTab::GoTo => Some(FindRequest::GoTo { kind: self.go_to_kind, target: self.go_to_target.clone() }),
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
                Press::Highlight(on) => self.request = Some(FindRequest::Highlight { query, options, on: *on }),
                Press::Replace if !query.is_empty() => self.request = Some(FindRequest::Replace { query, with, options }),
                Press::ReplaceAll if !query.is_empty() => self.request = Some(FindRequest::ReplaceAll { query, with, options }),
                Press::Replace | Press::ReplaceAll => {}
            }
        }
        (picked == Some(close_at) || shown.output.closed.is_some()).then_some(DialogResult::Ok(()))
    }
}

/// The Special menu: the `^` codes of Find what, or of Replace with.
fn special_entries(words: &FindLabels, replacing: bool) -> Vec<Entry<&'static str>> {
    let find: [(&str, &'static str); 14] = [
        (words.paragraph_mark.as_str(), "^p"),
        (words.tab_character.as_str(), "^t"),
        (words.any_character.as_str(), "^?"),
        (words.any_digit.as_str(), "^#"),
        (words.any_letter.as_str(), "^$"),
        (words.caret_character.as_str(), "^^"),
        (words.em_dash.as_str(), "^+"),
        (words.en_dash.as_str(), "^="),
        (words.nonbreaking_hyphen.as_str(), "^~"),
        (words.nonbreaking_space.as_str(), "^s"),
        (words.optional_hyphen.as_str(), "^-"),
        (words.white_space.as_str(), "^w"),
        (words.section_break.as_str(), "^b"),
        (words.line_break.as_str(), "^l"),
    ];
    let replace: [(&str, &'static str); 11] = [
        (words.paragraph_mark.as_str(), "^p"),
        (words.tab_character.as_str(), "^t"),
        (words.caret_character.as_str(), "^^"),
        (words.em_dash.as_str(), "^+"),
        (words.en_dash.as_str(), "^="),
        (words.nonbreaking_hyphen.as_str(), "^~"),
        (words.nonbreaking_space.as_str(), "^s"),
        (words.optional_hyphen.as_str(), "^-"),
        (words.line_break.as_str(), "^l"),
        (words.found_text.as_str(), "^&"),
        (words.clipboard_contents.as_str(), "^c"),
    ];
    let items: &[(&str, &'static str)] = if replacing { &replace } else { &find };
    items.iter().map(|(label, code)| Entry::new(*code, words.special_item.fill(&[("name", *label), ("code", *code)]))).collect()
}
