//! The swatch library model: keys, state, groups and their editing.

use std::collections::HashSet;

use craft_behavior::Selection;
use craft_cms::convert::CmykConverter;
use craft_theme::Srgba;
use craft_ui::choice::SwatchFace;
use craft_ui::contract::{MemoKey, MemoProps, ReorderMove};
use craft_ui::patterns::presets::{PresetFns, PresetRow, PresetSource, PresetState, ThumbFn, ThumbInfo};
use craft_ui::primitives::{ContentPainterCx, IconSpec};
use craft_ui::runtime::env::Env;
use egui::Rect;
use std::collections::BTreeMap;

use super::{MAX_GROUPS, MAX_SWATCHES, SWATCH_ZOOM, SwatchesWords};
use crate::palette::{SwatchColor, SwatchEntry, SwatchGroup, SwatchPaint, SwatchReport};
use crate::ui::PickedColor;

/// What the browser names its rows by. Swatch names are unique across the library and group names among groups, so
/// both survive reordering.
#[derive(Clone, Debug, PartialEq, Eq, Hash, craft_ui::contract::MemoProps)]
#[non_exhaustive]
pub enum SwatchKey {
    /// A group, by name.
    Group(String),
    /// A swatch, by name.
    Swatch(String),
}

/// What the Swatches panel keeps between passes, owned by the host: the browser's view, the selected rows, the row to
/// rename in place next, and the notes of an export in flight.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SwatchesState {
    /// The browser's view settings: thumbnail size, open groups.
    pub browser: PresetState<SwatchKey>,
    /// The selected rows, with the lead.
    pub selection: Selection<SwatchKey>,
    /// A row to open the rename on in the next pass.
    pub rename: Option<SwatchKey>,
    /// The notes of the export whose file the host is writing.
    pub(super) export_notes: Option<SwatchReport>,
}

impl Default for SwatchesState {
    fn default() -> Self {
        let mut browser = PresetState::new();
        browser.zoom = SWATCH_ZOOM.default_value();
        SwatchesState { browser, selection: Selection::default(), rename: None, export_notes: None }
    }
}

/// The app's swatches: groups of named swatches, names unique across the library. Kept by the
/// app (a settings section or its presets store).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct SwatchLibrary {
    /// The groups, in display order.
    pub groups: Vec<SwatchGroup>,
}

impl Default for SwatchLibrary {
    /// The default swatches ([`default_swatches`](crate::palette::default_swatches)).
    fn default() -> Self {
        SwatchLibrary { groups: crate::palette::default_swatches() }
    }
}

/// Where a swatch is: its group's index and its index in the group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct SwatchAt {
    /// Index of the group.
    pub group: usize,
    /// Index of the swatch within its group.
    pub index: usize,
}

impl SwatchAt {
    /// The swatch at `index` of group `group`.
    pub const fn new(group: usize, index: usize) -> Self {
        SwatchAt { group, index }
    }
}

/// The swatch named `name` among `groups`.
pub(super) fn find_swatch(groups: &[SwatchGroup], name: &str) -> Option<SwatchAt> {
    groups.iter().enumerate().find_map(|(group, g)| g.entries.iter().position(|e| e.name == name).map(|index| SwatchAt { group, index }))
}

/// The first name of `template` (numbered from 1) no swatch of `groups` has.
pub(super) fn free_name(groups: &[SwatchGroup], template: &str) -> String {
    let taken: HashSet<&str> = groups.iter().flat_map(|g| g.entries.iter().map(|e| e.name.as_str())).collect();
    (1..).map(|n| craft_i18n::fmt_plain(template, &[("n", &n.to_string())])).find(|name| !taken.contains(name.as_str())).unwrap_or_default()
}

