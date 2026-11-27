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
use craft_ui::choice::Checkbox;
use craft_ui::contract::{
    Bind, CollectionEvent, NoValue, Outcome, Pad, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing,
};
use craft_ui::dialog::{ButtonPlace, Dialog, DialogId, DialogOutput, DialogResult, DialogStack, StackDialog, dialog_id};
use craft_ui::display::Text;
use craft_ui::form::Field;
use craft_ui::list::{List, ListRow};
use craft_ui::notices::{Notice, NoticeLayout};
use craft_ui::runtime::env::Env;
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::style::SurfaceKind;
use craft_ui::text::TextField;
use egui::{Context, Response, Ui};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[cfg(feature = "suite")]
pub(crate) mod stories;

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

/// A button of the side column, or a body change, that the dialog or the app acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Press {
    Ignore,
    IgnoreAll,
    Change,
    ChangeAll,
    Add,
    /// Check All Layers changed to this.
    Relist(bool),
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
    /// The sentence's text before the word; empty with `after` for the word alone.
    pub before: String,
    /// The sentence's text after the word.
    pub after: String,
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
    /// Replace the listed `item`'s word with `to`: in that occurrence's text object, or in every text object when
    /// `all_layers`.
    #[non_exhaustive]
    ChangeAll {
        /// The listed misspelling the button was pressed on.
        item: SpellingItem,
        /// The replacement text.
        to: String,
        /// Whether to change it in every text object, not only the current one.
        all_layers: bool,
    },
    /// Add `word` to the user's dictionary ([`SpellingSettings`]).
    Add(String),
    /// Ignore reports the item it skipped; the dialog still moves to the next item itself.
    Ignore(SpellingItem),
    /// Find the misspellings again: Ignore All added a word to [`CheckSpellingDialog::ignored`],
    /// or Check All Layers changed.
    Relist,
}

impl SpellingItem {
    /// The misspelling `word` at `range` of the text object `object`, with the `suggestions` best first.
    pub fn new(object: u64, range: Range<usize>, word: impl Into<String>, suggestions: Vec<String>) -> Self {
        SpellingItem { object, range, word: word.into(), suggestions, before: String::new(), after: String::new() }
    }

