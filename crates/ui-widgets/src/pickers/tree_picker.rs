//! `TreePicker`: a field that shows the picked item's path and opens a tree of folders and items.
//!
//! The closed field is a `SelectFace`. The popup is the dropdown preset holding one tree option list: folders open and
//! close with Left, Right, Enter and a click, an item is picked with Enter or a click, and type-ahead jumps to a row
//! by its name.

#[cfg(feature = "testing")]
mod stories;

use super::SelectFace;
use super::face::toggle_on_key;
use super::words::PickerWords;
use crate::contract::overlay::{Dismiss, Popup};
use crate::contract::{
    Access, Bind, BindMut, CollectionEvent, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, Overlay, OverlayUi, Overlays as _, PaintUi,
    PartsCx, Props as _, Remember as _, Role, ShowCollection as _, WordSetCx as _,
};
use crate::list::{List, ListRow, OpenSet, RowInk};
use crate::primitives::IconSpec;
use crate::runtime::env::{EnvView, Label};
use crate::runtime::words::CommonWords;
use crate::style::{ComponentStyle, FocusLook, StyleCx, SurfaceLook};
use craft_behavior::{DisplayOrder, HitLevel, HitPriority, ItemSource, Metrics, PartKey, Selection, Sense, Value};
use egui::{Rect, Vec2};
use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::Hash;

/// The face under the whole field.
const FACE: PartKey = PartKey::new("face");

/// One item of the tree, in tree order: each folder is followed by what it holds. The entry carries the key, the name,
/// the icon and whether it can be picked.
#[derive(Clone, Debug)]
pub struct TreeEntry<K> {
    key: K,
    label: Label,
    icon: Option<IconSpec<'static>>,
    enabled: bool,
    parent: Option<K>,
    folder: bool,
    blocked: Option<Label>,
}

impl<K> TreeEntry<K> {
    /// An item at the top, named `name`, that can be picked.
    pub fn new(key: K, name: impl Into<Label>) -> Self {
        TreeEntry { key, label: name.into(), icon: None, enabled: true, parent: None, folder: false, blocked: None }
    }

    /// A folder at the top, named `name`: it is for browsing, not picking.
    pub fn folder(key: K, name: impl Into<Label>) -> Self {
        TreeEntry { folder: true, ..TreeEntry::new(key, name) }
    }

    /// The entry sits in the folder `parent`.
    #[must_use]
    pub fn within(mut self, parent: K) -> Self {
        self.parent = Some(parent);
        self
    }

    /// The entry cannot be picked: it shows dimmed with `reason` as its tooltip.
    #[must_use]
    pub fn blocked_by(mut self, reason: impl Into<Label>) -> Self {
        self.enabled = false;
        self.blocked = Some(reason.into());
        self
    }

    /// An icon before the name.
    #[must_use]
    pub fn with_icon(mut self, icon: impl Into<IconSpec<'static>>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// The key a pick reports.
    fn key(&self) -> &K {
        &self.key
    }

    fn name(&self) -> &Label {
        &self.label
    }
}

/// The style of a tree picker: the control metrics and the focus ring; the closed field is its `SelectFace`.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TreePickerStyle {
    /// The field well; the face carries the padding.
    #[style(surface)]
    surface: SurfaceLook,
    /// No padding of its own: the face carries the field's.
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
}

/// What the picker keeps while its popup is open: the folders shown open.
#[derive(Clone)]
struct Browsing<K: Hash + Eq> {
    open: OpenSet<K>,
}

impl<K: Hash + Eq> Default for Browsing<K> {
    fn default() -> Self {
        Browsing { open: OpenSet::default() }
    }
}

/// The parent of each entry as a position, and where each key is.
struct Tree<'a, K> {
    entries: &'a [TreeEntry<K>],
    positions: HashMap<&'a K, usize>,
    parents: Vec<Option<usize>>,
}

impl<'a, K: Hash + Eq + Clone> Tree<'a, K> {
    fn of(entries: &'a [TreeEntry<K>]) -> Self {
        let positions: HashMap<&K, usize> = entries.iter().enumerate().map(|(at, entry)| (entry.key(), at)).collect();
        let parents = entries.iter().map(|entry| entry.parent.as_ref().and_then(|parent| positions.get(parent).copied())).collect();
        Tree { entries, positions, parents }
    }

    fn position(&self, key: &K) -> Option<usize> {
        self.positions.get(key).copied()
    }

    /// The folders holding entry `at`, nearest first.
    fn ancestors(&self, at: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(self.parents[at], |&parent| self.parents[parent]).take(self.entries.len())
    }