/// `base`, or `base 1`, `base 2`… when a group of `groups` has it.
pub(super) fn unique_group_name(groups: &[SwatchGroup], base: &str) -> String {
    let taken = |n: &str| groups.iter().any(|g| g.name == n);
    if !taken(base) { base.to_owned() } else { (1..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap_or_default() }
}

/// The index of the group `key` is, or is in.
pub(super) fn group_of(groups: &[SwatchGroup], key: &SwatchKey) -> Option<usize> {
    match key {
        SwatchKey::Swatch(name) => find_swatch(groups, name).map(|at| at.group),
        SwatchKey::Group(name) => groups.iter().position(|g| g.name == *name),
    }
}

/// Whether `key` names a row of `groups`.
pub(super) fn has(groups: &[SwatchGroup], key: &SwatchKey) -> bool {
    group_of(groups, key).is_some()
}

/// Whether the groups hold the most swatches a library may.
pub(super) fn swatches_full(groups: &[SwatchGroup]) -> bool {
    groups.iter().map(|g| g.entries.len()).sum::<usize>() >= MAX_SWATCHES
}

/// The name the swatch at `at` takes when renamed to `text`; `None` when `at` names nothing, the name is empty or
/// another swatch has it.
pub(super) fn swatch_rename(groups: &[SwatchGroup], at: SwatchAt, text: &str) -> Option<String> {
    let name = text.trim();
    groups.get(at.group)?.entries.get(at.index)?;
    (!name.is_empty() && find_swatch(groups, name).is_none_or(|other| other == at)).then(|| name.to_owned())
}

/// The name group `index` takes when renamed to `text`; `None` when there is no such group, the name is empty or
/// another group has it.
pub(super) fn group_rename(groups: &[SwatchGroup], index: usize, text: &str) -> Option<String> {
    let name = text.trim();
    groups.get(index)?;
    (!name.is_empty() && !groups.iter().enumerate().any(|(i, g)| i != index && g.name == name)).then(|| name.to_owned())
}

/// Whether the browser shows a header row for `group`: every group but the one with an empty name, which holds the
/// swatches outside every group.
fn has_header(group: &SwatchGroup) -> bool {
    !group.name.is_empty()
}

/// The flat rows of the browser: each group's header (none for the swatches outside every group), then its swatches.
fn flat<'a>(groups: impl IntoIterator<Item = &'a SwatchGroup>) -> Vec<SwatchKey> {
    let mut rows = Vec::new();
    for group in groups {
        if has_header(group) {
            rows.push(SwatchKey::Group(group.name.clone()));
        }
        rows.extend(group.entries.iter().map(|e| SwatchKey::Swatch(e.name.clone())));
    }
    rows
}

/// The group and the slot swatches `names` land in when dropped at flat row `to` (counted over the rows that remain
/// once they are out). The slot counts the entries that stay, so it survives taking the moved ones out.
fn resolve_move(groups: &[SwatchGroup], names: &[&str], to: usize, parent: Option<&SwatchKey>) -> Option<(usize, usize)> {
    let moved = |e: &SwatchEntry| names.contains(&e.name.as_str());
    let remaining: Vec<SwatchKey> = flat(groups).into_iter().filter(|k| !matches!(k, SwatchKey::Swatch(n) if names.contains(&n.as_str()))).collect();
    let before = &remaining[..to.min(remaining.len())];
    let (group, slot) = match before.last() {
        Some(SwatchKey::Swatch(n)) => {
            let at = find_swatch(groups, n)?;
            let slot = groups[at.group].entries[..=at.index].iter().filter(|e| !moved(e)).count();
            match parent {
                Some(SwatchKey::Group(g)) if groups[at.group].name != *g => (groups.iter().position(|x| x.name == *g)?, 0),
                _ => (at.group, slot),
            }
        }
        Some(SwatchKey::Group(g)) => (groups.iter().position(|x| x.name == *g)?, 0),
        None => (0, 0),
    };
    (group < groups.len()).then_some((group, slot))
}

/// The slot groups `names` land in when dropped at flat row `to`, counted in the groups that stay: each staying group
/// whose first row (its header, or its first swatch when it has none) is above `to`. Groups never land above the
/// swatches outside every group when those come first.
fn group_slot(groups: &[SwatchGroup], names: &[&str], to: usize) -> usize {
    let mut row = 0;
    let mut slot = 0;
    let mut staying = groups.iter().filter(|g| !names.contains(&g.name.as_str())).peekable();
    let floor = usize::from(staying.peek().is_some_and(|g| !has_header(g)));
    for group in staying {
        if row < to {
            slot += 1;
        }
        row += usize::from(has_header(group)) + group.entries.len();
    }
    slot.max(floor)
}

