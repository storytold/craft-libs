//! The preset library: groups, nested folders and presets with unique names, and the browser rows made from them (`PresetKey`, `GroupPath`, `PresetLibrary`, `PresetChange`, `PresetError`).

use serde::{Deserialize, Serialize};

use super::PresetRow;

/// A group or folder by its names from the top group down: `["Brushes", "Dry Media"]`. The empty
/// path is the library's root, which holds the ungrouped presets and the top groups. Names are
/// unique among siblings, so a path names one group.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GroupPath(Vec<String>);

impl GroupPath {
    /// The path of `names`, from the top group down.
    pub fn new<S: Into<String>>(names: impl IntoIterator<Item = S>) -> Self {
        GroupPath(names.into_iter().map(Into::into).collect())
    }

    /// The root: the ungrouped presets and the top groups.
    pub fn root() -> Self {
        GroupPath(Vec::new())
    }

    /// The names, from the top group down.
    pub fn names(&self) -> &[String] {
        &self.0
    }

    /// The group's own name; `""` for the root.
    pub fn name(&self) -> &str {
        self.0.last().map_or("", String::as_str)
    }

    /// How deep the group is: 0 for the root, 1 for a top group, 2 for a folder in it.
    pub fn depth(&self) -> usize {
        self.0.len()
    }

    /// The group that holds this one; `None` for the root.
    pub fn parent(&self) -> Option<GroupPath> {
        let (_, parent) = self.0.split_last()?;
        Some(GroupPath(parent.to_vec()))
    }

    /// The path of the child group `name` of this one.
    pub fn child(&self, name: impl Into<String>) -> GroupPath {
        let mut names = self.0.clone();
        names.push(name.into());
        GroupPath(names)
    }
}

/// The key of a browser row: a group (or folder) or a preset, so a group's key and a preset's
/// key never collide (PhotoCraft's `"group:{name}"` strings, the private key of
/// `SwatchesPanel`). `I` is the preset's id: [`PresetId`] in a [`PresetLibrary`]; PhotoCraft's
/// engine library uses the preset's name (`PresetKey<String>`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum PresetKey<I> {
    /// A group or folder.
    Group(GroupPath),
    /// A preset.
    Item(I),
}

impl<I> PresetKey<I> {
    /// The key of the group `names` (from the top group down).
    pub fn group<S: Into<String>>(names: impl IntoIterator<Item = S>) -> Self {
        PresetKey::Group(GroupPath::new(names))
    }

    /// The key of preset `id`.
    pub fn item(id: I) -> Self {
        PresetKey::Item(id)
    }
}

/// One preset of a [`PresetLibrary`], unique by id; ids are never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PresetId(u64);

impl PresetId {
    /// The id `value`.
    pub const fn new(value: u64) -> Self {
        PresetId(value)
    }

    /// The number inside.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A preset: its id, its name (unique across the library, ignoring case) and the app's value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PresetItem<T> {
    /// The library-wide id; stays the same when the preset is renamed or moved.
    pub id: PresetId,
    /// The preset's name, unique across the library ignoring case.
    pub name: String,
    /// The app's value.
    pub value: T,
}

/// A group or folder: its name (unique among its siblings, ignoring case), its presets and its
/// folders, each in display order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(bound(serialize = "T: Serialize", deserialize = "T: Deserialize<'de>"))]
#[non_exhaustive]
pub struct PresetGroup<T> {
    /// The group's name, unique among its siblings ignoring case.
    pub name: String,
    /// The presets directly in the group, in display order.
    pub items: Vec<PresetItem<T>>,
    /// The folders directly in the group, in display order.
    pub groups: Vec<PresetGroup<T>>,
}

/// What a library change did, for the app to save the library or follow its selection.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PresetChange {
    /// A preset was added.
    #[non_exhaustive]
    Added {
        /// The new preset's id.
        id: PresetId,
        /// The group it went into.
        group: GroupPath,
    },
    /// A group was added.
    GroupAdded(GroupPath),
    /// A preset was renamed.
    #[non_exhaustive]
    Renamed {
        /// The preset renamed.
        id: PresetId,
        /// Its new name.
        name: String,
    },
    /// A group was renamed; `to` is its new path (its folders' paths change with it).
    #[non_exhaustive]
    GroupRenamed {
        /// The path before the rename.
        from: GroupPath,
        /// The path after the rename.
        to: GroupPath,
    },
    /// A preset was deleted.
    Deleted(PresetId),
    /// A group was deleted with everything inside it; `items` are the presets that went with it.
    #[non_exhaustive]
    GroupDeleted {
        /// The path the group had.
        path: GroupPath,
        /// The presets deleted with it.
        items: Vec<PresetId>,
    },
    /// A preset moved to another group (Move to, or a drag into a group).
    #[non_exhaustive]
    Moved {
        /// The preset moved.
        id: PresetId,
        /// The group it moved into.
        to: GroupPath,
    },
    /// A group moved to another parent; `to` is its new path.
    #[non_exhaustive]
    GroupMoved {
        /// The path before the move.
        from: GroupPath,
        /// The path after the move.
        to: GroupPath,
    },
    /// Presets or groups changed order inside one group.
    Reordered(GroupPath),
}

