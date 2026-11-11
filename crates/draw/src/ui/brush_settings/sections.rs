//! The Brush Settings section list ([`BrushSectionList`]).

use craft_behavior::{ColumnKey, ColumnWidth, DisplayOrder, IconId, ItemSource, Mixed, Selection, SelectionChange, Value};
use craft_ui::contract::{
    ActivateOn, Bind, CollectionEvent, ColumnKind, ColumnSpec, Outcome, Remember as _, SelectMode, ShowCollection as _, ShowContainer as _, Sizing,
    SpaceAction, SweepApply, SweepStep, ToggleColumn, ToggleEdit,
};
use craft_ui::layout_components::Stack;
use craft_ui::list::{ListRow, ToggleCell, ToggleState};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::table::Table;
use egui::{Response, Ui};
use std::borrow::Cow;
use std::hash::Hash;

use super::words::BrushSettingsWords;

/// The enable box column, at the reading start.
const CHECK: ColumnKey = ColumnKey::new("check");
/// The names.
const NAME: ColumnKey = ColumnKey::new("name");
/// The lock column, at the reading end.
const LOCK: ColumnKey = ColumnKey::new("lock");

/// What a section row does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BrushSectionKind {
    /// A section with a pane of controls (Brush Tip Shape, Shape Dynamics, …, Brush Pose): a click
    /// shows its pane.
    #[default]
    Pane,
    /// A switch with no pane (Noise, Wet Edges, Build-up, Smoothing, Protect Texture): a click
    /// toggles it and the shown pane stays.
    Switch,
}

/// One row of the section list, built by the app each pass from its brush settings.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct BrushSection<K> {
    /// The key events name the section by.
    pub key: K,
    /// The section's name, shown and read by a screen reader.
    pub name: String,
    /// Pane or switch.
    pub kind: BrushSectionKind,
    /// The enable box's state (a switch's state); `None`: no box (Brush Tip Shape).
    pub enabled: Option<bool>,
    /// The lock's state; `None`: the row has no lock.
    pub locked: Option<bool>,
}

impl<K> BrushSection<K> {
    /// A pane section without an enable box or lock.
    pub fn pane(key: K, name: impl Into<String>) -> Self {
        Self { key, name: name.into(), kind: BrushSectionKind::Pane, enabled: None, locked: None }
    }

    /// A switch row, on or off.
    pub fn switch(key: K, name: impl Into<String>, on: bool) -> Self {
        Self { key, name: name.into(), kind: BrushSectionKind::Switch, enabled: Some(on), locked: None }
    }

    /// Gives the row an enable box in this state.
    #[must_use]
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = Some(on);
        self
    }

    /// Gives the row a lock in this state.
    #[must_use]
    pub fn locked(mut self, locked: bool) -> Self {
        self.locked = Some(locked);
        self
    }

    /// Whether the section is off: it has a box and the box is clear. Its pane shows greyed.
    pub fn is_off(&self) -> bool {
        self.enabled == Some(false)
    }
}

/// What the section list reports: one-shot changes, in the order they happened. The app applies
/// them to its brush settings as tool options (no history state).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BrushSectionEvent<K> {
    /// A pane row was clicked (or Enter, or the lead moved onto it): show its pane.
    Select(K),
    /// A pane section's enable box changed, or a click on its row turned it on.
    #[non_exhaustive]
    Enable {
        /// The section.
        key: K,
        /// The new state.
        on: bool,
    },
    /// A section's lock changed: a locked section keeps its settings when a preset is picked.
    #[non_exhaustive]
    Lock {
        /// The section.
        key: K,
        /// The new state.
        on: bool,
    },
    /// A switch row changed.
    #[non_exhaustive]
    Switch {
        /// The section.
        key: K,
        /// The new state.
        on: bool,
    },
    /// The "Brushes" link row was clicked: open the Brushes panel.
    OpenLink,
}

