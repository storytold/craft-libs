//! `ToolbarDrawer`: Edit Toolbar's All Tools drawer beside the tool rail, where tools move between the toolbar and the
//! drawer.

use super::tool::{DropAt, RailTool, SlotId, ToolDrag, ToolDropped, ToolSlot};
use super::words::RailWords;
use crate::buttons::{Button, IconButton};
use crate::contract::overlay::{Anchor, Dismiss, Overlay, Place};
use crate::contract::{
    ActivateOn, Bind, CollectionEvent, Gap, ListDrag, NoValue, Outcome, PopupKind, Props as _, SelectMode, Show as _, ShowCollection as _,
    ShowContainer as _, Sizing, Trigger, clicked,
};
use crate::display::{Shade, TargetMark, Text};
use crate::layout_components::{Row, Stack};
use crate::list::{List, ListRow, RowInk, RowRun};
use crate::menu::{Menu, PanelMenu, PanelRow};
use crate::popover::Popover;
use crate::primitives::IconSpec;
use crate::runtime::commands::{self, ResolvedCommand};
use crate::runtime::env::{Env, Label};
use crate::runtime::memory::Remember as _;
use crate::runtime::windows::os_of;
use crate::sections::DropAction;
use crate::style::TypeRole;
use crate::text::TextField;
use craft_behavior::overlay::{Align, Side};
use craft_behavior::{DisplayOrder, IconId, ItemSource, Value};
use egui::{Rect, Response, Ui};
use std::borrow::Cow;

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// A group of the drawer: a heading over the tools that belong under it.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct DrawerGroup<T> {
    /// The group's heading.
    pub label: Label,
    /// The group's tools, in the order the drawer lists them.
    pub tools: Vec<RailTool<T>>,
    /// Run lengths over `tools`: the rows of each slot, by position. Empty for a group without slots.
    pub slots: Vec<usize>,
}

impl<T> DrawerGroup<T> {
    /// A group headed `label` that lists `tools`.
    pub fn new(label: impl Into<Label>, tools: Vec<RailTool<T>>) -> Self {
        DrawerGroup { label: label.into(), tools, slots: Vec::new() }
    }

    /// A group headed `label` that lists the tools of `slots` one after another; each row carries its slot's run by
    /// position.
    pub fn slotted(label: impl Into<Label>, slots: Vec<Vec<RailTool<T>>>) -> Self {
        let lengths = slots.iter().map(Vec::len).collect();
        DrawerGroup { label: label.into(), tools: slots.into_iter().flatten().collect(), slots: lengths }
    }
}

/// A change to the toolbar that the user made in the drawer. The host applies it with [`apply_toolbar_edit`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ToolbarEdit<T> {
    /// Put the tool on the toolbar at `at`.
    #[non_exhaustive]
    Add {
        /// The tool.
        tool: RailTool<T>,
        /// Where it goes.
        at: DropAt,
    },
    /// Take every occurrence of the tool off the toolbar. Use `RemoveFrom` to remove one occurrence.
    #[non_exhaustive]
    Remove {
        /// The tool.
        tool: T,
    },
    /// Move a tool by id to `at`, removing its other occurrences. Use `MoveFrom` to move one occurrence.
    #[non_exhaustive]
    Move {
        /// The tool.
        tool: T,
        /// Where it goes.
        at: DropAt,
    },
    /// Remove one occurrence, identified by its slot and index.
    #[non_exhaustive]
    RemoveFrom {
        /// The source slot.
        slot: SlotId,
        /// The index in the source slot.
        index: usize,
    },
    /// Move one occurrence, identified by its slot and index.
    #[non_exhaustive]
    MoveFrom {
        /// The source slot.
        slot: SlotId,
        /// The index in the source slot.
        index: usize,
        /// The destination.
        at: DropAt,
    },
    /// Go back to the default toolbar.
    Reset,
}

impl<T> ToolbarEdit<T> {
    /// Put `tool` on the toolbar at `at`.
    pub fn add(tool: RailTool<T>, at: DropAt) -> Self {
        ToolbarEdit::Add { tool, at }
    }

    /// Take the tool `tool` off the toolbar.
    pub fn remove(tool: T) -> Self {
        ToolbarEdit::Remove { tool }
    }