/// The `at` of each edit that moves `moved` (in library order) one by one within `order`, so that they end up together
/// at `slot` among the names that stay. Edits run in turn, so each index treats the moved names still waiting in the
/// list as absent: only the staying names and the ones already placed count.
fn move_slots(order: &[&str], moved: &[&str], slot: usize) -> Vec<usize> {
    let mut list: Vec<&str> = order.to_vec();
    let mut slots = Vec::with_capacity(moved.len());
    for (i, name) in moved.iter().enumerate() {
        list.retain(|n| n != name);
        let anchors = slot + i;
        let mut at = if anchors == 0 { 0 } else { list.len() };
        let mut seen = 0;
        if anchors > 0 {
            for (index, n) in list.iter().enumerate() {
                if !moved[i + 1..].contains(n) {
                    seen += 1;
                    if seen == anchors {
                        at = index + 1;
                        break;
                    }
                }
            }
        }
        list.insert(at, *name);
        slots.push(at);
    }
    slots
}

/// The edits a reorder of the browser makes, one per moved swatch or group in library order; each `at` counts the
/// edits before it, so the host keeps their order. Anything the library cannot do gives none.
pub(super) fn reorder_edits(groups: &[SwatchGroup], mv: &ReorderMove<SwatchKey>) -> Vec<SwatchEdit> {
    if mv.copy {
        return Vec::new();
    }
    let swatches: Option<Vec<&str>> = mv.keys.iter().map(|k| if let SwatchKey::Swatch(n) = k { Some(n.as_str()) } else { None }).collect();
    let group_names: Option<Vec<&str>> = mv.keys.iter().map(|k| if let SwatchKey::Group(n) = k { Some(n.as_str()) } else { None }).collect();
    if let Some(names) = swatches {
        let Some((group, slot)) = resolve_move(groups, &names, mv.to, mv.parent.as_ref()) else { return Vec::new() };
        let target = &groups[group];
        let order: Vec<&str> = target.entries.iter().map(|e| e.name.as_str()).collect();
        let moved: Vec<&str> = groups.iter().flat_map(|g| g.entries.iter()).map(|e| e.name.as_str()).filter(|n| names.contains(n)).collect();
        return move_slots(&order, &moved, slot)
            .into_iter()
            .zip(&moved)
            .map(|(at, name)| SwatchEdit::MoveTo { swatch: (*name).to_owned(), group: target.name.clone(), at })
            .collect();
    }
    let Some(names) = group_names else { return Vec::new() };
    let slot = group_slot(groups, &names, mv.to);
    let order: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
    let moved: Vec<&str> = order.iter().copied().filter(|n| names.contains(n)).collect();
    move_slots(&order, &moved, slot).into_iter().zip(&moved).map(|(to, name)| SwatchEdit::MoveGroup { group: (*name).to_owned(), to }).collect()
}

impl SwatchLibrary {
    /// A library of `groups`.
    pub fn new(groups: Vec<SwatchGroup>) -> Self {
        SwatchLibrary { groups }
    }

    /// Every swatch, in order.
    pub fn total(&self) -> usize {
        self.groups.iter().map(|g| g.entries.len()).sum()
    }

    /// The swatch named `name`.
    pub fn find(&self, name: &str) -> Option<SwatchAt> {
        find_swatch(&self.groups, name)
    }

    /// The swatch at `at`, or `None` when out of range.
    pub fn get(&self, at: SwatchAt) -> Option<&SwatchEntry> {
        self.groups.get(at.group)?.entries.get(at.index)
    }

    /// Adds `color` to group `group` (the first group when `None`, a new "Swatches" group when
    /// the library is empty) under a free name ("Color Swatch N" when `name` is `None` or taken);
    /// where it went, or `None` when the library holds [`MAX_SWATCHES`].
    // pc: crates/engine/src/cmd/swatch_cmds.rs:150-171 (`swatches.add`: the cap, the name, group 0).
    pub fn add(&mut self, color: SwatchColor, name: Option<&str>, group: Option<usize>) -> Option<SwatchAt> {
        self.add_entry(color.into(), name, group)
    }