    /// The folders holding the entry with `key`, open.
    fn opened_for(&self, key: Option<&K>) -> OpenSet<K> {
        let mut open = OpenSet::new();
        for folder in key.and_then(|key| self.position(key)).into_iter().flat_map(|at| self.ancestors(at)) {
            open.set(self.entries[folder].key(), true);
        }
        open
    }

    /// The positions of the entries shown with the folders in `open` open, and the depth of every entry.
    fn visible(&self, open: &OpenSet<K>) -> (Vec<usize>, Vec<usize>) {
        let mut depth = vec![0usize; self.entries.len()];
        let mut shown = vec![true; self.entries.len()];
        for at in 0..self.entries.len() {
            if let Some(parent) = self.parents[at].filter(|&parent| parent < at) {
                depth[at] = depth[parent] + 1;
                shown[at] = shown[parent] && open.is_open(self.entries[parent].key());
            }
        }
        ((0..self.entries.len()).filter(|&at| shown[at]).collect(), depth)
    }
}

/// The visible entries as a collection source, keyed by the entry's key.
struct Rows<'a, 'k, K: Hash + Eq> {
    tree: &'k Tree<'a, K>,
    labels: &'k [Label],
    shown: &'k [usize],
    depth: &'k [usize],
    open: &'k OpenSet<K>,
}

impl<K: Clone + Hash + Eq> DisplayOrder<K> for Rows<'_, '_, K> {
    fn len(&self) -> usize {
        self.shown.len()
    }

    fn key(&self, at: usize) -> K {
        self.tree.entries[self.shown[at]].key().clone()
    }

    fn depth(&self, at: usize) -> usize {
        self.depth[self.shown[at]]
    }

    fn has_children(&self, at: usize) -> bool {
        self.tree.entries[self.shown[at]].folder
    }

    fn open(&self, at: usize) -> bool {
        let entry = &self.tree.entries[self.shown[at]];
        entry.folder && self.open.is_open(entry.key())
    }

    fn enabled(&self, at: usize) -> bool {
        self.tree.entries[self.shown[at]].enabled
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.labels[self.shown[at]].as_str()))
    }
}

impl<K: Clone + Hash + Eq> ItemSource<K> for Rows<'_, '_, K> {}

/// A field that opens a tree of items and folders and sets the binding to the picked item's key. The field shows the
/// item's path, folder names joined by the path separator, the placeholder while nothing is picked, or the mixed word.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(overlay, probe(TreePicker<'static, u32>))]
pub struct TreePicker<'a, K: Value + Eq + Hash + 'static> {
    #[common]
    common: Common,
    value: Bind<'a, Option<K>>,
    entries: &'a [TreeEntry<K>],
    /// Shown while nothing is picked.
    #[prop(into)]
    placeholder: Option<Label>,
}

impl<'a, K: Value + Eq + Hash + 'static> TreePicker<'a, K> {
    /// A picker named `label` over `value`, choosing among `entries` in tree order.
    pub fn new(label: impl Into<Label>, value: Bind<'a, Option<K>>, entries: &'a [TreeEntry<K>]) -> Self {
        TreePicker { common: Common::new(), value, entries, placeholder: None }.label(label)
    }

    /// The names from the top folder down to the entry with `key`, joined with `separator`.
    fn path(&self, key: &K, separator: &str, env: &EnvView<'_>) -> Option<String> {
        let tree = Tree::of(self.entries);
        let at = tree.position(key)?;
        let mut names: Vec<Label> = std::iter::once(at).chain(tree.ancestors(at)).map(|at| env.resolve(self.entries[at].name())).collect();
        names.reverse();
        Some(names.iter().map(Label::as_str).collect::<Vec<_>>().join(separator))
    }

    /// What the field shows: the picked item's path; else, in the muted look, the mixed word for a mixed value or the
    /// placeholder.
    fn face(&self, value: Option<&Option<K>>, words: &PickerWords, separator: &str, env: &EnvView<'_>) -> SelectFace<'_> {
        let Some(value) = value else {
            return SelectFace::new(env.resolve(&words.mixed)).placeholder();
        };
        match value.as_ref().and_then(|key| self.path(key, separator, env)) {
            Some(path) => SelectFace::new(path),
            None => SelectFace::new(env.resolve(&self.placeholder.clone().unwrap_or_else(|| words.tree_placeholder.clone()))).placeholder(),
        }
    }
}

impl<K: Value + Eq + Hash + 'static> Component for TreePicker<'_, K> {
    const ROLE: Role = Role::COMBO_BOX;
    type Style = TreePickerStyle;
    type Output = EditResponse<Option<K>>;

