//! The swatch library model: keys, state, groups and their editing.

use std::collections::HashSet;

use craft_ui::contract::ReorderMove;
use craft_ui::patterns::presets::PresetState;

use super::{MAX_GROUPS, MAX_SWATCHES, SwatchesWords};
use crate::palette::{SwatchColor, SwatchEntry, SwatchGroup, SwatchReport};
use crate::ui::PickedColor;

/// What the browser names its rows by. Swatch names are unique across the library and group names among groups, so
/// both survive reordering.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SwatchKey {
    /// A group, by name.
    Group(String),
    /// A swatch, by name.
    Swatch(String),
}

/// What the Swatches panel keeps between passes, owned by the host: the browser's view, the selected row, the row to
/// rename in place next, and the notes of an export in flight.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SwatchesState {
    /// The browser's view settings: thumbnail size, open groups.
    pub browser: PresetState<SwatchKey>,
    /// The selected row.
    pub selected: Option<SwatchKey>,
    /// A row to open the rename on in the next pass.
    pub rename: Option<SwatchKey>,
    /// The notes of the export whose file the host is writing.
    pub(super) export_notes: Option<SwatchReport>,
}

impl Default for SwatchesState {
    fn default() -> Self {
        SwatchesState { browser: PresetState::new(), selected: None, rename: None, export_notes: None }
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
        self.groups.iter().enumerate().find_map(|(group, g)| g.entries.iter().position(|e| e.name == name).map(|index| SwatchAt { group, index }))
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
        self.add_numbered(color, name, group, SwatchesWords::default().swatch_n.as_str())
    }

    /// [`add`](SwatchLibrary::add) with the template (`{n}`) that names a swatch left without one.
    pub(super) fn add_numbered(&mut self, color: SwatchColor, name: Option<&str>, group: Option<usize>, template: &str) -> Option<SwatchAt> {
        if self.groups.iter().map(|g| g.entries.len()).sum::<usize>() >= MAX_SWATCHES {
            return None;
        }
        let name = name.map(str::trim).filter(|n| !n.is_empty() && self.find(n).is_none()).map_or_else(|| self.free_name(template), str::to_owned);
        if self.groups.is_empty() {
            self.groups.push(SwatchGroup { name: "Swatches".to_owned(), entries: Vec::new() });
        }
        let group = group.unwrap_or(0).min(self.groups.len() - 1);
        let entries = &mut self.groups[group].entries;
        entries.push(SwatchEntry::new(name, color));
        Some(SwatchAt { group, index: entries.len() - 1 })
    }

    /// The first free name of `template`, numbered from 1.
    fn free_name(&self, template: &str) -> String {
        let taken: HashSet<&str> = self.groups.iter().flat_map(|g| g.entries.iter().map(|e| e.name.as_str())).collect();
        (1..).map(|n| craft_i18n::fmt_plain(template, &[("n", &n.to_string())])).find(|name| !taken.contains(name.as_str())).unwrap_or_default()
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
        let name = name.trim();
        if name.is_empty() || self.get(at).is_none() {
            return false;
        }
        if self.find(name).is_some_and(|other| other != at) {
            return false;
        }
        self.groups[at.group].entries[at.index].name = name.to_owned();
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
        let name = self.unique_group_name(name.trim());
        self.groups.push(SwatchGroup { name, entries: Vec::new() });
        Some(self.groups.len() - 1)
    }

    /// `base`, or `base 1`, `base 2`… when a group has it.
    fn unique_group_name(&self, base: &str) -> String {
        let taken = |n: &str| self.groups.iter().any(|g| g.name == n);
        if !taken(base) { base.to_owned() } else { (1..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap_or_default() }
    }

    /// Renames a group; `false` when the name is empty or another group has it.
    pub(super) fn rename_group(&mut self, group: usize, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() || group >= self.groups.len() || self.groups.iter().enumerate().any(|(i, g)| i != group && g.name == name) {
            return false;
        }
        self.groups[group].name = name.to_owned();
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
    /// groups. `to` counts in the flat rows (a header, then its swatches) that remain once the moved ones are
    /// taken out. Anything else leaves the library as it is.
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

    /// The flat rows of the browser: each group's header, then its swatches.
    fn flat(&self) -> Vec<SwatchKey> {
        let mut rows = Vec::new();
        for group in &self.groups {
            rows.push(SwatchKey::Group(group.name.clone()));
            rows.extend(group.entries.iter().map(|e| SwatchKey::Swatch(e.name.clone())));
        }
        rows
    }

    fn move_swatches(&mut self, names: &[&str], to: usize, parent: Option<&SwatchKey>) -> bool {
        let moved = |e: &SwatchEntry| names.contains(&e.name.as_str());
        let remaining: Vec<SwatchKey> =
            self.flat().into_iter().filter(|k| !matches!(k, SwatchKey::Swatch(n) if names.contains(&n.as_str()))).collect();
        let before = &remaining[..to.min(remaining.len())];
        // The slot is counted in the entries that stay, so it survives taking the moved ones out.
        let (group, slot) = match before.last() {
            Some(SwatchKey::Swatch(n)) => {
                let Some(at) = self.find(n) else { return false };
                let entries = &self.groups[at.group].entries;
                let slot = entries[..=at.index].iter().filter(|e| !moved(e)).count();
                match parent {
                    Some(SwatchKey::Group(g)) if self.groups[at.group].name != *g => match self.groups.iter().position(|x| x.name == *g) {
                        Some(group) => (group, 0),
                        None => return false,
                    },
                    _ => (at.group, slot),
                }
            }
            Some(SwatchKey::Group(g)) => match self.groups.iter().position(|x| x.name == *g) {
                Some(group) => (group, 0),
                None => return false,
            },
            None => (0, 0),
        };
        if group >= self.groups.len() {
            return false;
        }
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
        let moved = |g: &SwatchGroup| names.contains(&g.name.as_str());
        let remaining: Vec<SwatchKey> = {
            let mut rows = Vec::new();
            for group in self.groups.iter().filter(|g| !moved(g)) {
                rows.push(SwatchKey::Group(group.name.clone()));
                rows.extend(group.entries.iter().map(|e| SwatchKey::Swatch(e.name.clone())));
            }
            rows
        };
        let slot = remaining[..to.min(remaining.len())].iter().filter(|k| matches!(k, SwatchKey::Group(_))).count();
        let (taken, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.groups).into_iter().partition(|g| moved(g));
        self.groups = keep;
        if taken.is_empty() {
            return false;
        }
        let slot = slot.min(self.groups.len());
        self.groups.splice(slot..slot, taken);
        true
    }
}

impl SwatchLibrary {
    /// The index of the group `key` is, or is in.
    pub(super) fn group_of(&self, key: &SwatchKey) -> Option<usize> {
        match key {
            SwatchKey::Swatch(name) => self.find(name).map(|at| at.group),
            SwatchKey::Group(name) => self.groups.iter().position(|g| g.name == *name),
        }
    }

    /// Whether `key` names a row of the library.
    pub(super) fn has(&self, key: &SwatchKey) -> bool {
        match key {
            SwatchKey::Swatch(name) => self.find(name).is_some(),
            SwatchKey::Group(name) => self.groups.iter().any(|g| g.name == *name),
        }
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