    /// [`add`](Self::add) for any paint.
    fn add_entry(&mut self, paint: SwatchPaint, name: Option<&str>, group: Option<usize>) -> Option<SwatchAt> {
        if swatches_full(&self.groups) {
            return None;
        }
        let name = name
            .map(str::trim)
            .filter(|n| !n.is_empty() && self.find(n).is_none())
            .map_or_else(|| free_name(&self.groups, SwatchesWords::default().swatch_n.as_str()), str::to_owned);
        if self.groups.is_empty() {
            self.groups.push(SwatchGroup { name: "Swatches".to_owned(), entries: Vec::new() });
        }
        let group = group.unwrap_or(0).min(self.groups.len() - 1);
        let entries = &mut self.groups[group].entries;
        entries.push(SwatchEntry::with_paint(name, paint));
        Some(SwatchAt { group, index: entries.len() - 1 })
    }

    /// Add to Swatches from the picker: in the model the user typed (CMYK inks or Lab values the
    /// picker kept), else RGB; `None` when the library is full.
    // pc: crates/ui-egui/src/panels/swatches_ui.rs:260 `add_from_picker`.
    pub fn add_picked(&mut self, picked: &PickedColor) -> Option<SwatchAt> {
        let color = match picked.model {
            Some(m) if m.model == "cmyk" => SwatchColor::Cmyk([0, 1, 2, 3].map(|i| (m.values[i] / 100.0).clamp(0.0, 1.0))),
            Some(m) if m.model == "lab" => SwatchColor::Lab([m.values[0], m.values[1], m.values[2]]),
            _ => SwatchColor::Rgb(picked.color.to_rgb_array().map(|v| v.clamp(0.0, 1.0))),
        };
        self.add(color, None, None)
    }

    /// Renames a swatch; `false` when the name is empty or taken.
    pub fn rename(&mut self, at: SwatchAt, name: &str) -> bool {
        let Some(name) = swatch_rename(&self.groups, at, name) else { return false };
        self.groups[at.group].entries[at.index].name = name;
        true
    }

    /// Removes and returns the swatch at `at`, or `None` when out of range.
    pub fn remove(&mut self, at: SwatchAt) -> Option<SwatchEntry> {
        let entries = &mut self.groups.get_mut(at.group)?.entries;
        (at.index < entries.len()).then(|| entries.remove(at.index))
    }

    /// Moves a swatch to the end of group `to`.
    pub fn move_to(&mut self, at: SwatchAt, to: usize) {
        if to >= self.groups.len() {
            return;
        }
        if let Some(entry) = self.remove(at) {
            self.groups[to].entries.push(entry);
        }
    }

    /// Adds an empty group named `name` (made unique); its index, or `None` when the library
    /// holds [`MAX_GROUPS`].
    pub fn new_group(&mut self, name: &str) -> Option<usize> {
        if self.groups.len() >= MAX_GROUPS {
            return None;
        }
        let name = unique_group_name(&self.groups, name.trim());
        self.groups.push(SwatchGroup { name, entries: Vec::new() });
        Some(self.groups.len() - 1)
    }

    /// Renames a group; `false` when the name is empty or another group has it.
    fn rename_group(&mut self, group: usize, name: &str) -> bool {
        let Some(name) = group_rename(&self.groups, group, name) else { return false };
        self.groups[group].name = name;
        true
    }

    /// Appends the groups read from a file (names made unique), or replaces the library with
    /// them.
    // pc: crates/ui-egui/src/panels/swatches_ui.rs:155 `import_bytes` (`mode: append | replace`).
    pub fn import(&mut self, groups: Vec<SwatchGroup>, replace: bool) {
        if replace {
            self.groups.clear();
        }
        let mut taken: HashSet<String> = self.groups.iter().flat_map(|g| g.entries.iter().map(|e| e.name.clone())).collect();
        let mut group_taken: HashSet<String> = self.groups.iter().map(|g| g.name.clone()).collect();
        for mut group in groups {
            if group_taken.contains(&group.name) {
                group.name = (1..).map(|i| format!("{} {i}", group.name)).find(|n| !group_taken.contains(n)).unwrap_or_default();
            }
            group_taken.insert(group.name.clone());
            for entry in &mut group.entries {
                if taken.contains(&entry.name) {
                    entry.name = (2..).map(|i| format!("{} {i}", entry.name)).find(|n| !taken.contains(n)).unwrap_or_default();
                }
                taken.insert(entry.name.clone());
            }
            self.groups.push(group);
        }
    }