/// What [`BrushSectionList::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the list's events"]
pub struct BrushSectionListOutput<K> {
    /// The list.
    pub response: Response,
    /// This pass's changes, in order.
    pub events: Vec<BrushSectionEvent<K>>,
}

impl<K> Outcome for BrushSectionListOutput<K> {
    type Value = craft_ui::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A row of the laid-out list.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Slot<K> {
    /// The "Brushes" link row.
    Link,
    /// Empty room (a spacer row, so keys and clicks skip it), counted from 0.
    Gap(u8),
    /// The section with this key.
    Section(K),
}

/// The rows in display order over the sections.
struct Rows<'a, K> {
    sections: &'a [BrushSection<K>],
    slots: Vec<Slot<K>>,
}

impl<'a, K: Clone + Eq> Rows<'a, K> {
    /// The link row and its gap, then the sections with a gap after the first row and one before the first switch
    /// row.
    fn new(sections: &'a [BrushSection<K>], link_row: bool) -> Self {
        let mut slots = Vec::new();
        let mut gaps = 0..;
        let mut gap = |slots: &mut Vec<Slot<K>>| slots.push(Slot::Gap(gaps.next().unwrap_or_default()));
        if link_row {
            slots.push(Slot::Link);
            gap(&mut slots);
        }
        let first_switch = sections.iter().position(|section| section.kind == BrushSectionKind::Switch);
        for (at, section) in sections.iter().enumerate() {
            if first_switch == Some(at) && at > 0 {
                gap(&mut slots);
            }
            slots.push(Slot::Section(section.key.clone()));
            if at == 0 && sections.len() > 1 && first_switch != Some(1) {
                gap(&mut slots);
            }
        }
        Rows { sections, slots }
    }

    fn section(&self, key: &K) -> Option<&'a BrushSection<K>> {
        self.sections.iter().find(|section| section.key == *key)
    }
}

impl<K: Clone + Eq> DisplayOrder<Slot<K>> for Rows<'_, K> {
    fn len(&self) -> usize {
        self.slots.len()
    }

    fn key(&self, at: usize) -> Slot<K> {
        self.slots[at].clone()
    }

    fn is_section(&self, at: usize) -> bool {
        matches!(self.slots[at], Slot::Gap(_))
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        match &self.slots[at] {
            Slot::Section(key) => self.section(key).map(|section| Cow::Borrowed(section.name.as_str())),
            _ => None,
        }
    }
}

impl<K: Clone + Eq> ItemSource<Slot<K>> for Rows<'_, K> {
    fn toggled(&self, at: usize, column: &ColumnKey) -> bool {
        let Slot::Section(key) = &self.slots[at] else { return false };
        let Some(section) = self.section(key) else { return false };
        if *column == LOCK { section.locked == Some(true) } else { section.enabled == Some(true) }
    }
}

/// Photoshop's Brush Settings section list: an optional "Brushes" link row, the pane rows with
/// their enable boxes and locks, and after a gap the switch rows. A `Table` of `ListRow`s with an
/// enable box column at the reading start and a lock column at the reading end.
///
/// - **Look.** Rows are the list's own; Brush Tip Shape is in the strong weight; a spacer row follows the first
///   row and precedes the first switch row, taking room and having no AccessKit node. The link row has the brush
///   icon.
/// - **Pointer.** A click on a pane row reports `Select` and, if its box is clear, `Enable { on: true }`; the box
///   toggles only the flag; the lock toggles only the lock; a click on a switch row toggles it (`Switch`).
/// - **Keys.** One Tab stop. The list's own keys (Up, Down, Home, End, Page Up and Page Down) move the lead; onto a
///   pane row it also reports `Select`. Space toggles the lead's box or switch; Enter selects a pane row and turns
///   it on.
/// - **Selection.** `shown` is the pane shown; a switch row or the link row never takes the selection.
/// - **AccessKit.** A table named by the label; a pane row's box is a check box "Enable {name}", its lock a button
///   named "Lock: {name}"; a switch row has one check box.
/// - **RTL.** Mirrors: the box at the reading start, the lock at the reading end.
#[must_use = "show the list with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
#[pattern(probe(BrushSectionList<'static, u8>))]
pub struct BrushSectionList<'a, K: Value + Eq + Hash> {
    label: Label,
    sections: &'a [BrushSection<K>],
    shown: Bind<'a, K>,
    /// Shows the "Brushes" link row at the top (Photoshop; reports `OpenLink`).
    #[prop(flag)]
    link_row: bool,
}

