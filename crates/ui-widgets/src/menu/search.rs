//! The command search of the Help menu and the palette: [`HelpEntry`], a command found by its label, its English label,
//! its menu path, its ids or its description words, and [`SearchIndex`] with [`rank`], which index the entries and
//! order them for a query.

use crate::primitives::IconSpec;
use crate::runtime::commands::CommandRef;
use crate::runtime::env::Label;

mod index;
mod synonyms;

pub use index::{SearchIndex, rank};
pub use synonyms::{SearchWords, Synonyms};

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
    home: Option<Label>,
    words: Option<String>,
    icon: Option<IconSpec<'static>>,
    ids: Vec<String>,
    tie_order: u8,
}

impl HelpEntry {
    /// A command found by `label`, the label its menu row shows.
    pub fn new(command: impl Into<CommandRef>, label: impl Into<Label>) -> Self {
        HelpEntry {
            command: command.into(),
            label: label.into(),
            english: None,
            path: Vec::new(),
            home: None,
            words: None,
            icon: None,
            ids: Vec::new(),
            tie_order: 0,
        }
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

    /// The label shown in place of the path. It is not searched.
    #[must_use]
    pub fn with_home(mut self, home: impl Into<Label>) -> Self {
        self.home = Some(home.into());
        self
    }

    /// The label shown in place of the path, if any.
    #[must_use]
    pub fn home(&self) -> Option<&Label> {
        self.home.as_ref()
    }

    /// The icon the palette row shows. It is not searched.
    #[must_use]
    pub fn with_icon(mut self, icon: IconSpec<'static>) -> Self {
        self.icon = Some(icon);
        self
    }

    /// The icon the palette row shows, if any.
    #[must_use]
    pub fn icon(&self) -> Option<&IconSpec<'static>> {
        self.icon.as_ref()
    }

    /// The description words: searched (word index, description tier), never shown.
    #[must_use]
    pub fn with_words(mut self, words: impl Into<String>) -> Self {
        self.words = Some(words.into());
        self
    }

    /// The ids searched in the ID and exact-id tiers; with none given, the command's own id.
    #[must_use]
    pub fn with_ids(mut self, ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.ids = ids.into_iter().map(Into::into).collect();
        self
    }

    /// The entry's place among entries that match a query as well and were picked as often and as recently, lower
    /// first, before the label's length decides (an app's tools before its menu commands, say). 0 when not given.
    #[must_use]
    pub fn with_tie_order(mut self, order: u8) -> Self {
        self.tie_order = order;
        self
    }

    /// The ids searched in the ID and exact-id tiers.
    #[must_use]
    pub fn ids(&self) -> &[String] {
        &self.ids
    }

    /// The searched words, if any.
    #[must_use]
    pub fn words(&self) -> Option<&str> {
        self.words.as_deref()
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