/// Why a library change was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PresetError {
    /// The name is empty or all blank.
    EmptyName,
    /// Another preset (or a sibling group) has the name, ignoring case.
    NameTaken,
    /// The group path names no group.
    NoGroup,
    /// No preset has the id.
    NoPreset,
    /// The group would sit deeper than the library's nesting.
    TooDeep,
    /// A rename to the name the preset or group already has (the same letters): nothing changes,
    /// so the app saves nothing. [`PresetLibrary::apply`] returns no change for it.
    Unchanged,
}

impl std::fmt::Display for PresetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PresetError::EmptyName => "the name is empty",
            PresetError::NameTaken => "the name is taken",
            PresetError::NoGroup => "no group has that path",
            PresetError::NoPreset => "no preset has that id",
            PresetError::TooDeep => "the group would be nested too deep",
            PresetError::Unchanged => "the name is the current name",
        })
    }
}

impl std::error::Error for PresetError {}

/// A library of presets the app owns and keeps in a settings section (it is serde): ungrouped
/// presets and groups at the root, folders inside groups up to [`nesting`](Self::nesting) levels
/// deep, presets with names unique across the library ignoring case (as `Profiles` and New
/// Document). An optional helper: the browser's contract is [`PresetRow`] and [`PresetEvent`](crate::patterns::presets::PresetEvent),
/// and an app with its own engine library uses only [`PresetKey`]. `rows` makes the browser's rows and `apply`
/// carries out the browser's events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(bound(serialize = "T: Serialize", deserialize = "T: Deserialize<'de>"))]
pub struct PresetLibrary<T> {
    /// The root: its `name` is empty, its items are the ungrouped presets.
    pub(super) root: PresetGroup<T>,
    /// How deep folders go below a top group: 0 for flat groups (PhotoCraft's Gradients,
    /// Patterns, Styles, Shapes, Swatches), 32 for PhotoCraft's Brushes.
    pub(super) nesting: usize,
    /// The number of the next id; never goes down.
    pub(super) next_id: u64,
}

impl<T> Default for PresetLibrary<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> PresetLibrary<T> {
    /// An empty library with flat groups (nesting 0).
    pub fn new() -> Self {
        Self { root: PresetGroup { name: String::new(), items: Vec::new(), groups: Vec::new() }, nesting: 0, next_id: 1 }
    }

    /// Sets how deep folders may go below a top group: the number `PresetReorder::nest` takes
    /// (PhotoCraft's Brushes `MAX_FOLDER_DEPTH` 32).
    #[must_use]
    pub fn with_nesting(mut self, nesting: usize) -> Self {
        self.nesting = nesting;
        self
    }

    /// How deep folders may go below a top group.
    pub fn nesting(&self) -> usize {
        self.nesting
    }