    /// Applies a reorder of the browser: swatches go into a group at a place, or groups to a place among the
    /// groups. `to` counts in the flat rows (a header, then its swatches; no header for the group with an empty
    /// name) that remain once the moved ones are taken out. Anything else leaves the library as it is.
    pub fn apply_reorder(&mut self, mv: &ReorderMove<SwatchKey>) -> bool {
        if mv.copy {
            return false;
        }
        let swatches: Option<Vec<&str>> = mv.keys.iter().map(|k| if let SwatchKey::Swatch(n) = k { Some(n.as_str()) } else { None }).collect();
        let groups: Option<Vec<&str>> = mv.keys.iter().map(|k| if let SwatchKey::Group(n) = k { Some(n.as_str()) } else { None }).collect();
        if let Some(names) = swatches {
            return self.move_swatches(&names, mv.to, mv.parent.as_ref());
        }
        groups.is_some_and(|names| self.move_groups(&names, mv.to))
    }

    fn move_swatches(&mut self, names: &[&str], to: usize, parent: Option<&SwatchKey>) -> bool {
        let Some((group, slot)) = resolve_move(&self.groups, names, to, parent) else { return false };
        let moved = |e: &SwatchEntry| names.contains(&e.name.as_str());
        let mut taken = Vec::new();
        for g in &mut self.groups {
            let (out, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut g.entries).into_iter().partition(|e| moved(e));
            g.entries = keep;
            taken.extend(out);
        }
        if taken.is_empty() {
            return false;
        }
        let entries = &mut self.groups[group].entries;
        let slot = slot.min(entries.len());
        entries.splice(slot..slot, taken);
        true
    }

    fn move_groups(&mut self, names: &[&str], to: usize) -> bool {
        let slot = group_slot(&self.groups, names, to);
        let (taken, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.groups).into_iter().partition(|g| names.contains(&g.name.as_str()));
        self.groups = keep;
        if taken.is_empty() {
            return false;
        }
        let slot = slot.min(self.groups.len());
        self.groups.splice(slot..slot, taken);
        true
    }
}

/// A swatch's value in its own model, for the tooltip and a screen reader.
pub(super) fn value_text(color: &SwatchColor, book: &str) -> String {
    let pct = |v: f32| (v * 100.0).round();
    match *color {
        SwatchColor::Rgb(v) => {
            let [r, g, b] = v.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
            format!("#{r:02x}{g:02x}{b:02x}")
        }
        SwatchColor::Hsb([h, s, b]) => format!("H {} S {}% B {}%", h.round(), pct(s), pct(b)),
        SwatchColor::Cmyk([c, m, y, k]) => format!("C {}% M {}% Y {}% K {}%", pct(c), pct(m), pct(y), pct(k)),
        SwatchColor::Lab([l, a, b]) => format!("L {} a {} b {}", l.round(), a.round(), b.round()),
        SwatchColor::Gray(k) => format!("K {}%", pct(k)),
        SwatchColor::Book { .. } => book.to_owned(),
    }
}

