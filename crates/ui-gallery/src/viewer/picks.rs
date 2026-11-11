//! What the centre tabs keep and derive: each shown story's state, and the components of the catalog by level.

use super::catalog::{Catalog, ComponentInfo, fold_filter};
use craft_ui::contract::{Catalog as Levels, CatalogEntry, Level};
use craft_ui::testing::{StoryData, StorySnapshot};

/// Whether a story's dialog is closed behind its button.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct DialogState {
    /// The story opened a dialog.
    pub known: bool,
    /// The dialog is closed: the card shows its button and does not run the story.
    pub hidden: bool,
    /// The dialog was seen open since the button was pressed.
    pub shown: bool,
    /// The story has painted in the card, so it is past its start: a dialog the user opens with the story's own
    /// controls stays open.
    pub painted: bool,
}

/// What one shown story keeps between frames.
#[derive(Default)]
#[non_exhaustive]
pub struct CardState {
    /// The story's values.
    pub data: StoryData,
    /// The values after the first pass, which Reset restores; `None` before the story has run.
    pub start: Option<StorySnapshot>,
    /// Where the story's dialog is: a story that opens one is shown closed, behind a "Show dialog" button.
    pub dialog: DialogState,
    /// Counts the resets of this story; the card's id scope carries it, so a reset mounts the story anew.
    pub resets: u32,
}

impl CardState {
    /// Whether the story's values differ from the ones it started with.
    pub fn changed(&self) -> bool {
        self.start.as_ref().is_some_and(|start| !self.data.changed_since(start).is_empty())
    }

    /// Notes the values after the story's first pass as the ones Reset restores.
    pub fn note_start(&mut self) {
        if self.start.is_none() {
            self.start = Some(self.data.snapshot());
        }
    }

    /// Opens the dialog the card holds closed behind its button, as the button does: the story runs again with its
    /// `open` value on.
    pub fn show_dialog(&mut self) {
        self.dialog = DialogState { known: true, hidden: false, shown: false, painted: true };
        if let Some(open) = self.data.present::<bool>("open") {
            *open = true;
        }
    }

    /// Gives the story its start values again, on a fresh id scope.
    pub fn reset(&mut self) {
        *self = CardState { resets: self.resets + 1, ..CardState::default() };
    }
}

/// The level, display name and surface of every registered type linked into the binary, the library's and the
/// domain crates'.
pub fn levels() -> &'static Levels {
    super::catalog::linked_catalog()
}

/// The level facts of the component registered as `type_name`.
pub fn entry_of(type_name: &str) -> Option<&'static CatalogEntry> {
    levels().entry(type_name)
}

/// The components of `catalog` that match `filter`, by level in listing order, each level's components by display name
/// ignoring case; only levels that have components are listed.
pub fn by_level<'a>(catalog: &'a Catalog, filter: &str) -> Vec<(Level, Vec<&'a ComponentInfo>)> {
    let filter = fold_filter(filter);
    let mut out: Vec<(Level, Vec<(&str, &ComponentInfo)>)> = Level::ALL.iter().map(|level| (*level, Vec::new())).collect();
    for component in catalog.components.iter().filter(|component| component.matches(&filter)) {
        if let Some((level, name)) = component.listed()
            && let Some((_, list)) = out.iter_mut().find(|(at, _)| *at == level)
        {
            list.push((name, component));
        }
    }
    out.into_iter()
        .filter(|(_, list)| !list.is_empty())
        .map(|(level, mut list)| {
            list.sort_by_key(|(name, _)| *name);
            (level, list.into_iter().map(|(_, component)| component).collect())
        })
        .collect()
}