type ListEvents<K> = Vec<(CollectionEvent<Slot<K>, craft_ui::list::RowEvent>, Option<ToggleEdit<Slot<K>>>)>;

impl<'a, K: Value + Eq + Hash> BrushSectionList<'a, K> {
    /// The list named `label` of `sections`, in order (pane rows first, then switch rows), with the pane in `shown`
    /// selected.
    pub fn new(label: impl Into<Label>, sections: &'a [BrushSection<K>], shown: Bind<'a, K>) -> Self {
        Self { label: label.into(), sections, shown, link_row: false }
    }

    /// Shows the list.
    pub fn show(self, ui: &mut Ui) -> BrushSectionListOutput<K> {
        let BrushSectionList { label, sections, mut shown, link_row } = self;
        let words = Env::of(ui).words::<BrushSettingsWords>();
        let rows = Rows::new(sections, link_row);
        let enable_name = Label::from(words.enable.fill(&[("name", "{row}")]));
        let lock_name = Label::from(words.lock.fill(&[("name", "{row}")]));
        let switch_name = Label::from("{row}".to_owned());
        let columns = || {
            let toggle = ColumnKind::Toggle(ToggleColumn::default());
            vec![
                ColumnSpec::new(CHECK, Label::Empty, ColumnWidth::Controls(1.0)).kind(toggle),
                ColumnSpec::new(NAME, Label::Empty, ColumnWidth::Content),
                ColumnSpec::new(LOCK, Label::Empty, ColumnWidth::Controls(1.0)).kind(toggle).icon(IconId::new("lock")),
            ]
        };
        let shown_stack = Stack::new().show(ui, |ui, cx| {
            let pane = match shown.reborrow().read(cx) {
                Some(Mixed::Same(key)) => {
                    rows.section(&key).filter(|section| section.kind == BrushSectionKind::Pane).map(|section| section.key.clone())
                }
                _ => None,
            };
            let selection = pane.clone().map_or_else(Selection::default, |key| Selection::single(Slot::Section(key)));
            let table = Table::<Slot<K>>::new(label.clone(), columns())
                .selection(Bind::given(selection))
                .select(SelectMode::Single)
                .allow_empty(false)
                .header(false)
                .activate(ActivateOn::Click)
                .space(SpaceAction::ToggleCell)
                .sizing(Sizing::Content);
            let out = table.show_items(ui, &rows, &mut |key, icx| {
                let keep = |column: &ColumnKey, uses: &[ColumnKey]| uses.contains(column);
                let placed = |uses: &[ColumnKey]| -> Vec<_> { icx.cells().iter().filter(|(column, _)| keep(column, uses)).cloned().collect() };
                match key {
                    Slot::Gap(_) => ListRow::spacer(),
                    Slot::Link => ListRow::new(words.brushes.clone()).cells(&placed(&[NAME])).icon(IconSpec::svg(IconId::new("brush"))).rule(true),
                    Slot::Section(key) => {
                        let Some(section) = rows.section(key) else { return ListRow::spacer() };
                        let mut uses = vec![NAME];
                        let mut row =
                            ListRow::new(Label::from(section.name.clone())).strong(rows.sections.first().is_some_and(|first| first.key == *key));
                        match section.kind {
                            BrushSectionKind::Switch => {
                                uses.insert(0, CHECK);
                                let state = ToggleState::of(section.enabled == Some(true));
                                row = row.toggle(ToggleCell::check(CHECK, state, switch_name.clone()));
                            }
                            BrushSectionKind::Pane => {
                                if let Some(on) = section.enabled {
                                    uses.insert(0, CHECK);
                                    row = row.toggle(ToggleCell::check(CHECK, ToggleState::of(on), enable_name.clone()));
                                }
                                if let Some(locked) = section.locked {
                                    uses.push(LOCK);
                                    let tip = if locked { words.unlock_tip.clone() } else { words.lock_tip.clone() };
                                    row = row.toggle(ToggleCell::lock(LOCK, locked).name(lock_name.clone()).tooltip(tip).always_visible());
                                }
                            }
                        }
                        row.cells(&placed(&uses))
                    }
                }
            });
            // The columns do not sweep: each toggle edit is one press, read once here.
            let pressed: Vec<Option<ToggleEdit<Slot<K>>>> = out
                .events
                .iter()
                .map(|event| match event {
                    CollectionEvent::Toggle(edit) => match cx.keep::<SweepApply>().feed(edit) {
                        Some(SweepStep::Flip(cell) | SweepStep::Restore(cell)) => Some(cell),
                        _ => None,
                    },
                    _ => None,
                })
                .collect();
            (out.response, out.events.into_iter().zip(pressed).collect::<Vec<_>>(), pane)
        });
        let Some((_, list_events, pane)) = shown_stack.inner else {
            return BrushSectionListOutput { response: shown_stack.output, events: Vec::new() };
        };
        let events = Self::events(&rows, list_events, pane.as_ref());
        BrushSectionListOutput { response: shown_stack.output, events }
    }