/// One resolved edit of the swatch library for the host to run.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SwatchEdit {
    /// Adds `paint` named `name` to `group`, or the first group.
    #[non_exhaustive]
    Add {
        /// The group's name, `None` for the first group.
        group: Option<String>,
        /// The new swatch's name.
        name: String,
        /// What the new swatch holds.
        paint: SwatchPaint,
    },
    /// Adds an empty group.
    #[non_exhaustive]
    NewGroup {
        /// The new group's name.
        name: String,
    },
    /// Renames a swatch.
    #[non_exhaustive]
    Rename {
        /// The swatch's current name.
        swatch: String,
        /// Its new name.
        name: String,
    },
    /// Renames a group.
    #[non_exhaustive]
    RenameGroup {
        /// The group's current name.
        group: String,
        /// Its new name.
        name: String,
    },
    /// Deletes a swatch.
    #[non_exhaustive]
    Delete {
        /// The swatch's name.
        swatch: String,
    },
    /// Deletes a group with its swatches.
    #[non_exhaustive]
    DeleteGroup {
        /// The group's name.
        group: String,
    },
    /// Moves a swatch.
    #[non_exhaustive]
    MoveTo {
        /// The swatch's name.
        swatch: String,
        /// The group it goes to.
        group: String,
        /// Its index in `group` once it is taken out: the group's length for a drop on its header.
        at: usize,
    },
    /// Moves a group.
    #[non_exhaustive]
    MoveGroup {
        /// The group's name.
        group: String,
        /// The group's new index.
        to: usize,
    },
    /// Puts the default swatches back.
    Reset,
    /// Imports `groups`, replacing the library when `replace`.
    #[non_exhaustive]
    Import {
        /// The groups the file holds.
        groups: Vec<SwatchGroup>,
        /// Whether they replace the library.
        replace: bool,
    },
}

impl SwatchEdit {
    /// Adds `paint` named `name` to `group`, or the first group.
    pub fn add(group: Option<String>, name: String, paint: impl Into<SwatchPaint>) -> Self {
        SwatchEdit::Add { group, name, paint: paint.into() }
    }

    /// Deletes the swatch named `swatch`.
    pub fn delete(swatch: String) -> Self {
        SwatchEdit::Delete { swatch }
    }

    /// Moves the group named `group` to position `to`.
    pub fn move_group(group: String, to: usize) -> Self {
        SwatchEdit::MoveGroup { group, to }
    }
}

impl SwatchLibrary {
    /// Runs `edit` on the library: the in-memory host's path (stories, tests). False when the edit names nothing in the
    /// library.
    pub fn apply(&mut self, edit: &SwatchEdit) -> bool {
        match edit {
            SwatchEdit::Add { group, name, paint } => {
                let group = match group {
                    Some(g) => match self.groups.iter().position(|x| x.name == *g) {
                        Some(index) => Some(index),
                        None => return false,
                    },
                    None => None,
                };
                self.add_entry(paint.clone(), Some(name), group).is_some()
            }
            SwatchEdit::NewGroup { name } => self.new_group(name).is_some(),
            SwatchEdit::Rename { swatch, name } => self.find(swatch).is_some_and(|at| self.rename(at, name)),
            SwatchEdit::RenameGroup { group, name } => match self.groups.iter().position(|g| g.name == *group) {
                Some(index) => self.rename_group(index, name),
                None => false,
            },
            SwatchEdit::Delete { swatch } => match self.find(swatch) {
                Some(at) => self.remove(at).is_some(),
                None => false,
            },
            SwatchEdit::DeleteGroup { group } => match self.groups.iter().position(|g| g.name == *group) {
                Some(index) => {
                    self.groups.remove(index);
                    true
                }
                None => false,
            },
            SwatchEdit::MoveTo { swatch, group, at } => {
                let (Some(from), Some(to)) = (self.find(swatch), self.groups.iter().position(|g| g.name == *group)) else {
                    return false;
                };
                let Some(entry) = self.remove(from) else { return false };
                let entries = &mut self.groups[to].entries;
                let slot = (*at).min(entries.len());
                entries.insert(slot, entry);
                true
            }
            SwatchEdit::MoveGroup { group, to } => {
                let Some(from) = self.groups.iter().position(|g| g.name == *group) else { return false };
                let moved = self.groups.remove(from);
                let slot = (*to).min(self.groups.len());
                self.groups.insert(slot, moved);
                true
            }
            SwatchEdit::Reset => {
                self.groups = crate::palette::default_swatches();
                true
            }
            SwatchEdit::Import { groups, replace } => {
                self.import(groups.clone(), *replace);
                true
            }
        }
    }
}