    /// Move the toolbar's tool `tool` to `at`.
    pub fn move_to(tool: T, at: DropAt) -> Self {
        ToolbarEdit::Move { tool, at }
    }

    /// Remove the occurrence at `index` in `slot`.
    pub fn remove_from(slot: SlotId, index: usize) -> Self {
        ToolbarEdit::RemoveFrom { slot, index }
    }

    /// Move the occurrence at `index` in `slot` to `at`.
    pub fn move_from(slot: SlotId, index: usize, at: DropAt) -> Self {
        ToolbarEdit::MoveFrom { slot, index, at }
    }
}

/// What the drawer reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ToolbarDrawerOutput<T> {
    /// The drawer's response.
    pub response: Response,
    /// The changes made in this pass, in order.
    pub edits: Vec<ToolbarEdit<T>>,
    /// The tool of the row the user selected.
    pub select: Option<T>,
    /// The drawer closed in this pass; the host stops showing it.
    pub closed: bool,
}

impl<T: Value> Outcome for ToolbarDrawerOutput<T> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The All Tools drawer: a popover beside the rail with one list over all groups, a heading row for each group. A
/// tool that is on the toolbar is dimmed. The drawer is as tall as its rows, up to the share of the window a dialog
/// takes, and the list scrolls inside that height.
///
/// - **Rows.** A click on a row, or Enter on the lead row, reports its tool as `select`. The rows pick without
///   selecting: no row keeps a selection fill.
/// - **Drags.** A tool dragged from the rail and dropped on the drawer is taken off the toolbar. A drop on the rail
///   comes back from the rail as a [`ToolDropped`]; hand it to [`ToolbarDrawer::new`] and the drawer turns it into an
///   edit.
/// - **Closing.** Escape, the close button or a press outside the drawer and the rail closes it: `closed` is set and
///   the host stops showing it. A press on the rail is the start of a drag or a pick there, so it leaves the drawer open.
///   The focus then returns to the drawer's [opener](ToolbarDrawer::opened_by), the rail's Edit Toolbar button.
/// - **Menu and footer.** With [`menu`](ToolbarDrawer::menu) rows the header holds a menu button, and
///   [`show_with`](ToolbarDrawer::show_with) draws the host's content in the footer above Restore Defaults.
#[must_use = "show the drawer with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
#[pattern(probe(ToolbarDrawer<'static, u8>))]
pub struct ToolbarDrawer<'a, T: Value + 'static> {
    rail: Rect,
    groups: &'a [DrawerGroup<T>],
    toolbar: &'a [ToolSlot<T>],
    /// The side of the rail the drawer opens on, as given to `ToolRail::opens`: the side of the canvas.
    #[prop]
    opens: Side,
    /// The tool whose row wears the target mark.
    #[prop]
    mark: Option<&'a T>,
    /// A request of the pass: scrolls the tool's row into the list's view. Send it in the pass that needs it, not every
    /// pass.
    #[prop]
    reveal: Option<&'a T>,
    /// With rows, an icon button after the title opens a menu of them.
    #[prop]
    menu: Vec<PanelRow>,
    dropped: Option<ToolDropped>,
    opener: Option<Response>,
    /// Whether the drawer searches: a searchable drawer shows a search field under its title and filters its tool rows.
    #[prop]
    kind: DrawerKind,
    /// PdfCraft's drawer: a tool the toolbar holds only inside a group (behind another tool of its slot) stays
    /// draggable and undimmed, so it can also get a slot of its own. By default any tool on the toolbar is dimmed and
    /// does not drag.
    #[prop(flag)]
    grouped_tools_drag: bool,
}

/// What a `ToolbarDrawer` offers besides its list.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawerKind {
    /// The list alone.
    #[default]
    Plain,
    /// A `TextField::search` under the title filters the tool rows; the query lives in the popover's kept state and
    /// clears on close.
    Searchable,
}

