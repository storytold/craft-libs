//! The header row (17.1, 17.3 I0): column widths and order, sort presses, resize grips, column moves and the
//! header commands, in one roving group of focusable parts.

use super::ItemsHooks;
use crate::contract::collection::{
    CollectionEvent, CollectionProps, ColumnEvent, ColumnKind, ColumnSort, ColumnSpec, HeaderMenuRequest, ToggleColumn,
};
use crate::contract::reorder::{self, Gap};
use crate::contract::{
    Collection, Common, Component, Cx, DragPreview, DropMark, DropTarget, Focus as _, MeasureCx, PaintUi, PartsCx, Pointer as _, Props as _, Role,
    Show as _, WordSetCx as _,
};
use crate::primitives::{DataWidth, PaintShape};
use crate::primitives::{IconSpec, TextRun};
use crate::runtime::access::items::{HeaderCell, report_header};
use crate::runtime::commands::{self, CommandRef, CommandSet, CommandState};
use crate::runtime::egui::{alloc, child};
use crate::runtime::env::{Env, Label};
use crate::runtime::memory::{MemoryStage, sub};
use crate::runtime::words::HeaderWords;
use crate::style::{ComponentStyle, ResizeLineLook, ScrollBarKind, StateSet, StyleColor, StyleCx, TypeRole, resize_line_look, scroll_bar_look};
use craft_behavior::grid::resolve_columns;
use craft_behavior::{
    Axis, ColumnKey, ColumnWidth, CursorIcon, DropOperation, Flow, HitLevel, HitPriority, IconId, Insets, Intent, Metrics, NavMove, NavShape,
    PartKey, PointerButton, PressPhase, Roving, Sense,
};
use egui::{Align, Context, Id, Pos2, Rect, Response, Ui, UiBuilder, Vec2};

/// The type role of a header label.
const HEADER_ROLE: TypeRole = TypeRole::Label;
const HEADER: PartKey = PartKey::new("header");
const GRIP: PartKey = PartKey::new("header_grip");
/// The narrowest a resized column gets, in control heights.
const MIN_COLUMN: f32 = 2.0;

/// The commands of a column header (17.1): resize and move work from the keyboard through them. The app lists
/// them in the menu it opens for `CollectionEvent::HeaderMenu`, and `commands::fire` routes them to the collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui_macros::CommandSet)]
#[command(prefix = "column", scope = "column")]
#[non_exhaustive]
pub enum ColumnCommands {
    /// Resize the focused column with the arrow keys.
    #[command(label = "Resize Column…")]
    Resize,
    /// Move the focused column toward the left edge.
    #[command(label = "Move Column Left")]
    MoveLeft,
    /// Move the focused column toward the right edge.
    #[command(label = "Move Column Right")]
    MoveRight,
}

impl ColumnCommands {
    /// Every header command, in menu order.
    pub const ALL: [ColumnCommands; 3] = [ColumnCommands::Resize, ColumnCommands::MoveLeft, ColumnCommands::MoveRight];
}

/// A column resize in progress: the column and its width before the drag.
#[derive(Clone, Debug, PartialEq)]
struct Grab {
    key: ColumnKey,
    from: f32,
}

/// The payload of a header drag: the column it moves, in the collection `table`.
#[derive(Clone, Debug)]
struct ColumnDrag {
    table: Id,
    key: ColumnKey,
}

/// Where a column drop lands: the gap among the header cells nearest the pointer, in display order.
struct ColumnGap(Option<Gap>);

impl DropTarget<ColumnDrag> for ColumnGap {
    type Drop = Gap;

    fn accept(&self, _: &ColumnDrag, _: Pos2, op: DropOperation) -> Option<Gap> {
        if matches!(op, DropOperation::Copy) { None } else { self.0 }
    }

    fn mark(&self, drop: &Gap) -> DropMark {
        drop.mark
    }
}

/// The names the toggle cells of one row gave themselves this pass, by column.
#[derive(Clone, Default)]
struct ToggleNames(std::sync::Arc<[(ColumnKey, String, bool)]>);

