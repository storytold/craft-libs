//! The completion list of a text part (18.2): candidates from the options' `Completer` for the draft and caret, and
//! acceptance by replacing the candidate's range. The list's popup is a dropdown: the host hands the part's list keys
//! to it, its lead is the highlight, and closing it hides the list until the draft changes.

use super::byte_of;
use crate::primitives::{Completer, Completion};
use std::ops::Range;

/// What the adapter keeps of the list between passes.
#[derive(Clone, Default)]
pub(crate) struct Completions {
    /// The candidates shown; empty while the list is closed.
    pub items: Vec<Completion>,
    /// The list has candidates to show: the part holds Escape for the popup while this is set.
    pub open: bool,
    /// The popup was asked to open in the last pass.
    requested: bool,
    /// A candidate the list activated (Enter, Tab or a click), accepted by the next pass.
    pub picked: Option<usize>,
    /// The bytes of `for_text` the candidates replace.
    range: Option<Range<usize>>,
    /// The draft the candidates were computed for.
    for_text: String,
    /// The draft Escape hid the list for; the list stays hidden until the draft changes.
    dismissed: Option<String>,
}

impl Completions {
    fn close(&mut self) {
        self.items.clear();
        self.open = false;
        self.range = None;
        self.for_text.clear();
    }

    /// Reads the candidate the list activated, against the draft `text`. Returns the bytes of `text` the accepted
    /// candidate replaces and the text that replaces them.
    pub(crate) fn take(&mut self, text: &str) -> Option<(Range<usize>, String)> {
        let at = self.picked.take().filter(|_| self.open)?;
        if self.for_text != text {
            return None;
        }
        let range = self.range.clone()?;
        let insert = self.items.get(at)?.insert.clone();
        // What the draft becomes stays hidden: an accepted word does not offer itself again.
        self.dismissed = Some(format!("{}{insert}{}", text.get(..range.start)?, text.get(range.end..)?));
        self.close();
        Some((range, insert))
    }

    /// Reconciles the list with its popup before the pass opens or closes it: `shown` is whether the popup is open.
    /// A popup closed while it was asked to show (Escape, a press outside) hides the list until the draft changes.
    /// Returns whether the popup should open.
    pub(crate) fn sync(&mut self, shown: bool) -> bool {
        if self.requested && !shown && self.open {
            self.dismissed = Some(std::mem::take(&mut self.for_text));
            self.close();
        }
        self.requested = self.open;
        self.open
    }

    /// Asks `completer` for the candidates of the draft and caret in `at` (the text and the caret in characters; `None`
    /// while the part is not being typed in).
    pub(crate) fn refresh(&mut self, completer: Option<&dyn Completer>, at: Option<(&str, usize)>) {
        let Some((text, caret)) = at else {
            self.close();
            return;
        };
        if self.dismissed.as_deref().is_some_and(|dismissed| dismissed != text) {
            self.dismissed = None;
        }
        let mut items = Vec::new();
        let range = completer.and_then(|completer| completer.complete(text, byte_of(text, caret), &mut items));
        let Some(range) = range.filter(|_| !items.is_empty() && self.dismissed.is_none()) else {
            self.close();
            return;
        };
        self.items = items;
        self.range = Some(range);
        self.for_text = text.to_owned();
        self.open = true;
    }
}