impl<'a, T: Value + 'static> ToolbarDrawer<'a, T> {
    /// A drawer beside `rail` (the rail's rect), listing `groups`, over a toolbar of `toolbar` slots. `dropped` is the
    /// drag the rail took in this pass (`ToolRailOutput::dropped`); the drawer turns it into an edit.
    pub fn new(rail: Rect, groups: &'a [DrawerGroup<T>], toolbar: &'a [ToolSlot<T>], dropped: Option<ToolDropped>) -> Self {
        ToolbarDrawer {
            rail,
            groups,
            toolbar,
            opens: Side::End,
            mark: None,
            reveal: None,
            menu: Vec::new(),
            dropped,
            opener: None,
            kind: DrawerKind::Plain,
            grouped_tools_drag: false,
        }
    }

    /// The control that opened the drawer, the rail's Edit Toolbar button (`ToolRailOutput::edit_toolbar`): the focus
    /// returns to it when the drawer closes. The drawer still hangs from the rail.
    pub fn opened_by(mut self, opener: &Response) -> Self {
        self.opener = Some(opener.clone());
        self
    }
}

/// A row of the drawer's list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum DrawerKey {
    /// The heading row of group `0`.
    Group(usize),
    /// The tool at `tool` of group `group`.
    Tool { group: usize, tool: usize },
}

/// The source of the drawer's list: a heading row for each group that has tools, then its tools. Navigation skips the
/// headings.
struct Rows<'a> {
    keys: Vec<DrawerKey>,
    resolved: &'a [Vec<std::sync::Arc<ResolvedCommand>>],
}

impl<'a> Rows<'a> {
    fn new(resolved: &'a [Vec<std::sync::Arc<ResolvedCommand>>]) -> Self {
        let mut keys = Vec::new();
        for (group, tools) in resolved.iter().enumerate().filter(|(_, tools)| !tools.is_empty()) {
            keys.push(DrawerKey::Group(group));
            keys.extend((0..tools.len()).map(|tool| DrawerKey::Tool { group, tool }));
        }
        Rows { keys, resolved }
    }
}

impl DisplayOrder<DrawerKey> for Rows<'_> {
    fn len(&self) -> usize {
        self.keys.len()
    }

    fn key(&self, at: usize) -> DrawerKey {
        self.keys[at]
    }

    fn index_of(&self, key: &DrawerKey) -> Option<usize> {
        self.keys.iter().position(|held| held == key)
    }

    fn is_section(&self, at: usize) -> bool {
        matches!(self.keys[at], DrawerKey::Group(_))
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        match self.keys[at] {
            DrawerKey::Group(_) => None,
            DrawerKey::Tool { group, tool } => Some(Cow::Borrowed(&*self.resolved[group][tool].label)),
        }
    }
}

impl ItemSource<DrawerKey> for Rows<'_> {}

/// The place of the row at `tool` in the runs `slots` count: `None` without slots or past the last run.
fn run_of(slots: &[usize], tool: usize) -> Option<RowRun> {
    let mut start = 0;
    for length in slots {
        let end = start + length;
        if tool < end {
            return Some(match (tool == start, tool + 1 == end) {
                (true, true) => RowRun::Alone,
                (true, false) => RowRun::First,
                (false, true) => RowRun::Last,
                (false, false) => RowRun::Middle,
            });
        }
        start = end;
    }
    None
}

/// What a searchable drawer keeps in its stack's memory while it is open: the query its search field edits and its
/// list filters by.
#[derive(Clone, Default)]
struct DrawerSearch {
    query: String,
}

/// The texts a query finds a tool by: its label as shown, its catalogue (English) label and its command id.
fn match_texts<T>(command: &ResolvedCommand, tool: &RailTool<T>) -> Vec<Label> {
    let mut texts = vec![Label::from(command.label.clone())];
    texts.extend(command.catalog_label.clone().map(Label::from));
    texts.push(Label::from(tool.command.id.clone()));
    texts
}

/// Whether the toolbar holds the tool `id`: in any slot, or with `own_slot` only in a slot of its own (a tool grouped
/// behind another counts as off the toolbar).
fn on_toolbar<T: PartialEq>(toolbar: &[ToolSlot<T>], id: &T, own_slot: bool) -> bool {
    toolbar.iter().any(|slot| (!own_slot || slot.tools.len() == 1) && slot.tools.iter().any(|tool| tool.id == *id))
}

impl<T: Value + 'static> ToolbarDrawer<'_, T> {
    /// Shows the drawer and reports what the user changed.
    pub fn show(self, ui: &mut Ui) -> ToolbarDrawerOutput<T> {
        self.show_with(ui, |_| {})
    }