/// Records the toggle cells of the row `item` with their accessible names (`{row}` filled; empty when the cell gives
/// none) and whether each shows the mixed state, for the collection's cell nodes of the same pass. A row that has none records an empty list.
#[doc(hidden)]
pub fn put_toggle_names(ctx: &Context, item: Id, names: Vec<(ColumnKey, String, bool)>) {
    crate::runtime::egui::memory::put(ctx, sub(item, "toggle_names"), ToggleNames(names.into()));
}

fn recorded_toggle_names(ctx: &Context, item: Id) -> Option<std::sync::Arc<[(ColumnKey, String, bool)]>> {
    crate::runtime::egui::memory::peek_clone::<ToggleNames>(ctx, sub(item, "toggle_names")).map(|ToggleNames(names)| names)
}

/// What the header keeps in session memory under the collection's id.
#[derive(Clone, Debug, Default)]
pub(super) struct HeaderMemory {
    /// Column keys in display order; empty until the user moves one.
    order: Vec<ColumnKey>,
    /// User widths of `Content` and `Fill` columns.
    widths: Vec<(ColumnKey, f32)>,
    /// The column a context menu or command acts on when no header part holds focus, and the frame the menu was
    /// requested in. The menu is gone once no layer that takes keys is open a frame after its request.
    menu: Option<(ColumnKey, u64)>,
    /// The column the keyboard resize command works on.
    resizing: Option<ColumnKey>,
    grab: Option<Grab>,
}

struct Cell {
    spec: usize,
    rect: Rect,
    hash: u64,
}

#[derive(Default)]
pub(super) struct Header {
    /// The header row's height, zero while no column has a header.
    pub(super) height: f32,
    memory: HeaderMemory,
    specs: Vec<ColumnSpec>,
    /// The app's sort, drawn on the header and advanced by a press.
    sort: Option<ColumnSort>,
    /// `CollectionProps.header`: the row is built when some column has a header text.
    on: bool,
    /// The control height, the unit of `ColumnWidth::Controls`.
    unit: f32,
    cells: Vec<Cell>,
    events: Vec<ColumnEvent>,
    menus: Vec<HeaderMenuRequest>,
    /// The narrowest a resized column gets, and the keyboard resize step.
    min: f32,
    step: f32,
    inset: f32,
}

fn hash_of(key: &ColumnKey) -> u64 {
    super::key_hash(key)
}

impl Header {
    /// Step 4: the columns in their display order and the header's session memory.
    pub(super) fn read(&mut self, stage: &mut MemoryStage, id: Id, props: &CollectionProps) {
        self.memory = stage.session::<HeaderMemory>(sub(id, "columns")).clone();
        self.sort = props.sort.clone();
        self.on = props.header;
        let mut specs = props.columns.to_vec();
        let order = &self.memory.order;
        specs.sort_by_key(|spec| order.iter().position(|key| *key == spec.key).unwrap_or(usize::MAX));
        self.specs = specs;
    }

    /// Step 5: the header takes the control height, or the line of its text when that is taller.
    pub(super) fn measure(&mut self, scx: &StyleCx<'_>) {
        let control = scx.control();
        self.unit = control.height;
        // The row shows only when the props ask for it and some column has a header to show (17.3).
        self.height = if !self.on || self.specs.iter().all(|spec| spec.header.as_str().is_empty()) {
            0.0
        } else {
            control.height.max(scx.tokens.typography.get(HEADER_ROLE).line_height)
        };
        self.min = control.height * MIN_COLUMN;
        self.step = control.height;
        self.inset = control.gap;
    }

    fn width_of(&self, spec: &ColumnSpec) -> ColumnWidth {
        match spec.width.in_unit(self.unit) {
            fixed @ ColumnWidth::Fixed(_) => fixed,
            _ => self.memory.widths.iter().find(|(key, _)| *key == spec.key).map_or(spec.width, |(_, width)| ColumnWidth::Fixed(*width)),
        }
    }

    /// The width of each column, in display order, in a row `room` wide.
    fn widths(&self, room: f32) -> Vec<f32> {
        let kinds: Vec<ColumnWidth> = self.specs.iter().map(|spec| self.width_of(spec)).collect();
        let content = vec![self.min; kinds.len()];
        resolve_columns(&kinds, &content, room, 0.0)
    }

