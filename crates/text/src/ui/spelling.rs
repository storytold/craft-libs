//! [`CheckSpellingDialog`]: Edit › Check Spelling on the dialog stack, and [`SpellingSettings`], the user's
//! dictionary as a settings section.
//!
//! The dialog walks a list of misspellings the app found (with [`crate::spell::Dictionary`] over its text objects).
//! Each button the app has to act on comes back from [`CheckSpellingDialog::take_action`]; the app changes its text,
//! finds the misspellings again and hands the dialog the new list with [`CheckSpellingDialog::set_items`]:
//!
//! ```ignore
//! dialogs.open(CheckSpellingDialog::new(find_misspellings(doc, false, &[])));
//! // each pass, while open:
//! if let Some(dialog) = dialogs.get_mut::<CheckSpellingDialog>() {
//!     if let Some(action) = dialog.take_action() {
//!         run_spelling_action(doc, &action, &mut settings);
//!         dialog.set_items(find_misspellings(doc, dialog.all_layers_checked(), dialog.ignored()));
//!     }
//! }
//! let _ = dialogs.take::<CheckSpellingDialog>(); // Done or the close box
//! ```

use std::cell::Cell;
use std::ops::Range;

use craft_behavior::{Selection, SelectionChange, keyed};
use craft_host::Section;
use craft_theme::variant::Tone;
use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{
    Bind, CollectionEvent, NoValue, Outcome, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing, clicked,
};
use craft_ui::dialog::{Dialog, DialogId, DialogOutput, DialogResult, DialogStack, StackDialog, dialog_id};
use craft_ui::display::Text;
use craft_ui::form::Field;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::{List, ListRow};
use craft_ui::notices::{Notice, NoticeLayout};
use craft_ui::runtime::env::Env;
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::text::TextField;
use egui::{Response, Ui};
use serde::{Deserialize, Serialize};

#[cfg(feature = "screen-testing")]
mod stories;

craft_ui::words! {
    /// The dialog's words, English by default.
    pub struct CheckSpellingWords {
        title: "checkSpelling.title" = "Check Spelling";
        not_in_dictionary: "checkSpelling.notInDictionary" = "Not in Dictionary:";
        change_to: "checkSpelling.changeTo" = "Change To:";
        suggestions: "checkSpelling.suggestions" = "Suggestions:";
        all_layers: "checkSpelling.allLayers" = "Check All Layers";
        complete: "checkSpelling.complete" = "Spell check complete.";
        done: "checkSpelling.done" = "Done";
        ignore: "checkSpelling.ignore" = "Ignore";
        ignore_all: "checkSpelling.ignoreAll" = "Ignore All";
        change: "checkSpelling.change" = "Change";
        change_all: "checkSpelling.changeAll" = "Change All";
        add: "checkSpelling.add" = "Add";
    }
}

/// How many suggestions the app asks [`Dictionary::suggest`](crate::spell::Dictionary::suggest) for.
pub const SUGGESTION_COUNT: usize = 8;

/// A button of the right-hand column that the dialog or the app acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Press {
    Ignore,
    IgnoreAll,
    Change,
    ChangeAll,
    Add,
    /// Check All Layers changed.
    Relist,
}

/// The user's dictionary, saved under `[spelling]`. Words are kept as added; read them through
/// [`UserWords::from_words`](crate::spell::UserWords::from_words).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct SpellingSettings {
    /// The words the user added.
    pub user_words: Vec<String>,
}

impl Section for SpellingSettings {
    const KEY: &'static str = "spelling";
}

/// One misspelling in the app's text.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SpellingItem {
    /// The text object holding it (the layer id, for example).
    pub object: u64,
    /// Byte range of the word in that object's text.
    pub range: Range<usize>,
    /// The misspelled word.
    pub word: String,
    /// At most [`SUGGESTION_COUNT`], best first.
    pub suggestions: Vec<String>,
}

