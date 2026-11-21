//! What the centre tabs keep and derive: each shown story's state, and the components of the catalog by level.

use super::catalog::{Catalog, ComponentInfo, fold_filter};
use craft_ui::contract::{Catalog as Levels, CatalogEntry, Level};
use craft_ui::testing::{StoryData, StorySnapshot};
use std::collections::BTreeMap;

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

/// A card's view of a story's state, by what the card shows from it: the values by their revision (whether they differ
/// from the start follows from it and the start), whether the start is taken, the dialog's flags and the reset count. A
/// write the card's own run makes moves the key, so the card runs once more and then replays.
impl craft_ui::contract::MemoProps for &mut CardState {
    fn key(&self, key: &mut craft_ui::contract::MemoKey) {
        self.feed(key);
    }
}

/// What each shown story keeps between frames, by story index, in index order.
#[derive(Default)]
pub struct Cells(BTreeMap<usize, CardState>);

impl Cells {
    /// The state of the story at catalog index `index`, made fresh the first time it is asked for.
    pub fn state_of(&mut self, index: usize) -> &mut CardState {
        self.0.entry(index).or_default()
    }

    /// The states of the stories at `indices`, in index order, each made fresh the first time it is asked for.
    pub fn states_of(&mut self, indices: &[usize]) -> Vec<(usize, &mut CardState)> {
        for &index in indices {
            self.0.entry(index).or_default();
        }
        self.0.iter_mut().filter(|(index, _)| indices.contains(index)).map(|(index, state)| (*index, state)).collect()
    }

    /// Every kept story's state, in index order.
    pub fn states_mut(&mut self) -> impl Iterator<Item = &mut CardState> {
        self.0.values_mut()
    }

    /// Forgets every story's state.
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// A view's view of every shown story's state, as one digest of each state's key (`&mut CardState`) in index order: the
/// key stays a fixed size at any number of cells. Computing it walks the cells once per pass of the view that holds it.
impl craft_ui::contract::MemoProps for &mut Cells {
    fn key(&self, key: &mut craft_ui::contract::MemoKey) {
        let mut cells = craft_ui::contract::MemoKey::new();
        for (index, state) in &self.0 {
            index.key(&mut cells);
            state.feed(&mut cells);
        }
        cells.finish().key(key);
    }
}

impl CardState {
    /// Feeds what a view shows from the state into `key`: the `&mut CardState` key.
    fn feed(&self, key: &mut craft_ui::contract::MemoKey) {
        use craft_ui::contract::MemoProps as _;
        let DialogState { known, hidden, shown, painted } = self.dialog;
        (self.data.revision(), self.start.is_some(), (known, hidden, shown, painted), self.resets).key(key);
    }

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
        self.data.settle();
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