    /// The widths for a collection `width` wide whose widest row measured `widest` this pass: the `Content` columns take
    /// what that row leaves beside the fixed and control columns (at most what `width` leaves), split evenly, and share
    /// no room with the `Fill` columns.
    fn widths_measured(&self, width: f32, widest: f32) -> Vec<f32> {
        let kinds: Vec<ColumnWidth> = self.specs.iter().map(|spec| self.width_of(spec)).collect();
        let is_content = |kind: &ColumnWidth| matches!(kind, ColumnWidth::Content);
        let count = kinds.iter().filter(|kind| is_content(kind)).count();
        let content = vec![self.min; kinds.len()];
        if count == 0 {
            return resolve_columns(&kinds, &content, width, 0.0);
        }
        let apart: Vec<ColumnWidth> = kinds.iter().map(|kind| if is_content(kind) { ColumnWidth::Fixed(0.0) } else { *kind }).collect();
        let others: f32 =
            resolve_columns(&apart, &content, width, 0.0).iter().zip(&kinds).filter(|(_, kind)| !is_content(kind)).map(|(wide, _)| *wide).sum();
        let each = (widest - others).min(width - others).max(self.min) / count as f32;
        let sized: Vec<ColumnWidth> = kinds.iter().map(|kind| if is_content(kind) { ColumnWidth::Fixed(each) } else { *kind }).collect();
        resolve_columns(&sized, &content, width, 0.0)
    }

    /// The least width the columns take together: a fixed column its width, every other column its least width.
    pub(super) fn least_width(&self) -> f32 {
        self.specs
            .iter()
            .map(|spec| match self.width_of(spec) {
                ColumnWidth::Fixed(width) => width,
                _ => self.min,
            })
            .sum()
    }

    /// Whether the collection declares columns.
    pub(super) fn has_columns(&self) -> bool {
        !self.specs.is_empty()
    }