/// A button the app acts on. Ignore is the dialog's own (it moves to the next item).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpellingAction {
    /// Replace this one occurrence with `to`.
    #[non_exhaustive]
    Change {
        /// The misspelling to replace.
        item: SpellingItem,
        /// The replacement text.
        to: String,
    },
    /// Replace every occurrence of `word` with `to`, in every text object when `all_layers`.
    #[non_exhaustive]
    ChangeAll {
        /// The misspelled word.
        word: String,
        /// The replacement text.
        to: String,
        /// Whether to change it in every text object, not only the current one.
        all_layers: bool,
    },
    /// Add `word` to the user's dictionary ([`SpellingSettings`]).
    Add(String),
    /// Find the misspellings again: Ignore All added a word to [`CheckSpellingDialog::ignored`],
    /// or Check All Layers changed.
    Relist,
}

impl SpellingItem {
    /// The misspelling `word` at `range` of the text object `object`, with the `suggestions` best first.
    pub fn new(object: u64, range: Range<usize>, word: impl Into<String>, suggestions: Vec<String>) -> Self {
        SpellingItem { object, range, word: word.into(), suggestions }
    }
}

/// What a spelling dialog reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct SpellingOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass (Done, Escape or the close action); `None` while it stays open.
    pub closed: Option<CloseReason>,
}

impl Outcome for SpellingOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Edit › Check Spelling. Photoshop's layout: at the left, "Not in Dictionary:" with the word (read only),
/// "Change To:" (a text field, opening with the first suggestion), "Suggestions:" (a list), Check All Layers, and
/// "Spell check complete." once the list is empty; at the right, the buttons Ignore, Ignore All, Change, Change All
/// and Add, disabled when the list is empty, and Done in the footer.
///
/// - A click on a suggestion puts it in Change To; a double click or Enter also changes.
/// - Enter is Done unless Change To has focus; Escape and the close action are Done too (nothing to undo: every
///   change was made as it was asked for). Tab walks Change To, the suggestions (one stop, arrows inside), the check
///   box and the buttons.
/// - Result: `()`; the dialog always ends with [`DialogResult::Ok`] or, closed from outside, `Cancel`.
#[derive(Clone, Debug, PartialEq, craft_ui::contract::Pattern)]
pub struct CheckSpellingDialog {
    items: Vec<SpellingItem>,
    index: usize,
    /// Ignore All words for this check.
    ignored: Vec<String>,
    change_to: String,
    all_layers: bool,
    action: Option<SpellingAction>,
    open: bool,
}

impl CheckSpellingDialog {
    /// Over `items` from a first search (the current layer: Check All Layers off).
    pub fn new(items: Vec<SpellingItem>) -> Self {
        let change_to = items.first().and_then(|i| i.suggestions.first()).cloned().unwrap_or_default();
        CheckSpellingDialog { items, index: 0, ignored: Vec::new(), change_to, all_layers: false, action: None, open: true }
    }

    /// Open with Check All Layers on.
    #[must_use]
    pub fn all_layers(mut self, on: bool) -> Self {
        self.all_layers = on;
        self
    }

    /// The button pressed this pass that the app must run, once.
    pub fn take_action(&mut self) -> Option<SpellingAction> {
        self.action.take()
    }

    /// The misspellings found again after an action. The dialog keeps its position in the list (after Change, Ignore
    /// All and Add those words are gone, so the same position is the next misspelling; earlier ones skipped with
    /// Ignore stay skipped), except after Check All Layers changed, when it starts from the first. Change To gets the
    /// current item's first suggestion.
    pub fn set_items(&mut self, items: Vec<SpellingItem>) {
        self.items = items;
        self.change_to = self.items.get(self.index).and_then(|i| i.suggestions.first()).cloned().unwrap_or_default();
    }

    /// Whether Check All Layers is on.
    pub fn all_layers_checked(&self) -> bool {
        self.all_layers
    }

    /// The words Ignore All added during this check, for the app's next search to skip.
    pub fn ignored(&self) -> &[String] {
        &self.ignored
    }