    /// The section events of the collection's events, in order.
    fn events(rows: &Rows<'_, K>, list_events: ListEvents<K>, pane: Option<&K>) -> Vec<BrushSectionEvent<K>> {
        let mut events = Vec::new();
        // A press on a box or lock flips only that cell, whatever else the press reported for the row.
        let toggled: Vec<Slot<K>> = list_events
            .iter()
            .filter_map(|(event, pressed)| match event {
                CollectionEvent::Toggle(_) => pressed.as_ref().map(|toggle| toggle.key.clone()),
                _ => None,
            })
            .collect();
        for (event, pressed) in list_events {
            match event {
                CollectionEvent::Toggle(_) => {
                    let Some(toggle) = pressed else { continue };
                    let Slot::Section(key) = &toggle.key else { continue };
                    let Some(section) = rows.section(key) else { continue };
                    let key = key.clone();
                    events.push(if toggle.column == LOCK {
                        BrushSectionEvent::Lock { key, on: toggle.on }
                    } else if section.kind == BrushSectionKind::Switch {
                        BrushSectionEvent::Switch { key, on: toggle.on }
                    } else {
                        BrushSectionEvent::Enable { key, on: toggle.on }
                    });
                }
                CollectionEvent::Selection(SelectionChange::Set(keys)) => {
                    for slot in keys.iter().filter(|slot| !toggled.contains(slot)) {
                        let Slot::Section(key) = slot else { continue };
                        let pane_row = rows.section(key).is_some_and(|section| section.kind == BrushSectionKind::Pane);
                        let select = BrushSectionEvent::Select(key.clone());
                        if pane_row && pane != Some(key) && !events.contains(&select) {
                            events.push(select);
                        }
                    }
                }
                CollectionEvent::Activate(slot) if !toggled.contains(&slot) => match slot {
                    Slot::Link => events.push(BrushSectionEvent::OpenLink),
                    Slot::Gap(_) => {}
                    Slot::Section(key) => {
                        let Some(section) = rows.section(&key) else { continue };
                        match section.kind {
                            BrushSectionKind::Pane => {
                                let select = BrushSectionEvent::Select(key.clone());
                                if !events.contains(&select) {
                                    events.push(select);
                                }
                                if section.is_off() {
                                    events.push(BrushSectionEvent::Enable { key, on: true });
                                }
                            }
                            BrushSectionKind::Switch => {
                                events.push(BrushSectionEvent::Switch { key, on: section.enabled != Some(true) });
                            }
                        }
                    }
                },
                _ => {}
            }
        }
        events
    }
}