    /// [`show`](ToolbarDrawer::show) with `below` drawn in the footer above Restore Defaults.
    pub fn show_with(self, ui: &mut Ui, mut below: impl FnMut(&mut Ui)) -> ToolbarDrawerOutput<T> {
        let ToolbarDrawer { rail, groups, toolbar, opens, mark, reveal, menu, dropped, opener, kind, grouped_tools_drag } = self;
        let searchable = kind == DrawerKind::Searchable;
        let env = Env::of(ui);
        let words = env.words::<RailWords>();
        let os = os_of(ui.ctx());
        let ctx = ui.ctx().clone();
        let resolved: Vec<Vec<std::sync::Arc<ResolvedCommand>>> =
            groups.iter().map(|group| group.tools.iter().map(|tool| commands::resolve(&ctx, tool.command.clone())).collect()).collect();
        let mut edits: Vec<ToolbarEdit<T>> = dropped.and_then(|dropped| drop_edit(dropped.drag, dropped.at, groups, toolbar)).into_iter().collect();
        let mut select: Option<T> = None;
        let mut open = true;
        let mut leave = false;
        let mut restore = false;
        let layout = env.view().keyboard_layout();
        let chord_of = |command: &ResolvedCommand| command.chord.as_ref().map(|chord| chord.display_for(os, layout));
        let key_of = |id: &T| {
            groups
                .iter()
                .enumerate()
                .find_map(|(group, held)| held.tools.iter().position(|tool| tool.id == *id).map(|tool| DrawerKey::Tool { group, tool }))
        };

        let mut popover = Popover::new(words.drawer_title.clone(), Bind::bound(&mut open));
        if let Some(opener) = &opener {
            popover = popover.opened_by(opener);
        }
        let popover = popover
            .overlay(|overlay| {
                let mut overlay = overlay.place(Place::anchored(Anchor::Rect(rail), opens, Align::Start));
                if overlay.opener.is_some() {
                    overlay = overlay.opener_anchor(Anchor::Rect(rail));
                }
                overlay.fit = Overlay::dialog().fit;
                overlay
            })
            .dismiss(Dismiss::new().focus_left(false))
            .collection_body();
        let shown = popover.show(ui, |ui, _| {
            let accepts = |drag: &ToolDrag| matches!(drag, ToolDrag::Rail { .. });
            let slot = DropAction::<ToolDrag>::new(accepts).show(ui, |ui, _| {
                let rows = Rows::new(&resolved);
                let list_id = ui.scope_id().with("drawer-tools");
                // The search field is the stack's header and the list its body; both read the query from the stack's
                // memory, which lives while the drawer is open.
                Stack::new()
                    .gap(Gap::Row)
                    .header(|ui, stack| {
                        Row::new().show(ui, |ui, _| {
                            Text::new(words.drawer_title.clone()).role(TypeRole::Title).show(ui);
                            if !menu.is_empty() {
                                let more = IconButton::new(IconId::new("menu")).label(words.drawer_menu.clone()).popup(PopupKind::Menu).show(ui);
                                let _ = Menu::new(words.drawer_menu.clone()).opened_by(&more, Trigger::Click).show(ui, |ui, _| {
                                    PanelMenu::new(menu.clone(), Vec::new()).show(ui);
                                });
                            }
                            let close = IconButton::new(IconId::new("close")).label(words.drawer_close.clone()).show(ui);
                            leave |= clicked(&close);
                        });
                        if searchable {
                            let state = stack.memory::<DrawerSearch>();
                            TextField::search(words.drawer_search.clone(), Bind::bound(&mut state.query)).drives(list_id).show(ui);
                        }
                        Text::new(words.drawer_hint.clone()).role(TypeRole::Caption).shade(Shade::Faint).show(ui);
                    })
                    .footer(|ui, _| {
                        below(ui);
                        restore |= clicked(&Button::new(words.drawer_restore.clone()).show(ui));
                    })
                    .show(ui, |ui, stack| {
                        let query = stack.memory::<DrawerSearch>().query.clone();
                        let mut list = List::<DrawerKey>::new(words.drawer_tools.clone())
                            .id(list_id)
                            .select(SelectMode::None)
                            .activate(ActivateOn::Click)
                            .can_drag(|key| match *key {
                                DrawerKey::Tool { group, tool } => !on_toolbar(toolbar, &groups[group].tools[tool].id, grouped_tools_drag),
                                DrawerKey::Group(_) => false,
                            })
                            .drag_payload(|drag: &ListDrag<DrawerKey>| match drag.grabbed {
                                DrawerKey::Tool { group, tool } => Some(ToolDrag::drawer(group, tool)),
                                DrawerKey::Group(_) => None,
                            })
                            .sizing(Sizing::Content);
                        if let Some(key) = reveal.and_then(key_of) {
                            list = list.reveal(key);
                        }
                        if searchable {
                            // A heading has no text of its own: the list keeps it while a tool under it matches.
                            list = list.filter(&query, |key: &DrawerKey| match *key {
                                DrawerKey::Tool { group, tool } => match_texts(&resolved[group][tool], &groups[group].tools[tool]),
                                DrawerKey::Group(_) => Vec::new(),
                            });
                        }
                        let out = list.show_items(ui, &rows, &mut |key, _| match *key {
                            DrawerKey::Group(group) => ListRow::new(groups[group].label.clone()).section_heading(),
                            DrawerKey::Tool { group, tool } => {
                                let command = &resolved[group][tool];
                                let mut row = ListRow::new(command.label.to_string());
                                if let Some(run) = run_of(&groups[group].slots, tool) {
                                    row = row.run(run);
                                }
                                if let Some(icon) = &groups[group].tools[tool].icon {
                                    row = row.icon(icon.clone());
                                } else if let Some(icon) = &command.icon {
                                    row = row.icon(IconSpec::svg(IconId::from(icon.clone())));
                                }
                                if on_toolbar(toolbar, &groups[group].tools[tool].id, grouped_tools_drag) {
                                    row = row.ink(RowInk::Dimmed).description(words.in_toolbar.as_str());
                                }
                                match chord_of(command) {
                                    Some(chord) => row.trailing(chord),
                                    None => row,
                                }
                            }
                        });
                        if let Some(key) = mark.and_then(key_of)
                            && let Some(row) = out.shown.iter().find(|row| row.key == key)
                        {
                            TargetMark::new(row.rect).show_mark(ui);
                        }
                        let tool_of = |key: &DrawerKey| match *key {
                            DrawerKey::Tool { group, tool } => groups[group].tools.get(tool),
                            DrawerKey::Group(_) => None,
                        };
                        for event in out.events {
                            if let CollectionEvent::Activate(key) = event
                                && let Some(tool) = tool_of(&key)
                            {
                                select = Some(tool.id.clone());
                            }
                        }
                    });
            });
            if let Some(ToolDrag::Rail { slot: from, index }) = slot.output.dropped
                && toolbar.iter().find(|candidate| candidate.id == from).and_then(|candidate| candidate.tools.get(index)).is_some()
            {
                edits.push(ToolbarEdit::RemoveFrom { slot: from, index });
            }
        });
        if restore {
            edits.push(ToolbarEdit::Reset);
        }
        let closed = leave || !open || shown.output.closed.is_some();
        ToolbarDrawerOutput { response: shown.output.response, edits, select, closed }
    }
}