    /// The toggle columns in display order.
    pub(super) fn toggle_columns(&self) -> impl Iterator<Item = (ColumnKey, ToggleColumn)> + '_ {
        self.specs.iter().filter_map(|spec| match spec.kind {
            ColumnKind::Toggle(toggle) => Some((spec.key.clone(), toggle)),
            _ => None,
        })
    }

    /// The toggle columns that have a key, with the key.
    pub(super) fn toggle_keys(&self) -> Vec<(ColumnKey, craft_keymap::Chord)> {
        self.toggle_columns().filter_map(|(column, toggle)| Some((column, toggle.key?))).collect()
    }

    /// The name of the cell of toggle column `key` in the row `item` and whether it shows the mixed state: the name the
    /// row's cell gave (`put_toggle_names`), else the column's header text, else its icon's name, resolved in `env`.
    /// `None` when the row recorded its toggle cells and has none in this column: the collection reports no node there.
    pub(super) fn cell_name(&self, ctx: &Context, item: Id, key: &ColumnKey, env: &crate::runtime::env::EnvView<'_>) -> Option<(String, bool)> {
        let recorded = recorded_toggle_names(ctx, item);
        let (given, mixed) = match &recorded {
            Some(names) => names.iter().find(|(held, ..)| held == key).map(|(_, name, mixed)| (name.as_str(), *mixed))?,
            None => ("", false),
        };
        if !given.is_empty() {
            return Some((given.to_owned(), mixed));
        }
        let Some(spec) = self.specs.iter().find(|spec| spec.key == *key) else { return Some((String::new(), mixed)) };
        let header = env.resolve(&spec.header);
        Some((
            match (header.as_str(), &spec.icon) {
                ("", Some(icon)) => icon.name().to_owned(),
                (text, _) => text.to_owned(),
            },
            mixed,
        ))
    }

    /// The key of the column at position `at`.
    pub(super) fn key_at(&self, at: usize) -> Option<&ColumnKey> {
        self.specs.get(at).map(|spec| &spec.key)
    }

    /// The columns with their widths for the item layout.
    pub(super) fn columns(&self, room: f32) -> Vec<(ColumnKey, f32)> {
        self.specs.iter().map(|spec| spec.key.clone()).zip(self.widths(room)).collect()
    }

    /// The columns for a collection sized from its widest row, `width` wide (see `widths_measured`).
    pub(super) fn columns_measured(&self, width: f32, widest: f32) -> Vec<(ColumnKey, f32)> {
        self.specs.iter().map(|spec| spec.key.clone()).zip(self.widths_measured(width, widest)).collect()
    }

    /// Step 7: one focusable part per column, a resize grip at the end edge of each resizable one, and the group.
    pub(super) fn parts<C: Component>(&mut self, pcx: &mut PartsCx<'_, C>) {
        self.cells.clear();
        if self.height <= 0.0 {
            return;
        }
        let area = pcx.content_rect;
        let row = Rect::from_min_size(area.min, Vec2::new(area.width(), self.height));
        let grip = scroll_bar_look(pcx.tokens, ScrollBarKind::Overlay).thickness;
        let rtl = pcx.flow.is_rtl();
        let mut flow = Flow::new(row, rtl);
        let widths = self.widths(area.width());
        let help = pcx.words::<HeaderWords>().tip.clone();
        for (at, (spec, width)) in self.specs.iter().zip(widths).enumerate() {
            let (rect, rest) = flow.take_start(width);
            flow = rest;
            let hash = hash_of(&spec.key);
            let mut sense = Sense::CLICK | Sense::FOCUS;
            if spec.movable {
                sense |= Sense::DRAG;
            }
            pcx.part(HEADER.at(hash), rect)
                .sense(sense)
                .focusable()
                .priority(HitPriority::new(HitLevel::Body, 0))
                .state_layer()
                .tooltip(spec.tip.clone().unwrap_or_else(|| help.clone()));
            if spec.resizable {
                let edge = if rtl {
                    Rect::from_min_size(rect.min, Vec2::new(grip, rect.height()))
                } else {
                    Rect::from_min_max(Pos2::new(rect.max.x - grip, rect.min.y), rect.max)
                };
                pcx.part(GRIP.at(hash), edge)
                    .sense(Sense::DRAG)
                    .priority(HitPriority::new(HitLevel::Body, 1))
                    .cursor(CursorIcon::ResizeHorizontal)
                    .pointer_only("Resize Column… resizes from the keyboard");
            }
            self.cells.push(Cell { spec: at, rect, hash });
        }
        // A keyboard resize reads Forward and Back itself, so the cells do not rove while it lasts.
        if self.memory.resizing.is_none() {
            pcx.roving(Roving::new(NavShape::Flat));
        }
    }

    /// Whether `key` is the sorted column, and if so whether it sorts descending.
    fn sort_of(&self, key: &ColumnKey) -> Option<bool> {
        self.sort.as_ref().filter(|sort| sort.key == *key).map(|sort| sort.descending)
    }

    fn cell_at(&self, key: &ColumnKey) -> Option<usize> {
        self.cells.iter().position(|cell| self.specs[cell.spec].key == *key)
    }

    fn sort(&mut self, key: &ColumnKey) {
        let descending = match &self.sort {
            Some(sort) if sort.key == *key => !sort.descending,
            _ => self.specs.iter().any(|spec| spec.key == *key && spec.first_descending),
        };
        self.events.push(ColumnEvent::Sort { key: key.clone(), descending });
    }

    fn resize(&mut self, key: &ColumnKey, width: f32, shown: f32) {
        let width = width.max(self.min);
        if (width - shown).abs() <= f32::EPSILON {
            return;
        }
        let owned_by_app = self.specs.iter().any(|spec| spec.key == *key && matches!(spec.width, ColumnWidth::Fixed(_) | ColumnWidth::Controls(_)));
        if !owned_by_app {
            match self.memory.widths.iter_mut().find(|(held, _)| held == key) {
                Some((_, held)) => *held = width,
                None => self.memory.widths.push((key.clone(), width)),
            }
        }
        self.events.push(ColumnEvent::Resized { key: key.clone(), width });
    }

    /// Moves `key` to display position `to`: the session order changes and the move is reported.
    fn move_to(&mut self, key: &ColumnKey, to: usize) {
        let mut order: Vec<ColumnKey> = self.specs.iter().map(|spec| spec.key.clone()).collect();
        let Some(from) = order.iter().position(|held| held == key) else { return };
        let to = to.min(order.len() - 1);
        if to == from {
            return;
        }
        let moved = order.remove(from);
        order.insert(to, moved);
        self.memory.order = order;
        self.events.push(ColumnEvent::Moved { key: key.clone(), to });
    }

    /// The gap a column drag at `at` drops in, its `insert` in display order: the edge of the cell under the pointer on
    /// the side of its centre the pointer is on.
    fn gap_at(&self, at: Pos2, rtl: bool) -> Option<Gap> {
        let mut rects: Vec<Rect> = self.cells.iter().map(|cell| cell.rect).collect();
        if rtl {
            rects.reverse();
        }
        let gap = reorder::gap_at(&rects, Axis::X, at)?;
        Some(Gap { insert: if rtl { rects.len() - gap.insert } else { gap.insert }, ..gap })
    }

    /// Whether `command` can act on the column at display position `at`.
    fn allowed(&self, command: ColumnCommands, at: usize, rtl: bool) -> bool {
        let spec = &self.specs[self.cells[at].spec];
        let (toward_start, toward_end) = (at > 0, at + 1 < self.cells.len());
        match command {
            ColumnCommands::Resize => spec.resizable,
            ColumnCommands::MoveLeft => spec.movable && if rtl { toward_end } else { toward_start },
            ColumnCommands::MoveRight => spec.movable && if rtl { toward_start } else { toward_end },
        }
    }

    /// Step 9: presses on the header parts, the header's keys and its commands. Returns whether the header holds
    /// the keys this pass, so the rows take none.
    pub(super) fn update<C: Collection>(&mut self, cx: &mut Cx<'_, C>, id: Id, rtl: bool) -> bool {
        if self.cells.is_empty() {
            return false;
        }
        let ctx = cx.core.ctx.clone();
        commands::register_instance(&ctx, ColumnCommands::SCOPE, id);
        let active =
            cx.active_part().filter(|part| part.key == HEADER).and_then(|part| self.cells.iter().position(|cell| Some(cell.hash) == part.index));
        let keyed = active.is_some() && (cx.core.intents.take(Intent::Activate) || cx.activation().is_some_and(|activation| activation.by_key));
        for at in 0..self.cells.len() {
            let (rect, hash, spec) = (self.cells[at].rect, self.cells[at].hash, self.cells[at].spec);
            let (key, sortable, movable, resizable) = {
                let spec = &self.specs[spec];
                (spec.key.clone(), spec.sortable, spec.movable, spec.resizable)
            };
            let phase = cx.part(HEADER.at(hash)).press.phase;
            let primary = phase.start().is_some_and(|start| start.button == PointerButton::Primary);
            if sortable && ((phase.is_click() && primary) || (keyed && active == Some(at))) {
                self.sort(&key);
            }
            if cx.part(HEADER.at(hash)).hovered && crate::runtime::pointer::frame(&ctx).release == Some(PointerButton::Secondary) {
                self.memory.menu = Some((key.clone(), crate::runtime::egui::pass::root_frame_nr(&ctx)));
                self.menus.push(HeaderMenuRequest { column: key.clone(), at: rect.left_bottom() });
            }
            if movable && primary && matches!(phase, PressPhase::DragStart { .. }) {
                cx.start_drag(ColumnDrag { table: id, key: key.clone() }, DragPreview::None);
            }
            if resizable {
                match cx.part(GRIP.at(hash)).press.phase {
                    PressPhase::DragStart { start, pos, .. } | PressPhase::Drag { start, pos, .. } => {
                        let from = match &self.memory.grab {
                            Some(Grab { key: held, from }) if *held == key => *from,
                            _ => {
                                self.memory.grab = Some(Grab { key: key.clone(), from: rect.width() });
                                rect.width()
                            }
                        };
                        let travel = if rtl { start.origin.x - pos.x } else { pos.x - start.origin.x };
                        self.resize(&key, from + travel, rect.width());
                    }
                    PressPhase::Release { .. } => {
                        if matches!(&self.memory.grab, Some(Grab { key: held, .. }) if *held == key) {
                            self.memory.grab = None;
                        }
                    }
                    PressPhase::Cancel { .. } => {
                        if let Some(Grab { key: held, from }) = self.memory.grab.take()
                            && held == key
                        {
                            self.resize(&key, from, rect.width());
                        }
                    }
                    _ => {}
                }
            }
            if active == Some(at) && cx.core.intents.take(Intent::ContextMenu) {
                self.memory.menu = Some((key.clone(), crate::runtime::egui::pass::root_frame_nr(&ctx)));
                self.menus.push(HeaderMenuRequest { column: key, at: rect.left_bottom() });
            }
        }
        self.column_drop(cx, id, rtl);
        self.keyboard_resize(cx);
        self.commands(cx, id, active, rtl);
        active.is_some() || self.memory.resizing.is_some()
    }

    /// The keyboard resize the `Resize` command starts: Forward widens, Back narrows, Enter or Escape ends it.
    /// The drop target of this collection's own header drag: the gap mark while the drag is over the header, the move
    /// on release.
    fn column_drop<C: Collection>(&mut self, cx: &mut Cx<'_, C>, id: Id, rtl: bool) {
        let Some((drag, at)) = cx.dragging::<ColumnDrag>().filter(|(drag, _)| drag.table == id) else { return };
        let target = ColumnGap(self.gap_at(at, rtl));
        if let Some(gap) = cx.drop_target::<ColumnDrag, _>(&target)
            && let Some(from) = self.cells.iter().position(|cell| self.specs[cell.spec].key == drag.key)
        {
            self.move_to(&drag.key, reorder::move_to(from, gap.insert));
        }
    }

    fn keyboard_resize<C: Collection>(&mut self, cx: &mut Cx<'_, C>) {
        let Some(key) = self.memory.resizing.clone() else { return };
        let Some(at) = self.cell_at(&key).filter(|_| cx.is_focused()) else {
            self.memory.resizing = None;
            return;
        };
        let shown = self.cells[at].rect.width();
        while let Some((step, _)) = cx.core.intents.take_move() {
            match step {
                NavMove::Forward => self.resize(&key, shown + self.step, shown),
                NavMove::Back => self.resize(&key, shown - self.step, shown),
                _ => {}
            }
        }
        if cx.core.intents.take(Intent::Submit) || cx.core.intents.take(Intent::Cancel) {
            self.memory.resizing = None;
        }
    }

    fn commands<C: Collection>(&mut self, cx: &mut Cx<'_, C>, id: Id, active: Option<usize>, rtl: bool) {
        let ctx = cx.core.ctx.clone();
        if let Some((_, asked)) = &self.memory.menu
            && crate::runtime::egui::pass::root_frame_nr(&ctx) > *asked + 1
            && !crate::runtime::overlay::any_open(&ctx, crate::runtime::overlay::OpenFilter::Keys)
        {
            self.memory.menu = None;
        }
        let target = active.or_else(|| self.memory.menu.as_ref().and_then(|(key, _)| self.cell_at(key)));
        let chain = cx.core.env.command_chain().pushed(ColumnCommands::SCOPE, id);
        for command in ColumnCommands::ALL {
            let enabled = target.is_some_and(|at| self.allowed(command, at, rtl));
            commands::publish_for(&ctx, &chain, &CommandRef::from(command), CommandState::new().enabled(enabled));
        }
        while let Some((command, _)) = commands::take_for_instance(&ctx, Some(id), false, ColumnCommands::from_ref) {
            let Some(at) = target.filter(|&at| self.allowed(command, at, rtl)) else { continue };
            self.memory.menu = None;
            let key = self.specs[self.cells[at].spec].key.clone();
            match command {
                ColumnCommands::Resize => self.memory.resizing = Some(key),
                ColumnCommands::MoveLeft => self.move_to(&key, if rtl { at + 1 } else { at - 1 }),
                ColumnCommands::MoveRight => self.move_to(&key, if rtl { at - 1 } else { at + 1 }),
            }
        }
    }

    /// Step 14, I0: the header labels, with the sort mark on the sorted column.
    pub(super) fn paint<C: Collection>(&self, ui: &mut Ui, cx: &Cx<'_, C>, env: &Env, id: Id, rtl: bool) {
        let ctx = ui.ctx().clone();
        for cell in &self.cells {
            let spec = &self.specs[cell.spec];
            let text = cx.core.env.resolve(&spec.header).as_str().to_owned();
            let sort = self.sort_of(&spec.key);
            let builder = UiBuilder::new().max_rect(cell.rect).layout(child::column_layout(rtl)).scope_id(sub(id, ("header_cell", cell.hash)));
            let mut cell_ui = alloc::child(ui, builder);
            cell_ui.set_clip_rect(cell.rect.intersect(ui.clip_rect()));
            let installed = Env::install_for(&cell_ui, env.derive_local(|scoped| scoped.arena = None));
            let edge = spec
                .resizable
                .then(|| HeaderEdge { state: cx.core.part_state(GRIP.at(cell.hash)), keyed: self.memory.resizing.as_ref() == Some(&spec.key) });
            let mut label = HeaderText::new(Label::from(text), self.inset, sort).id(sub(id, ("header_text", cell.hash)));
            label.edge = edge;
            label.show_at(&mut cell_ui, cell.rect);
            Env::restore(&ctx, installed);
        }
    }

    /// Step 16: the header cells as `ColumnHeader` sub-nodes.
    pub(super) fn report<C: Collection>(&self, ctx: &Context, id: Id, cx: &Cx<'_, C>) {
        let focused = self.focused(cx);
        let names: Vec<String> = self.cells.iter().map(|cell| cx.core.env.resolve(&self.specs[cell.spec].header).as_str().to_owned()).collect();
        let cells: Vec<HeaderCell<'_>> = self
            .cells
            .iter()
            .zip(&names)
            .enumerate()
            .map(|(at, (cell, name))| HeaderCell {
                part: HEADER.at(cell.hash),
                name,
                rect: cell.rect,
                clickable: self.specs[cell.spec].sortable,
                active: focused == Some(at),
                sort: self.sort_of(&self.specs[cell.spec].key),
            })
            .collect();
        report_header(ctx, id, cx.core.resolved.enabled, &cells);
    }

    /// The display position of the header part that holds focus.
    pub(super) fn focused<C: Collection>(&self, cx: &Cx<'_, C>) -> Option<usize> {
        cx.active_part().filter(|part| part.key == HEADER).and_then(|part| self.cells.iter().position(|cell| Some(cell.hash) == part.index))
    }

    pub(super) fn persist(&self) -> HeaderMemory {
        self.memory.clone()
    }
}