/// A host's thumbnail painter over its own inputs, as `PresetFns::thumb` takes one: `key` holds the inputs and the
/// `fn`, so the swatch list keys on what the painter reads.
#[derive(craft_ui::contract::MemoProps)]
pub(super) struct HostThumb<'a> {
    key: u128,
    #[memo_props(covered_by = "key", reason = "the painter runs the host's `fn` over the host's inputs, and `key` holds both")]
    paint: Box<PaintThumb<'a>>,
}

/// The painter a [`HostThumb`] holds: the host's `fn` over its inputs.
type PaintThumb<'a> = dyn Fn(&ContentPainterCx<'_>, Rect, ThumbInfo<'_, SwatchKey>) + Send + 'a;

impl<'a> HostThumb<'a> {
    /// `paint` over `inputs`.
    pub(super) fn new<I: MemoProps + Send + 'a>(inputs: I, paint: ThumbFn<I, SwatchKey>) -> Self {
        let key = MemoKey::of(&(&inputs, paint as usize));
        HostThumb {
            key,
            paint: Box::new(move |cx: &ContentPainterCx<'_>, rect: Rect, info: ThumbInfo<'_, SwatchKey>| paint(&inputs, cx, rect, info)),
        }
    }
}

/// The swatch list's inputs: the library's groups, each swatch's colour as drawn and the icons trailing its row, by
/// name, and the host's thumbnail painter, which paints every swatch in place of the drawn colours.
#[derive(craft_ui::contract::MemoProps)]
struct SwatchInputs<'a> {
    groups: &'a [SwatchGroup],
    drawn: BTreeMap<&'a str, Srgba>,
    icons: BTreeMap<&'a str, Vec<(IconSpec<'static>, String)>>,
    host: Option<HostThumb<'a>>,
}

/// The presets of the swatch list: each group at depth 0 with its count, each swatch at depth 1 with "name, value" as
/// its tooltip and accessible name (the name alone for a swatch without a colour), painted in its colour through
/// `cmyk`. `SwatchesPanel` and `PaintChip`'s swatches list show it.
pub fn swatch_source<'a>(groups: &'a [SwatchGroup], cmyk: &CmykConverter) -> PresetSource<'a, SwatchKey> {
    panel_source(groups, cmyk, BTreeMap::new(), None)
}

/// [`swatch_source`] with `icons` trailing the swatch rows they name and the `host` painter, if any, painting every
/// swatch.
pub(super) fn panel_source<'a>(
    groups: &'a [SwatchGroup],
    cmyk: &CmykConverter,
    icons: BTreeMap<&'a str, Vec<(IconSpec<'static>, String)>>,
    host: Option<HostThumb<'a>>,
) -> PresetSource<'a, SwatchKey> {
    let entries = || groups.iter().flat_map(|g| g.entries.iter());
    let drawn = if host.is_some() {
        BTreeMap::new()
    } else {
        entries().filter_map(|e| e.color().map(|color| (e.name.as_str(), color.to_srgb(cmyk)))).collect()
    };
    let fns = PresetFns::new(source_rows).thumb(paint_swatch).accepts(|_, drag, _, _| {
        let groups = matches!(drag.grabbed, SwatchKey::Group(_));
        drag.keys.iter().all(|key| matches!(key, SwatchKey::Group(_)) == groups)
    });
    PresetSource::new(SwatchInputs { groups, drawn, icons, host }, fns)
}

fn source_rows(inputs: &SwatchInputs<'_>, env: &Env) -> Vec<PresetRow<SwatchKey>> {
    let mut rows = swatch_rows(inputs.groups, &env.words::<SwatchesWords>());
    if !inputs.icons.is_empty() {
        for row in &mut rows {
            if let SwatchKey::Swatch(name) = &row.key
                && let Some(icons) = inputs.icons.get(name.as_str())
            {
                row.trailing.clone_from(icons);
            }
        }
    }
    rows
}