    /// Adds a preset named `name` at the end of `group` (the root for ungrouped presets) and
    /// returns its id. The name is trimmed; a taken name is numbered "Red 2", "Red 3", …
    /// (PhotoCraft's `unique_name`, here ignoring case). An empty or blank name is
    /// [`PresetError::EmptyName`]; a path that names no group is [`PresetError::NoGroup`].
    pub fn add(&mut self, group: &GroupPath, name: &str, value: T) -> Result<PresetId, PresetError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(PresetError::EmptyName);
        }
        if self.group(group).is_none() {
            return Err(PresetError::NoGroup);
        }
        let name = self.unique_name(name);
        let id = PresetId(self.next_id);
        self.next_id += 1;
        let target = self.group_mut(group).ok_or(PresetError::NoGroup)?;
        target.items.push(PresetItem { id, name, value });
        Ok(id)
    }

    /// Adds a group under `parent` (the root for a top group) named `base`, numbered "Group 2",
    /// "Group 3", … when a sibling has the name, and returns its path. Refuses a group deeper
    /// than the nesting allows ([`PresetError::TooDeep`]: a top group is always allowed; a folder
    /// at depth `d` below its top group needs `d <= nesting`).
    pub fn new_group(&mut self, parent: &GroupPath, base: &str) -> Result<GroupPath, PresetError> {
        let base = base.trim();
        if base.is_empty() {
            return Err(PresetError::EmptyName);
        }
        if parent.depth() > 0 && parent.depth() > self.nesting {
            return Err(PresetError::TooDeep);
        }
        let siblings = self.group(parent).ok_or(PresetError::NoGroup)?;
        let taken = |n: &str| siblings.groups.iter().any(|g| same_name(&g.name, n));
        let name =
            if taken(base) { (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap_or_else(|| base.to_owned()) } else { base.to_owned() };
        let path = parent.child(name.clone());
        let target = self.group_mut(parent).ok_or(PresetError::NoGroup)?;
        target.groups.push(PresetGroup { name, items: Vec::new(), groups: Vec::new() });
        Ok(path)
    }

    /// Renames preset `id`: trimmed; [`PresetError::EmptyName`] for a blank name,
    /// [`PresetError::NameTaken`] when another preset has it ignoring case (a change of case of
    /// its own name is allowed), [`PresetError::Unchanged`] for exactly its current name.
    pub fn rename(&mut self, id: PresetId, name: &str) -> Result<PresetChange, PresetError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(PresetError::EmptyName);
        }
        let Some(current) = self.get(id) else { return Err(PresetError::NoPreset) };
        if current.name == name {
            return Err(PresetError::Unchanged);
        }
        if self.items().iter().any(|i| i.id != id && same_name(&i.name, name)) {
            return Err(PresetError::NameTaken);
        }
        let item = self.get_mut(id).ok_or(PresetError::NoPreset)?;
        item.name = name.to_owned();
        Ok(PresetChange::Renamed { id, name: item.name.clone() })
    }

    /// Renames group `path` among its siblings, with the same rules as [`rename`](Self::rename).
    pub fn rename_group(&mut self, path: &GroupPath, name: &str) -> Result<PresetChange, PresetError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(PresetError::EmptyName);
        }
        let parent = path.parent().ok_or(PresetError::NoGroup)?;
        let siblings = self.group(&parent).ok_or(PresetError::NoGroup)?;
        if !siblings.groups.iter().any(|g| g.name == path.name()) {
            return Err(PresetError::NoGroup);
        }
        if path.name() == name {
            return Err(PresetError::Unchanged);
        }
        if siblings.groups.iter().any(|g| g.name != path.name() && same_name(&g.name, name)) {
            return Err(PresetError::NameTaken);
        }
        let group = self.group_mut(path).ok_or(PresetError::NoGroup)?;
        group.name = name.to_owned();
        Ok(PresetChange::GroupRenamed { from: path.clone(), to: parent.child(name) })
    }

    /// Deletes preset `id`.
    pub fn delete(&mut self, id: PresetId) -> Result<PresetChange, PresetError> {
        let group = self.group_of(id).ok_or(PresetError::NoPreset)?;
        let target = self.group_mut(&group).ok_or(PresetError::NoGroup)?;
        target.items.retain(|i| i.id != id);
        Ok(PresetChange::Deleted(id))
    }

    /// Deletes group `path` and everything inside it. The root cannot be deleted
    /// ([`PresetError::NoGroup`]).
    pub fn delete_group(&mut self, path: &GroupPath) -> Result<PresetChange, PresetError> {
        let parent = path.parent().ok_or(PresetError::NoGroup)?;
        let removed = {
            let siblings = self.group_mut(&parent).ok_or(PresetError::NoGroup)?;
            let at = siblings.groups.iter().position(|g| g.name == path.name()).ok_or(PresetError::NoGroup)?;
            siblings.groups.remove(at)
        };
        let mut items = Vec::new();
        collect_ids(&removed, &mut items);
        Ok(PresetChange::GroupDeleted { path: path.clone(), items })
    }

    /// Moves preset `id` to the end of group `to` (Move to).
    pub fn move_to(&mut self, id: PresetId, to: &GroupPath) -> Result<PresetChange, PresetError> {
        let from = self.group_of(id).ok_or(PresetError::NoPreset)?;
        if self.group(to).is_none() {
            return Err(PresetError::NoGroup);
        }
        let source = self.group_mut(&from).ok_or(PresetError::NoGroup)?;
        let at = source.items.iter().position(|i| i.id == id).ok_or(PresetError::NoPreset)?;
        let item = source.items.remove(at);
        let target = self.group_mut(to).ok_or(PresetError::NoGroup)?;
        target.items.push(item);
        Ok(PresetChange::Moved { id, to: to.clone() })
    }

    /// Preset `id`.
    pub fn get(&self, id: PresetId) -> Option<&PresetItem<T>> {
        find_item(&self.root, id)
    }

    /// Preset `id`, to change its value.
    pub fn get_mut(&mut self, id: PresetId) -> Option<&mut PresetItem<T>> {
        find_item_mut(&mut self.root, id)
    }

    /// The group at `path` (the root for the empty path).
    pub fn group(&self, path: &GroupPath) -> Option<&PresetGroup<T>> {
        path.names().iter().try_fold(&self.root, |group, name| group.groups.iter().find(|g| g.name == *name))
    }

    /// The group that holds preset `id`.
    pub fn group_of(&self, id: PresetId) -> Option<GroupPath> {
        find_group_of(&self.root, id, &GroupPath::root())
    }

    /// Why `name` cannot be a new preset's name, if it cannot: blank, or taken ignoring case.
    /// For a named row's `check_name`.
    pub fn name_error(&self, name: &str) -> Option<PresetError> {
        if name.trim().is_empty() {
            Some(PresetError::EmptyName)
        } else if self.is_taken(name.trim()) {
            Some(PresetError::NameTaken)
        } else {
            None
        }
    }

    /// The number of presets.
    pub fn total(&self) -> usize {
        self.items().len()
    }

    /// Every preset, in display order (root presets, then each group's presets and folders depth
    /// first).
    pub fn items(&self) -> Vec<&PresetItem<T>> {
        let mut out = Vec::new();
        collect_items(&self.root, &mut out);
        out
    }

    /// The browser's display rows: the root's presets at depth 0, then each group as a header
    /// row (depth = its depth - 1, `count` = the presets inside it and its folders) followed by
    /// its presets (depth = the group's depth) and its folders, depth first.
    pub fn rows(&self) -> Vec<PresetRow<PresetKey<PresetId>>> {
        let mut rows = Vec::new();
        for item in &self.root.items {
            rows.push(PresetRow::item(PresetKey::Item(item.id), item.name.clone(), 0));
        }
        for group in &self.root.groups {
            push_group_rows(group, &GroupPath::root(), &mut rows);
        }
        rows
    }

    pub(super) fn group_mut(&mut self, path: &GroupPath) -> Option<&mut PresetGroup<T>> {
        path.names().iter().try_fold(&mut self.root, |group, name| group.groups.iter_mut().find(|g| g.name == *name))
    }

    fn is_taken(&self, name: &str) -> bool {
        self.items().iter().any(|i| same_name(&i.name, name))
    }

    /// `base`, or "base 2", "base 3", … so the name is free across the library.
    fn unique_name(&self, base: &str) -> String {
        if !self.is_taken(base) {
            return base.to_owned();
        }
        (2..).map(|i| format!("{base} {i}")).find(|n| !self.is_taken(n)).unwrap_or_else(|| base.to_owned())
    }
}