impl<C: Collection> ItemsHooks<'_, '_, C> {
    /// Step 9 for the header: its events join the collection's, in drawing order.
    pub(super) fn header_step(&mut self, cx: &mut Cx<'_, C>) -> bool {
        let (id, rtl) = (cx.core.id, cx.core.flow.is_rtl());
        let held = self.header.update(cx, id, rtl);
        for event in self.header.events.drain(..) {
            self.events.push(CollectionEvent::Column(event));
        }
        for menu in self.header.menus.drain(..) {
            self.events.push(CollectionEvent::HeaderMenu(menu));
        }
        held
    }
}

/// The style of a header label: the sort mark's size and its gap after the text, the label's ink (the sorted
/// column's reads stronger) and the column edge's line.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
struct HeaderTextStyle {
    #[style(metrics)]
    metrics: Metrics,
    mark: f32,
    gap: f32,
    ink: StyleColor,
    sorted: StyleColor,
    edge: ResizeLineLook,
}

/// The resize grip at a column's end edge, as the header cell paints it.
#[derive(Clone, Copy)]
struct HeaderEdge {
    /// The grip part's state: its line shows under the pointer and while dragged.
    state: StateSet,
    /// The keyboard resize works on the column.
    keyed: bool,
}

/// A header cell's label.
#[derive(crate::contract::Component)]
#[component(sample = HeaderText::sample)]
struct HeaderText {
    #[common]
    common: Common,
    inset: f32,
    /// Whether the column is the sorted one, and if so whether it sorts descending.
    sort: Option<bool>,
    /// The column's resize grip, for a resizable column.
    edge: Option<HeaderEdge>,
    /// Whether the label is its own AccessKit node. The header row's label is not: its `ColumnHeader` sub-node carries
    /// the name.
    node: bool,
}

