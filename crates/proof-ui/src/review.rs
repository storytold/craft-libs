//! A pass over a document's issues, apart from how it is drawn.
use std::collections::{HashMap, HashSet};

use crate::{Host, Issue, IssueKind};

pub(crate) const CHANGED: &str = "The text changed. Check again to continue.";
pub(crate) const OTHER_DOCUMENT: &str = "The document changed.";
pub(crate) const SAME_REPLACEMENT: &str = "Choose a suggestion or enter a different replacement.";
pub(crate) const NO_ISSUE: &str = "No issue to review.";
pub(crate) const NOT_SPELLING: &str = "Only spelling issues can be added to the dictionary.";

/// What can be done with the current issue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Replace it. `None` takes the typed replacement, which starts as the first suggestion.
    Change(Option<String>),
    /// Skip this occurrence for this pass.
    IgnoreOnce,
    /// Skip this word for this pass.
    IgnoreAll,
    AddToDictionary,
    /// Start a new pass from the top, forgetting what was ignored.
    Check,
}

/// One pass over a document. Empty until [`Review::start`].
#[derive(Clone, Debug, Default)]
pub struct Review {
    document: Option<u64>,
    /// The issues left, the current one first. Only the current one has suggestions.
    issues: Vec<Issue>,
    skipped: Vec<Issue>,
    ignored: HashSet<String>,
    /// [`Host::version`] of each issue's story when it was checked.
    versions: HashMap<u64, u64>,
    pub(crate) before: String,
    pub(crate) after: String,
    /// The Change… field's text.
    pub replacement: String,
    pub(crate) manual: bool,
    message: String,
    started: bool,
}

impl Review {
    /// Check the document from the top, forgetting any earlier pass.
    pub fn start(&mut self, host: &mut dyn Host) {
        *self = Review { started: true, document: host.document(), ..Default::default() };
        // A failure is kept as the message.
        let _ = self.report(|review| review.refresh(host));
    }

    pub fn started(&self) -> bool {
        self.started
    }

    pub fn current(&self) -> Option<&Issue> {
        self.issues.first()
    }

    pub fn remaining(&self) -> usize {
        self.issues.len()
    }

    pub fn ignored_any(&self) -> bool {
        !self.skipped.is_empty() || !self.ignored.is_empty()
    }

    /// The last failure, in English: look it up through [`crate::Text::ui`].
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The host's document isn't the one this pass checked.
    pub fn other_document(&self, host: &dyn Host) -> bool {
        self.started && host.document() != self.document
    }

    /// The current issue's text has changed since it was checked.
    pub fn stale(&self, host: &dyn Host) -> bool {
        self.current().is_some_and(|issue| host.version(issue.story).is_none_or(|v| self.versions.get(&issue.story) != Some(&v)))
    }

    /// Carry out `action` on the current issue. A failure is also kept as [`Review::message`].
    pub fn act(&mut self, host: &mut dyn Host, action: Action) -> Result<(), String> {
        self.report(|review| review.apply(host, action))
    }

    fn report(&mut self, run: impl FnOnce(&mut Self) -> Result<(), String>) -> Result<(), String> {
        self.message.clear();
        let result = run(self);
        if let Err(e) = &result {
            self.message.clone_from(e);
        }
        result
    }

    fn apply(&mut self, host: &mut dyn Host, action: Action) -> Result<(), String> {
        if self.other_document(host) {
            return Err(OTHER_DOCUMENT.into());
        }
        if action == Action::Check {
            self.skipped.clear();
            self.ignored.clear();
            return self.refresh(host);
        }
        let issue = self.current().cloned().ok_or(NO_ISSUE)?;
        self.validate(host, &issue)?;
        match action {
            Action::Change(to) => {
                let to = to.unwrap_or_else(|| self.replacement.clone());
                if to.is_empty() || to == issue.word {
                    return Err(SAME_REPLACEMENT.into());
                }
                host.change(&issue, &to)?;
                self.shift_skips(&issue, to.len());
            }
            Action::IgnoreOnce => {
                self.skipped.push(issue);
                self.issues.remove(0);
                return self.load_current(host);
            }
            Action::IgnoreAll => {
                self.issues.retain(|i| i.word != issue.word);
                self.ignored.insert(issue.word);
                return self.load_current(host);
            }
            Action::AddToDictionary => {
                if issue.kind != IssueKind::Spelling {
                    return Err(NOT_SPELLING.into());
                }
                host.add_word(&issue.word)?;
            }
            Action::Check => {}
        }
        self.refresh(host)
    }