    fn style(cx: &StyleCx<'_>, _: TreePickerStyleKey<K>) -> TreePickerStyle {
        TreePickerStyle { surface: cx.field(), metrics: Metrics::new(), focus: cx.focus_look() }
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let separator = cx.words::<CommonWords>().path_separator.clone();
        cx.face(self.face(cx.value(), &cx.words::<PickerWords>(), separator.as_str(), &cx.env))
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.part(FACE, rect)
            .sense(Sense::CLICK | Sense::HOVER | Sense::TAKE_FOCUS)
            .priority(HitPriority::new(HitLevel::Body, 0))
            .pointer_only("Enter, Space and Alt+Down open the tree")
            .toggles("tree")
            .on_press();
    }

    fn bind(&mut self) -> Option<BindMut<'_, Option<K>>> {
        Some(self.value.reborrow())
    }

    /// A press on the face and Alt+Down open the popup through the face's `toggles`; Enter and Space toggle it here.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("tree");
        toggle_on_key(cx, popup);
    }

    /// The popup: the tree with the folders closed except those holding the picked item, which is selected and in
    /// view. A click or Enter on a folder opens or closes it; on an item it sets the binding and closes the popup. A
    /// blocked item takes neither.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("tree");
        let entries = self.entries;
        let tree = Tree::of(entries);
        let chosen = cx.value().and_then(|value| value.clone());
        let labels: Vec<Label> = entries.iter().map(|entry| cx.env.resolve(entry.name())).collect();
        let reasons: Vec<Option<Label>> = entries.iter().map(|entry| entry.blocked.as_ref().map(|reason| cx.env.resolve(reason))).collect();
        let name = cx.label();
        // Folders open and close inside the popup, so activating one must not close it: a pick closes it below.
        let overlay = Overlay::dropdown().dismiss(Dismiss::new());
        let events = ui.show(popup, overlay, |ui, _| {
            let fresh = Popup::opening(popup, cx);
            if fresh {
                cx.memory::<Browsing<K>>().open = tree.opened_for(chosen.as_ref());
            }
            let open = cx.memory::<Browsing<K>>().open.clone();
            let (shown, depth) = tree.visible(&open);
            let rows = Rows { tree: &tree, labels: &labels, shown: &shown, depth: &depth, open: &open };
            let selection = chosen.clone().map_or_else(Selection::default, Selection::single);
            let mut list = List::<K>::options(name.clone()).allow_empty(false).tree(true).selection(Bind::given(selection));
            if let Some(key) = chosen.clone().filter(|_| fresh) {
                list = list.reveal(key);
            }
            list.show_items(ui, &rows, &mut |key, _| {
                let Some(at) = tree.position(key) else { return ListRow::spacer() };
                let entry = &entries[at];
                let mut row = ListRow::new(labels[at].clone()).option().depth(depth[at]);
                if entry.folder {
                    row = row.group(open.is_open(key));
                }
                if let Some(icon) = entry.icon.as_ref() {
                    row = row.icon(icon.clone());
                }
                match &reasons[at] {
                    Some(reason) => row.enabled(false).ink(RowInk::Dimmed).tooltip(reason.clone()),
                    None => row,
                }
            })
            .events
        });
        for event in events.into_iter().flatten() {
            match event {
                CollectionEvent::Activate(key) => {
                    let Some(entry) = tree.position(&key).map(|at| &entries[at]) else { continue };
                    if entry.folder {
                        let browsing = cx.memory::<Browsing<K>>();
                        let now = browsing.open.is_open(&key);
                        browsing.open.set(&key, !now);
                    } else if entry.enabled {
                        cx.propose(Some(key));
                        cx.commit();
                        popup.close(cx);
                    }
                }
                CollectionEvent::Open(change) => {
                    let folders = entries.iter().filter(|entry| entry.folder).map(TreeEntry::key);
                    cx.memory::<Browsing<K>>().open.apply(&change.key, change.open, change.all, folders);
                }
                _ => {}
            }
        }
    }

    /// The picked item's path as the field's value.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let separator = cx.words::<CommonWords>().path_separator.clone();
        if let Some(path) = cx.value().and_then(Option::as_ref).and_then(|key| self.path(key, separator.as_str(), &cx.env)) {
            access.value(path);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let separator = cx.words::<CommonWords>().path_separator.clone();
        ui.face(FACE, self.face(cx.value(), &cx.words::<PickerWords>(), separator.as_str(), &cx.env));
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<Option<K>> {
        cx.edit_response()
    }
}