impl HeaderText {
    #[cfg(feature = "testing")]
    fn sample() -> Self {
        HeaderText { node: true, ..HeaderText::new(Label::from("Name"), 0.0, Some(false)) }
    }

    fn new(text: Label, inset: f32, sort: Option<bool>) -> Self {
        HeaderText { common: Common::new(), inset, sort, edge: None, node: false }.label(text)
    }

    /// The label on one line, cut with an ellipsis when its column is narrower than the text.
    fn run<'t>(&self, text: &'t str) -> TextRun<'t> {
        TextRun::new(text).role(HEADER_ROLE).elide()
    }

    /// The line of the column's end edge, inside the cell `rect`: the grip's line under the pointer and while dragged,
    /// the focus line while the keyboard resizes the column.
    fn paint_edge(&self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, rtl: bool) {
        let Some(edge) = self.edge else { return };
        let look = &cx.style.edge;
        let (color, width) =
            if edge.keyed { (look.focus, look.focus_width) } else { (edge.state.resolve(&look.edge), edge.state.resolve(&look.width)) };
        if color.egui().a() == 0 {
            return;
        }
        let rect = cx.rect;
        let x = if rtl { rect.left() + width / 2.0 } else { rect.right() - width / 2.0 };
        let ends = vec![Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())];
        ui.draw().stroke(&PaintShape::Path(ends), DataWidth::points(width), color);
    }
}

