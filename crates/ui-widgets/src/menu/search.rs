//! The command search of the Help menu: [`HelpEntry`], a command found by its label, its English label, its menu
//! path or its command id, and [`search_order`], which ranks the entries for a query.

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
/// its `[label, english label, menu path, command id]` texts, matched by `folder` (case and accents folded). Every
/// word of the query must match one of an entry's texts. A word found in the label or English label ranks before one
/// found only in the path or command id; an entry ranks by its worst tier, then by the worst matcher rank among its
/// words, then by menu order.
pub fn search_order(folder: &Folder, query: &str, texts: &[[&str; 4]]) -> Vec<usize> {
    let matchers: Vec<Matcher> = query.split_whitespace().map(|word| Matcher::new(folder, word)).collect();
    if matchers.is_empty() {
        return Vec::new();
    }
    let mut ranked: Vec<_> = texts
        .iter()
        .enumerate()
        .filter_map(|(at, [label, english, path, id])| {
            let mut worst = (0_u8, None);
            for matcher in &matchers {
                let named = [label, english].into_iter().filter_map(|text| matcher.score(text)).map(|found| found.rank).min();
                let (tier, rank) = match named {
                    Some(rank) => (0_u8, rank),
                    None => (1, [path, id].into_iter().filter_map(|text| matcher.score(text)).map(|found| found.rank).min()?),
                };
                worst.0 = worst.0.max(tier);
                worst.1 = worst.1.max(Some(rank));
            }
            Some((worst.0, worst.1, at))
        })
        .collect();
    ranked.sort();
    ranked.into_iter().take(MAX_RESULTS).map(|(_, _, at)| at).collect()
}