    /// Sets the sentence's text before and after the word.
    #[must_use]
    pub fn context(mut self, before: impl Into<String>, after: impl Into<String>) -> Self {
        self.before = before.into();
        self.after = after.into();
        self
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
/// "Spell check complete." once the list is empty; at the right, a column of Done and the buttons Ignore, Ignore All,
/// Change, Change All and Add, which are disabled when the list is empty.
///
/// - A click on a suggestion puts it in Change To; a double click or Enter also changes.
/// - Enter is Done unless Change To has focus; Escape and the close action are Done too (nothing to undo: every
///   change was made as it was asked for). Tab walks Change To, the suggestions (one stop, arrows inside), the check
///   box and the buttons.
/// - Result: `()`; every close from inside (Done, Escape, the close action, automation's confirm) is
///   [`DialogResult::Ok`], and a close from outside is `Cancel`.
/// - Automation fields: `word` and `count` (read only), `changeTo` and `allLayers`.
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
    /// The misspellings the dialog lists.
    pub fn items(&self) -> &[SpellingItem] {
        &self.items
    }

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

    /// Replaces the list and shows its first item (Change To from its first suggestion): a host that walks its text
    /// itself hands the dialog the words from its current place.
    pub fn restart(&mut self, items: Vec<SpellingItem>) {
        self.action = None;
        self.index = 0;
        self.set_items(items);
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

    /// Sets Check All Layers to `on`. A change starts the list again and asks the app to find the misspellings anew;
    /// the check box and automation both come here.
    fn relist_all_layers(&mut self, on: bool) {
        if self.all_layers != on {
            self.all_layers = on;
            self.index = 0;
            self.action = Some(SpellingAction::Relist);
        }
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
        let CheckSpellingDialog { change_to, all_layers, open, .. } = self;

        let actions = [
            (&words.ignore, Press::Ignore),
            (&words.ignore_all, Press::IgnoreAll),
            (&words.change, Press::Change),
            (&words.change_all, Press::ChangeAll),
            (&words.add, Press::Add),
        ];
        let buttons = std::iter::once(FooterButton::new(words.done.clone(), DialogRole::Accept))
            .chain(actions.iter().map(|(label, _)| FooterButton::new((*label).clone(), DialogRole::Other).stays_open(true).enabled(has_item)));
        let dialog = Dialog::new(words.title.clone()).button_place(ButtonPlace::Side).buttons(buttons).opened(Bind::bound(open));
        let shown = dialog.show(ui, |ui, _| {
            let word = item.as_ref().map_or_else(String::new, |i| i.word.clone());
            Field::new(words.not_in_dictionary.clone()).id_salt("word").show(ui, |ui, _| {
                // PLAN: the word should be strong, underlined and in the accent ink inside the sunken field; no display
                // component takes a run of `Span`s (`Text::spans` wants `DocRun`s with fonts and data colours) and `Span`
                // has no colour (contract-03 OQ1), so the context reads as one wrapped line.
                match item.as_ref().filter(|i| !i.before.is_empty() || !i.after.is_empty()) {
                    Some(i) => {
                        Text::new(format!("{}{}{}", i.before, i.word, i.after)).show(ui);
                    }
                    None => {
                        TextField::new(words.not_in_dictionary.clone(), Bind::given(word.clone())).enabled(false).show(ui);
                    }
                }
            });
            Field::new(words.change_to.clone()).id_salt("change-to").show(ui, |ui, _| {
                TextField::new(words.change_to.clone(), Bind::bound(&mut *change_to)).enabled(has_item).show(ui);
            });
            Text::new(words.suggestions.clone()).show(ui);
            let suggestions: &[String] = item.as_ref().map_or(&[], |i| i.suggestions.as_slice());
            let positions: Vec<usize> = (0..suggestions.len()).collect();
            let source = keyed(&positions, |at: &usize| *at);
            let mut selection = suggestions.iter().position(|s| s.as_str() == change_to.as_str()).map_or_else(Selection::default, Selection::single);
            let events = List::new(words.suggestions.clone())
                .select(SelectMode::Single)
                .sizing(Sizing::FillWidth)
                .surface(SurfaceKind::Sunken)
                .pad(Pad::Control)
                .natural_height(true)
                .selection(Bind::bound(&mut selection))
                .show_items(ui, &source, &mut |at, _| ListRow::new(suggestions[*at].clone()))
                .events;
            for event in events {
                match event {
                    CollectionEvent::Selection { change: SelectionChange::Set(rows), .. } => {
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
            let mut checked = *all_layers;
            Checkbox::new(Bind::bound(&mut checked)).label(words.all_layers.clone()).show(ui);
            if checked != *all_layers {
                body_press.set(Some(Press::Relist(checked)));
            }
            if !has_item {
                Notice::new(Tone::Neutral, words.complete.clone()).layout(NoticeLayout::Centred).show(ui);
            }
        });
        let DialogOutput { response, closed, picked } = shown.output;
        let button_press = picked.and_then(|at| actions.get(at.checked_sub(1)?)).map(|(_, which)| *which);

        match body_press.get().or(button_press) {
            Some(Press::Ignore) => {
                if let Some(item) = item {
                    self.action = Some(SpellingAction::Ignore(item));
                }
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
                    self.action = Some(SpellingAction::ChangeAll { item, to: self.change_to.clone(), all_layers: self.all_layers });
                }
            }
            Some(Press::Add) => {
                if let Some(item) = item {
                    self.action = Some(SpellingAction::Add(item.word));
                }
            }
            Some(Press::Relist(on)) => self.relist_all_layers(on),
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

    fn json_fields(&self) -> Option<Value> {
        let word = self.items.get(self.index).map(|item| item.word.as_str());
        Some(json!({ "word": word, "changeTo": self.change_to, "allLayers": self.all_layers, "count": self.items.len() }))
    }

    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        match field {
            "changeTo" => {
                if self.items.is_empty() {
                    return Err("`changeTo` takes no input: the list of misspellings is empty".to_owned());
                }
                let text = value.as_str().ok_or_else(|| "`changeTo` takes a string".to_owned())?;
                self.change_to = text.to_owned();
                Ok(())
            }
            "allLayers" => {
                let on = value.as_bool().ok_or_else(|| "`allLayers` takes true or false".to_owned())?;
                self.relist_all_layers(on);
                Ok(())
            }
            "word" | "count" => Err(format!("`{field}` is read only")),
            _ => Err(format!("unknown field `{field}`: this dialog has `word`, `changeTo`, `allLayers` and `count`")),
        }
    }

    fn confirm(&mut self, _: &Context) -> Option<()> {
        Some(())
    }
}