impl Component for HeaderText {
    const ROLE: Role = Role::LABEL;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = HeaderTextStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: HeaderTextStyleKey) -> HeaderTextStyle {
        let text = &cx.tokens.color.text;
        HeaderTextStyle {
            metrics: Metrics::new(),
            mark: cx.tokens.size.icon.sm,
            gap: cx.tokens.space.gap.stack,
            ink: cx.color(text.muted),
            sorted: cx.color(text.normal),
            edge: resize_line_look(cx.tokens),
        }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn access(&self, access: &mut crate::contract::Access, _: &Cx<'_, Self>) {
        if !self.node {
            access.role(Role::None);
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let line = cx.layout(&self.run(cx.resolved.label)).size();
        let mark = if self.sort.is_some() { cx.style.mark + cx.style.gap } else { 0.0 };
        Vec2::new(line.x + mark + 2.0 * self.inset, line.y)
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let rtl = cx.core.env.direction().is_rtl();
        let inner = Flow::new(Flow::new(cx.rect, rtl).inset(Insets::horizontal(self.inset)), rtl);
        let ink = if self.sort.is_some() { cx.style.sorted } else { cx.style.ink };
        let run = self.run(cx.core.resolved.label).color(ink);
        match self.sort {
            Some(descending) => {
                // The mark follows the label's end; a label too long for the cell is cut to leave the mark its room.
                let (side, gap) = (cx.style.mark, cx.style.gap);
                let room = (inner.rect().width() - side - gap).max(0.0);
                let label = ui.draw().layout(&run, room).size().x.min(room);
                let (text, rest) = inner.take_start(label);
                ui.draw().text(text, &run);
                let slot = Flow::new(rest.span(gap, side), rtl);
                let id = IconId::new(if descending { "sort_descending" } else { "sort_ascending" });
                ui.draw().icon(slot.align(Vec2::splat(side), Align::Min, Align::Center), &IconSpec::svg(id));
            }
            None => {
                ui.draw().text(inner.rect(), &run);
            }
        }
        self.paint_edge(ui, cx, rtl);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.core.response.clone()
    }
}