    /// Shows the dialog.
    pub fn show(&mut self, ui: &mut Ui) -> SpellingOutput {
        let words = Env::of(ui).words::<CheckSpellingWords>();
        let item = self.items.get(self.index).cloned();
        let has_item = item.is_some();
        let body_press = Cell::new(None::<Press>);
        let button_press = Cell::new(None::<Press>);
        let CheckSpellingDialog { change_to, all_layers, open, .. } = self;

        let dialog = Dialog::new(words.title.clone()).buttons([FooterButton::new(words.done.clone(), DialogRole::Accept)]).opened(Bind::bound(open));
        let shown = dialog.show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                Stack::new().show(ui, |ui, _| {
                    let word = item.as_ref().map_or_else(String::new, |i| i.word.clone());
                    Field::new(words.not_in_dictionary.clone()).id_salt("word").show(ui, |ui, _| {
                        TextField::new(words.not_in_dictionary.clone(), Bind::given(word.clone())).enabled(false).show(ui);
                    });
                    Field::new(words.change_to.clone()).id_salt("change-to").show(ui, |ui, _| {
                        TextField::new(words.change_to.clone(), Bind::bound(&mut *change_to)).enabled(has_item).show(ui);
                    });
                    Text::new(words.suggestions.clone()).show(ui);
                    let suggestions: &[String] = item.as_ref().map_or(&[], |i| i.suggestions.as_slice());
                    let positions: Vec<usize> = (0..suggestions.len()).collect();
                    let source = keyed(&positions, |at: &usize| *at);
                    let mut selection =
                        suggestions.iter().position(|s| s.as_str() == change_to.as_str()).map_or_else(Selection::default, Selection::single);
                    let events = List::new(words.suggestions.clone())
                        .select(SelectMode::Single)
                        .sizing(Sizing::Content)
                        .natural_height(true)
                        .selection(Bind::bound(&mut selection))
                        .show_items(ui, &source, &mut |at, _| ListRow::new(suggestions[*at].clone()))
                        .events;
                    for event in events {
                        match event {
                            CollectionEvent::Selection(SelectionChange::Set(rows)) => {
                                if let Some(row) = rows.last() {
                                    change_to.clone_from(&suggestions[*row]);
                                }
                            }
                            CollectionEvent::Activate(row) => {
                                change_to.clone_from(&suggestions[row]);
                                body_press.set(Some(Press::Change));
                            }
                            _ => {}
                        }
                    }
                    let before = *all_layers;
                    Checkbox::new(Bind::bound(&mut *all_layers)).label(words.all_layers.clone()).show(ui);
                    if *all_layers != before {
                        body_press.set(Some(Press::Relist));
                    }
                    if !has_item {
                        Notice::new(Tone::Neutral, words.complete.clone()).layout(NoticeLayout::Centred).show(ui);
                    }
                });
                Stack::new().show(ui, |ui, _| {
                    let buttons = [
                        (&words.ignore, Press::Ignore),
                        (&words.ignore_all, Press::IgnoreAll),
                        (&words.change, Press::Change),
                        (&words.change_all, Press::ChangeAll),
                        (&words.add, Press::Add),
                    ];
                    for (label, which) in buttons {
                        if clicked(&Button::new(label.clone()).enabled(has_item).show(ui)) {
                            button_press.set(Some(which));
                        }
                    }
                });
            });
        });
        let DialogOutput { response, closed, .. } = shown.output;

        match body_press.get().or(button_press.get()) {
            Some(Press::Ignore) => {
                self.index = (self.index + 1).min(self.items.len());
                self.change_to = self.items.get(self.index).and_then(|i| i.suggestions.first()).cloned().unwrap_or_default();
            }
            Some(Press::IgnoreAll) => {
                if let Some(item) = &item {
                    self.ignored.push(item.word.clone());
                    self.action = Some(SpellingAction::Relist);
                }
            }
            Some(Press::Change) => {
                if let Some(item) = item {
                    self.action = Some(SpellingAction::Change { item, to: self.change_to.clone() });
                }
            }
            Some(Press::ChangeAll) => {
                if let Some(item) = item {
                    self.action = Some(SpellingAction::ChangeAll { word: item.word, to: self.change_to.clone(), all_layers: self.all_layers });
                }
            }
            Some(Press::Add) => {
                if let Some(item) = item {
                    self.action = Some(SpellingAction::Add(item.word));
                }
            }
            Some(Press::Relist) => {
                self.index = 0;
                self.action = Some(SpellingAction::Relist);
            }
            None => {}
        }
        SpellingOutput { response, closed }
    }
}

impl StackDialog for CheckSpellingDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id("check-spelling")
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let shown = CheckSpellingDialog::show(self, ui);
        shown.closed.map(|_| DialogResult::Ok(()))
    }
}