/// The rows the Swatches panel lists for `groups`: each group's header, then its swatches, each named by its name and
/// colour value (the name alone for a swatch without a colour). A group with an empty name holds the swatches outside
/// every group: its swatches show at the top level, with no header.
pub fn swatch_rows(groups: &[SwatchGroup], words: &SwatchesWords) -> Vec<PresetRow<SwatchKey>> {
    let mut rows = Vec::with_capacity(groups.len() + groups.iter().map(|g| g.entries.len()).sum::<usize>());
    for group in groups {
        let header = has_header(group);
        let depth = usize::from(header);
        if header {
            rows.push(PresetRow::group(SwatchKey::Group(group.name.clone()), group.name.as_str(), 0).count(group.entries.len()));
        }
        for entry in &group.entries {
            let named = match entry.color() {
                Some(color) => format!("{}, {}", entry.name, value_text(color, words.book_color.as_str())),
                None => entry.name.clone(),
            };
            rows.push(
                PresetRow::item(SwatchKey::Swatch(entry.name.clone()), entry.name.as_str(), depth).tooltip(named.clone()).accessible_name(named),
            );
        }
    }
    rows
}

fn paint_swatch(inputs: &SwatchInputs<'_>, cx: &ContentPainterCx<'_>, rect: Rect, info: ThumbInfo<'_, SwatchKey>) {
    if let Some(host) = &inputs.host {
        (host.paint)(cx, rect, info);
    } else if let SwatchKey::Swatch(name) = info.key
        && let Some(color) = inputs.drawn.get(name.as_str())
    {
        SwatchFace::new(*color).selected(info.selected).paint(cx, rect);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use craft_ui::testing::trial;

    fn group(name: &str, entries: &[&str]) -> SwatchGroup {
        SwatchGroup { name: name.to_owned(), entries: entries.iter().map(|n| SwatchEntry::new((*n).to_owned(), SwatchColor::Gray(0.5))).collect() }
    }

    /// The swatches outside every group (the group with an empty name) have no header row, so a place below them
    /// counts them as rows: a group dropped there lands after them, and a swatch dropped there joins them. A group
    /// dropped above them lands after them too.
    #[trial]
    fn drops_below_the_ungrouped_swatches_land_after_them() {
        let library = SwatchLibrary::new(vec![group("", &["A", "B"]), group("G", &["C"]), group("H", &["D"])]);
        let rows: Vec<_> = swatch_rows(&library.groups, &SwatchesWords::default()).into_iter().map(|row| row.key).collect();
        assert_eq!(rows, flat(&library.groups), "the reorder counts the rows the browser shows");

        for to in [0, 2] {
            let mut moved = library.clone();
            assert!(moved.apply_reorder(&ReorderMove::new(SwatchKey::Group("H".to_owned()), vec![SwatchKey::Group("H".to_owned())], to, false)));
            let order: Vec<_> = moved.groups.iter().map(|g| g.name.as_str()).collect();
            assert_eq!(order, ["", "H", "G"], "H dropped at row {to} lands below the ungrouped swatches, above G");
        }

        let mut moved = library.clone();
        assert!(moved.apply_reorder(&ReorderMove::new(SwatchKey::Swatch("C".to_owned()), vec![SwatchKey::Swatch("C".to_owned())], 2, false)));
        assert_eq!(moved.find("C"), Some(SwatchAt::new(0, 2)), "C joins the ungrouped swatches, after B");
    }

    /// Edits that move several items down or up give the order the browser's own reorder gives.
    #[trial]
    fn multi_item_moves_apply_as_the_reorder_does() {
        let library = SwatchLibrary::new(vec![group("G", &["A", "B", "C", "D"]), group("H", &["E"])]);
        let swatch = |n: &str| SwatchKey::Swatch(n.to_owned());
        let group_key = |n: &str| SwatchKey::Group(n.to_owned());
        let moves = [
            ReorderMove::new(swatch("A"), vec![swatch("A"), swatch("B")], 3, false),
            ReorderMove::new(swatch("C"), vec![swatch("C"), swatch("D")], 1, false),
            ReorderMove::new(group_key("G"), vec![group_key("G")], 2, false),
        ];
        for mv in moves {
            let mut expected = library.clone();
            assert!(expected.apply_reorder(&mv));
            let mut applied = library.clone();
            for edit in reorder_edits(&library.groups, &mv) {
                assert!(applied.apply(&edit));
            }
            assert_eq!(applied, expected);
        }
    }
}
