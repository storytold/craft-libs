//! `IconPicker`: a search field over a grid of icon tiles in named groups.
//!
//! The pattern puts a `TextField::search` over a `TileGrid`. The field drives the grid: Up and Down move into the
//! tiles, Enter picks the active tile, and focus stays in the field. A click on a tile picks it.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::words::PickerWords;
use crate::contract::{
    ActivateOn, Bind, BodyCx, CollectionEvent, NoValue, Outcome, Pattern, Props, Remember as _, SelectMode, Show as _, ShowCollection as _,
    ShowContainer as _,
};
use crate::layout_components::Stack;
use crate::primitives::IconSpec;
use crate::runtime::env::{Env, Label};
use crate::tabs::{Tab, TabKind, TabStrip};
use crate::text::TextField;
use crate::tile_grid::{CaptionPlace, GridEvent, Tile, TileGrid, TileGroup};
use craft_behavior::{DisplayOrder, EditEvent, ItemSource, Mixed, Selection, SelectionChange, keyed};
use craft_i18n::matcher::Matcher;
use egui::{Response, Ui};
use std::borrow::Cow;
use std::collections::HashSet;
use std::hash::Hash;
use std::sync::Arc;

/// One cell: what it picks, its name (tooltip, accessible name, search) and more words it is found by.
#[derive(Clone, Debug, PartialEq, Props)]
#[non_exhaustive]
pub struct IconPickerItem<K> {
    /// What the cell picks.
    pub key: K,
    /// The cell's name.
    pub label: String,
    /// Words the search also matches ("arrow", "direction").
    #[prop]
    pub keywords: Vec<String>,
    /// The cell's icon; a cell without one shows the placeholder.
    #[prop(into)]
    pub icon: Option<IconSpec<'static>>,
}

impl<K> IconPickerItem<K> {
    /// A cell that picks `key`, named `label`, found by no other words.
    pub fn new(key: K, label: impl Into<String>) -> Self {
        Self { key, label: label.into(), keywords: Vec::new(), icon: None }
    }

    /// Whether the name or a keyword matches `matcher`.
    fn matches(&self, matcher: &Matcher) -> bool {
        matcher.score(&self.label).is_some() || self.keywords.iter().any(|word| matcher.score(word).is_some())
    }
}

/// A titled group of cells; with a search typed the groups with no match are hidden.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct IconPickerGroup<K> {
    /// The group's title.
    pub name: String,
    /// The group's cells, in display order.
    pub items: Vec<IconPickerItem<K>>,
}

impl<K> IconPickerGroup<K> {
    /// A group titled `name` holding `items`.
    pub fn new(name: impl Into<String>, items: Vec<IconPickerItem<K>>) -> Self {
        Self { name: name.into(), items }
    }
}

/// What the picker reported this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct IconPickerOutput<K> {
    /// The grid's response.
    pub response: Response,
    /// The cell clicked or activated with Enter this pass. A host showing the picker in a popover closes it.
    pub picked: Option<K>,
    /// The search text changed.
    pub search_changed: bool,
}

impl<K> Outcome for IconPickerOutput<K> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        Outcome::story_facts(&self.response)
    }
}

/// A searchable grid of icons in named groups. The cell of the bound key is selected; a pick writes its key to the
/// binding and is reported in `picked`. Each cell shows the icon of its item; the cell's name is its tooltip and
/// accessible name, and the search matches it and its keywords.
///
/// ```ignore
/// let groups = [IconPickerGroup::new("Drawing", vec![IconPickerItem::new(Tool::Brush, "Brush").icon(IconId::new("brush"))])];
/// let out = IconPicker::new("Tool", Bind::bound(&mut tool), &groups).show(ui);
/// ```
#[derive(Pattern, Props)]
#[pattern(probe(IconPicker<'static, u8>))]
pub struct IconPicker<'a, K: Clone + Eq + Hash + Send + Sync + 'static> {
    label: Label,
    value: Bind<'a, Option<K>>,
    groups: &'a [IconPickerGroup<K>],
    /// How the groups show: as headings over their cells, or as a filter row.
    #[prop]
    group_display: IconPickerGroups,
    /// Shown when there are no cells.
    #[prop(into)]
    empty: Option<Label>,
}

/// How an [`IconPicker`] shows its groups.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum IconPickerGroups {
    /// Each group's name is a heading over its cells.
    #[default]
    Headings,
    /// A filter row: All first, then each group.
    Filter,
}

/// The visible cells as the grid's source, keyed by position.
struct Cells<'a, K> {
    items: Vec<&'a IconPickerItem<K>>,
}

impl<K> DisplayOrder<usize> for Cells<'_, K> {
    fn len(&self) -> usize {
        self.items.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.items.len()).then_some(*key)
    }

    fn is_section(&self, _: usize) -> bool {
        false
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.items[at].label.as_str()))
    }
}

impl<K> ItemSource<usize> for Cells<'_, K> {}