/// The edit a drag dropped on the rail at `at` asks for, `None` when it changes nothing: a tool of the rail dropped on its own
/// slot stays where it is, and a drag whose tool is gone is dropped.
pub fn drop_edit<T: Value>(drag: ToolDrag, at: DropAt, groups: &[DrawerGroup<T>], toolbar: &[ToolSlot<T>]) -> Option<ToolbarEdit<T>> {
    match drag {
        ToolDrag::Drawer { group, index } => {
            let tool = groups.get(group)?.tools.get(index)?;
            Some(ToolbarEdit::Add { tool: tool.clone(), at })
        }
        ToolDrag::Rail { slot, index } => {
            toolbar.iter().find(|candidate| candidate.id == slot)?.tools.get(index)?;
            if matches!(at, DropAt::Slot { slot: onto, .. } if onto == slot) {
                return None;
            }
            Some(ToolbarEdit::MoveFrom { slot, index, at })
        }
    }
}

/// The toolbar after `edit`; `defaults` is the toolbar `Reset` goes back to. A slot left with no tool goes.
pub fn apply_toolbar_edit<T: Clone + PartialEq>(slots: &[ToolSlot<T>], edit: &ToolbarEdit<T>, defaults: &[ToolSlot<T>]) -> Vec<ToolSlot<T>> {
    match edit {
        ToolbarEdit::Reset => defaults.to_vec(),
        ToolbarEdit::Add { tool, at } => insert(slots.to_vec(), tool.clone(), *at),
        ToolbarEdit::Move { tool, at } => match slots.iter().flat_map(|slot| &slot.tools).find(|candidate| candidate.id == *tool) {
            Some(found) => place(slots, found.clone(), *at),
            None => slots.to_vec(),
        },
        ToolbarEdit::Remove { tool } => {
            let mut out = slots.to_vec();
            for slot in &mut out {
                slot.tools.retain(|candidate| candidate.id != *tool);
            }
            out.retain(|slot| !slot.tools.is_empty());
            out
        }
        ToolbarEdit::RemoveFrom { slot, index } => {
            let mut out = slots.to_vec();
            let Some(source) = out.iter_mut().find(|candidate| candidate.id == *slot) else { return out };
            if *index < source.tools.len() {
                source.tools.remove(*index);
            }
            out.retain(|slot| !slot.tools.is_empty());
            out
        }
        ToolbarEdit::MoveFrom { slot, index, at } => {
            if matches!(at, DropAt::Slot { slot: target, .. } if !slots.iter().any(|candidate| candidate.id == *target))
                || matches!(at, DropAt::NewSlot { .. }) && slots.iter().any(|candidate| candidate.id == SlotId::MAX)
            {
                return slots.to_vec();
            }
            let mut out = slots.to_vec();
            let Some(source) = out.iter_mut().find(|candidate| candidate.id == *slot) else { return out };
            if *index >= source.tools.len() {
                return out;
            }
            let tool = source.tools.remove(*index);
            let destination = match *at {
                DropAt::Slot { slot: target, index: to } if target == *slot && *index < to => DropAt::Slot { slot: target, index: to - 1 },
                other => other,
            };
            insert(out, tool, destination)
        }
    }
}

