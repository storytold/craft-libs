//! The spelling review for the Craft apps: see the README.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod review;
mod ui;

pub use review::{Action, Review};
pub use ui::{Colors, ContextChoice, STRINGS, Text, context_section, panel};

/// One finding. `story` is the host's own id for a run of text; `start..end` are UTF-8 byte
/// offsets into it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Issue {
    pub story: u64,
    pub start: usize,
    pub end: usize,
    pub word: String,
    pub kind: IssueKind,
    /// The explanation to show, in the language of the text; may be empty.
    pub message: String,
    /// Empty until [`Host::suggestions`] is asked for this issue.
    pub suggestions: Vec<String>,
}

impl Issue {
    fn same_finding(&self, other: &Issue) -> bool {
        (self.story, self.start, self.end, self.kind) == (other.story, other.start, other.end, other.kind) && self.word == other.word
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IssueKind {
    #[default]
    Spelling,
    /// A preferred form or a simple rule: the word is known, so it can't join the dictionary.
    Grammar,
}

/// What the review needs from the application, for its open document.
pub trait Host {
    /// Every issue in the document in reading order. Suggestions may be left empty.
    fn check(&mut self) -> Result<Vec<Issue>, String>;
    /// The candidates for one issue, best first. An error when it no longer applies.
    fn suggestions(&mut self, issue: &Issue) -> Result<Vec<String>, String>;
    /// The current text of a story, `None` when it is gone.
    fn text(&self, story: u64) -> Option<&str>;
    /// A token that changes whenever a story's text might have, cheap enough for every frame.
    fn version(&self, story: u64) -> Option<u64>;
    /// Identifies the document: the review ends when it changes.
    fn document(&self) -> Option<u64>;
    /// Show the issue in the document, usually by selecting it.
    fn reveal(&mut self, issue: &Issue);
    /// Replace the issue's text, as one undoable edit.
    fn change(&mut self, issue: &Issue, to: &str) -> Result<(), String>;
    /// Accept the word from now on.
    fn add_word(&mut self, word: &str) -> Result<(), String>;
    /// A line above the review about what it can't cover (a language with no data), if any.
    fn notice(&self) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests;