pub(super) fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

fn collect_items<'a, T>(group: &'a PresetGroup<T>, out: &mut Vec<&'a PresetItem<T>>) {
    out.extend(group.items.iter());
    for child in &group.groups {
        collect_items(child, out);
    }
}

fn collect_ids<T>(group: &PresetGroup<T>, out: &mut Vec<PresetId>) {
    out.extend(group.items.iter().map(|i| i.id));
    for child in &group.groups {
        collect_ids(child, out);
    }
}

fn find_item<T>(group: &PresetGroup<T>, id: PresetId) -> Option<&PresetItem<T>> {
    group.items.iter().find(|i| i.id == id).or_else(|| group.groups.iter().find_map(|g| find_item(g, id)))
}

fn find_item_mut<T>(group: &mut PresetGroup<T>, id: PresetId) -> Option<&mut PresetItem<T>> {
    if let Some(at) = group.items.iter().position(|i| i.id == id) {
        return Some(&mut group.items[at]);
    }
    group.groups.iter_mut().find_map(|g| find_item_mut(g, id))
}

fn find_group_of<T>(group: &PresetGroup<T>, id: PresetId, path: &GroupPath) -> Option<GroupPath> {
    if group.items.iter().any(|i| i.id == id) {
        return Some(path.clone());
    }
    group.groups.iter().find_map(|g| find_group_of(g, id, &path.child(g.name.clone())))
}

fn count_items<T>(group: &PresetGroup<T>) -> usize {
    group.items.len() + group.groups.iter().map(count_items).sum::<usize>()
}

/// The header row of `group` (inside `parent`), then its presets and its folders.
fn push_group_rows<T>(group: &PresetGroup<T>, parent: &GroupPath, rows: &mut Vec<PresetRow<PresetKey<PresetId>>>) {
    let path = parent.child(group.name.clone());
    rows.push(PresetRow::group(PresetKey::Group(path.clone()), group.name.clone(), path.depth() - 1).count(count_items(group)));
    for item in &group.items {
        rows.push(PresetRow::item(PresetKey::Item(item.id), item.name.clone(), path.depth()));
    }
    for child in &group.groups {
        push_group_rows(child, &path, rows);
    }
}