fn place<T: Clone + PartialEq>(slots: &[ToolSlot<T>], tool: RailTool<T>, at: DropAt) -> Vec<ToolSlot<T>> {
    if matches!(at, DropAt::Slot { slot, .. } if !slots.iter().any(|candidate| candidate.id == slot))
        || matches!(at, DropAt::NewSlot { .. }) && slots.iter().any(|candidate| candidate.id == SlotId::MAX)
    {
        return slots.to_vec();
    }
    let mut out = slots.to_vec();
    let mut was: Option<(SlotId, usize)> = None;
    for slot in &mut out {
        if let Some(index) = slot.tools.iter().position(|candidate| candidate.id == tool.id) {
            slot.tools.remove(index);
            was = Some((slot.id, index));
        }
    }
    let at = match at {
        DropAt::Slot { slot, index } => {
            let shift = was.is_some_and(|(from, old)| from == slot && old < index);
            DropAt::Slot { slot, index: index.saturating_sub(usize::from(shift)) }
        }
        other => other,
    };
    insert(out, tool, at)
}

fn insert<T: Clone>(mut out: Vec<ToolSlot<T>>, tool: RailTool<T>, at: DropAt) -> Vec<ToolSlot<T>> {
    let original = out.clone();
    match at {
        DropAt::Slot { slot, index } => {
            let Some(target) = out.iter_mut().find(|candidate| candidate.id == slot) else {
                return original;
            };
            let index = index.min(target.tools.len());
            target.tools.insert(index, tool);
        }
        DropAt::NewSlot { before } => {
            let id = match out.iter().map(|slot| slot.id).max() {
                Some(max) => match max.checked_add(1) {
                    Some(id) => id,
                    None => return original,
                },
                None => 0,
            };
            out.insert(before.min(out.len()), ToolSlot::new(id, vec![tool]));
        }
    }
    out.retain(|slot| !slot.tools.is_empty());
    out
}