impl<'a, K: Clone + Eq + Hash + Send + Sync + 'static> IconPicker<'a, K> {
    /// A picker named `label` over `value`, choosing among the cells of `groups`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, Option<K>>, groups: &'a [IconPickerGroup<K>]) -> Self {
        IconPicker { label: label.into(), value, groups, group_display: IconPickerGroups::default(), empty: None }
    }

    /// Shows the picker and returns what the user did this pass.
    pub fn show(self, ui: &mut Ui) -> IconPickerOutput<K> {
        let IconPicker { label, mut value, groups, group_display, empty } = self;
        let shown = Stack::new().show(ui, |ui, cx| Self::body(ui, cx, &label, &mut value, groups, (group_display, empty.as_ref())));
        shown.inner.unwrap_or_else(|| IconPickerOutput { response: shown.output.clone(), picked: None, search_changed: false })
    }

    /// The search field over the grid, inside the picker's root container. The names of the closed groups are kept in
    /// the container's memory.
    fn body(
        ui: &mut Ui,
        cx: &mut BodyCx<'_>,
        label: &Label,
        value: &mut Bind<'a, Option<K>>,
        groups: &[IconPickerGroup<K>],
        (group_display, empty): (IconPickerGroups, Option<&Label>),
    ) -> IconPickerOutput<K> {
        let env = Env::of(ui);
        let words = env.words::<PickerWords>();
        let grid_id = ui.scope_id().with("icons");
        let search = TextField::search(words.icon_search.clone(), Bind::kept()).drives(grid_id).show(ui);
        let search_changed = !search.events.is_empty();
        let query = match &search.shown {
            Some(Mixed::Same(shown)) => shown.trim().to_owned(),
            _ => String::new(),
        };
        let matcher = (!query.is_empty()).then(|| Matcher::new(&env.folder(), &query));

        // The chosen category under `Filter`: 0 is All, `n` the group at `n - 1`. The strip's change applies before the
        // cells are built, so the grid shows the new category in the same pass.
        let mut category = 0;
        if group_display == IconPickerGroups::Filter {
            category = cx.memory::<Category>().0.min(groups.len());
            let tabs: Vec<(usize, String)> = std::iter::once((0, words.all.as_str().to_owned()))
                .chain(groups.iter().enumerate().map(|(at, group)| (at + 1, group.name.clone())))
                .collect();
            let source = keyed(&tabs, |tab: &(usize, String)| tab.0).with_text(|tab| Cow::Borrowed(tab.1.as_str()));
            let strip = TabStrip::new(label.clone())
                .kind(TabKind::Text)
                .shown(category)
                .show_items(ui, &source, &mut |key, _| Tab::new(tabs[*key].1.clone()));
            for event in &strip.response.events {
                if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
                    && let Some(key) = keys.last()
                {
                    category = (*key).min(groups.len());
                }
            }
            // A tab the strip moved to its overflow menu is picked there, and reported as `picked`.
            if let Some(key) = strip.picked {
                category = key.min(groups.len());
            }
            cx.memory::<Category>().0 = category;
        }

        let mut items = Vec::new();
        let mut named = Vec::new();
        for (_, group) in groups.iter().enumerate().filter(|(at, _)| category == 0 || at + 1 == category) {
            let kept: Vec<&IconPickerItem<K>> =
                group.items.iter().filter(|item| matcher.as_ref().is_none_or(|matcher| item.matches(matcher))).collect();
            if kept.is_empty() {
                continue;
            }
            if group_display == IconPickerGroups::Headings && !group.name.is_empty() {
                named.push((items.len(), kept.len(), group.name.clone()));
            }
            items.extend(kept);
        }
        let closed = cx.memory::<ClosedGroups>().clone();
        // The header rows are keyed after the cells, whose keys are their positions.
        let shown_groups: Vec<TileGroup<usize>> = named
            .iter()
            .enumerate()
            .map(|(at, (start, count, name))| TileGroup::new(items.len() + at, *start, *count, name.clone()).open(!closed.0.contains(name)))
            .collect();
        let held: Option<K> = match value.reborrow().get() {
            Some(Mixed::Same(Some(key))) => Some(key),
            _ => None,
        };
        let selection =
            held.as_ref().and_then(|held| items.iter().position(|item| item.key == *held)).map_or_else(Selection::default, Selection::single);
        let source = Cells { items };
        let grid = TileGrid::<usize>::new(label.clone())
            .id(grid_id)
            .select(SelectMode::Single)
            .activate(ActivateOn::Click)
            .caption(CaptionPlace::None)
            .content_cells()
            .groups(&shown_groups)
            .collapsible()
            .selection(Bind::given(selection))
            .rows_stamp(query_stamp(&query, category))
            .empty(empty.cloned().unwrap_or_else(|| words.icon_no_match.clone()))
            .show_items(ui, &source, &mut |at, _| {
                let item = source.items[*at];
                let tile = Tile::new(item.label.clone());
                match &item.icon {
                    Some(icon) => tile.icon(icon.clone()),
                    None => tile,
                }
            });
        for event in &grid.events {
            let (group, open) = match event {
                GridEvent::GroupOpen { group, open } => (*group, Some(*open)),
                GridEvent::GroupPicked { group, .. } => (*group, None),
                _ => continue,
            };
            if let Some((_, _, name)) = named.get(group) {
                let closed = &mut cx.memory::<ClosedGroups>().0;
                if open.unwrap_or_else(|| closed.contains(name)) {
                    closed.remove(name);
                } else {
                    closed.insert(name.clone());
                }
            }
        }
        let picked = grid.response.events.iter().find_map(|event| match event {
            CollectionEvent::Activate(at) => source.items.get(*at).map(|item| item.key.clone()),
            _ => None,
        });
        if let Some(key) = &picked
            && held.as_ref() != Some(key)
        {
            let name: Arc<str> = Arc::from(label.as_str());
            value.reborrow().apply(&EditEvent::set(Mixed::Same(held), Some(key.clone()), name, None));
        }
        IconPickerOutput { response: grid.response.response, picked, search_changed }
    }
}

/// The names of the groups the user closed.
#[derive(Clone, Default)]
struct ClosedGroups(HashSet<String>);

/// The category the filter row shows: 0 is All, `n` the group at `n - 1`.
#[derive(Clone, Default)]
struct Category(usize);

/// The rows stamp of a search text inside a category: the same pair keeps the layout, another one rebuilds it.
fn query_stamp(query: &str, category: usize) -> u64 {
    use std::hash::Hasher as _;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (query, category).hash(&mut hasher);
    hasher.finish()
}
