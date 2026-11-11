//! The command search of the Help menu: [`HelpEntry`], a command found by its label, its English label or its menu
//! path, and [`search_order`], which ranks the entries for a query.

use crate::runtime::commands::CommandRef;
use crate::runtime::env::Label;
use craft_i18n::Folder;
use craft_i18n::matcher::Matcher;

/// The most matches the help search lists.
pub const MAX_RESULTS: usize = 15;

/// A command the help search can find.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct HelpEntry {
    command: CommandRef,
    label: Label,
    english: Option<Label>,
    path: Vec<Label>,
}

impl HelpEntry {
    /// A command found by `label`, the label its menu row shows.
    pub fn new(command: impl Into<CommandRef>, label: impl Into<Label>) -> Self {
        HelpEntry { command: command.into(), label: label.into(), english: None, path: Vec::new() }
    }

    /// The English label, which matches too.
    #[must_use]
    pub fn with_english(mut self, english: impl Into<Label>) -> Self {
        self.english = Some(english.into());
        self
    }

    /// The menu names above the command, outermost first ("Image", "Adjustments").
    #[must_use]
    pub fn with_path(mut self, path: impl IntoIterator<Item = impl Into<Label>>) -> Self {
        self.path = path.into_iter().map(Into::into).collect();
        self
    }

    /// The command the entry runs.
    #[must_use]
    pub fn command(&self) -> &CommandRef {
        &self.command
    }

    /// The label its menu row shows.
    #[must_use]
    pub fn label(&self) -> &Label {
        &self.label
    }

    /// The English label, if any.
    #[must_use]
    pub fn english(&self) -> Option<&Label> {
        self.english.as_ref()
    }

    /// The menu names above the command, outermost first.
    #[must_use]
    pub fn path(&self) -> &[Label] {
        &self.path
    }
}

/// The entries matching `query`, best first and in menu order within a rank, at most `MAX_RESULTS`. Each entry is
/// its `[label, english label, menu path]` texts, matched by `folder` (case and accents folded). A label or English
/// label that matches ranks before an entry found only through its path; within each, the matcher's rank decides.
pub fn search_order(folder: &Folder, query: &str, texts: &[[&str; 3]]) -> Vec<usize> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let matcher = Matcher::new(folder, query);
    let mut ranked: Vec<_> = texts
        .iter()
        .enumerate()
        .filter_map(|(at, [label, english, path])| {
            let named = [label, english].into_iter().filter_map(|text| matcher.score(text)).map(|found| found.rank).min();
            match named {
                Some(found) => Some((0_u8, found, at)),
                None => matcher.score(path).map(|found| (1, found.rank, at)),
            }
        })
        .collect();
    ranked.sort();
    ranked.into_iter().take(MAX_RESULTS).map(|(_, _, at)| at).collect()
}