    /// The text must not have changed, and a fresh check must still find the issue: language,
    /// style and dictionary edits leave the text alone.
    fn validate(&self, host: &mut dyn Host, issue: &Issue) -> Result<(), String> {
        if self.stale(host) || !host.check()?.iter().any(|found| found.same_finding(issue)) {
            return Err(CHANGED.into());
        }
        Ok(())
    }

    /// A correction moves later offsets in its story; ignored occurrences follow them.
    fn shift_skips(&mut self, change: &Issue, new_len: usize) {
        let removed = change.end.saturating_sub(change.start);
        self.skipped.retain_mut(|skip| {
            if skip.story != change.story || skip.end <= change.start {
                return true;
            }
            if skip.start < change.end {
                return false;
            }
            skip.start = skip.start.saturating_sub(removed).saturating_add(new_len);
            skip.end = skip.end.saturating_sub(removed).saturating_add(new_len);
            true
        });
        // This pass made the change, so `refresh` must not take it for someone else's edit.
        self.versions.remove(&change.story);
    }

    /// Check again, keeping this pass's ignored occurrences while their text is unchanged.
    fn refresh(&mut self, host: &mut dyn Host) -> Result<(), String> {
        let found = host.check()?;
        let versions = &self.versions;
        self.skipped.retain(|skip| versions.get(&skip.story).is_none_or(|v| host.version(skip.story) == Some(*v)));
        let (skipped, ignored) = (&self.skipped, &self.ignored);
        self.issues = found.into_iter().filter(|i| !skipped.iter().any(|skip| skip.same_finding(i)) && !ignored.contains(&i.word)).collect();
        let stories: HashSet<u64> = self.issues.iter().chain(&self.skipped).map(|i| i.story).collect();
        self.versions = stories.into_iter().filter_map(|story| Some((story, host.version(story)?))).collect();
        self.load_current(host)
    }

    /// Suggestions for the current issue, its surroundings, and showing it in the document.
    fn load_current(&mut self, host: &mut dyn Host) -> Result<(), String> {
        self.manual = false;
        self.replacement.clear();
        self.before.clear();
        self.after.clear();
        let Some(issue) = self.issues.first().cloned() else { return Ok(()) };
        let suggestions = host.suggestions(&issue).map_err(|_| CHANGED.to_string())?;
        self.replacement = suggestions.first().cloned().unwrap_or_default();
        if let Some(text) = host.text(issue.story) {
            (self.before, self.after) = surroundings(text, issue.start, issue.end);
        }
        host.reveal(&issue);
        if let Some(current) = self.issues.first_mut() {
            current.suggestions = suggestions;
        }
        Ok(())
    }
}

/// What reads around a word in a narrow panel: its paragraph, up to 60 characters each side.
fn surroundings(text: &str, start: usize, end: usize) -> (String, String) {
    const CHARS: usize = 60;
    let before = text.get(..start).unwrap_or("").rsplit('\n').next().unwrap_or("");
    let after = text.get(end..).unwrap_or("").split('\n').next().unwrap_or("");
    let left = before.char_indices().rev().nth(CHARS - 1).map_or(0, |(i, _)| i);
    let right = after.char_indices().nth(CHARS).map_or(after.len(), |(i, _)| i);
    (
        format!("{}{}", if left > 0 { "…" } else { "" }, before.get(left..).unwrap_or("")),
        format!("{}{}", after.get(..right).unwrap_or(""), if right < after.len() { "…" } else { "" }),
    )
}
