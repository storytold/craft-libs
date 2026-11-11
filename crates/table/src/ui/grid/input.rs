//! The grid's keys and pointer gestures: the in-cell editor modes, cell navigation the collection does not do,
//! header selection, edge resizing, the fill handle and the range border.

use std::sync::Arc;

use craft_behavior::{EditEvent, EditorInput, GestureId, ItemLayout as _, Mixed, PointerButton, PressCancel, PressPhase, PressState};
use craft_keymap::{Chord, Key, Mods};
use craft_ui::contract::{Cx, Focus as _, Pointer as _, Remember as _};
use craft_ui::runtime::commands::{ClipContent, Commands as _};
use craft_ui::runtime::pending::Pending;
use egui::{Pos2, Rect};

use super::editor::{EditState, Mode};
use super::paint::{BORDER, COLUMN, COLUMN_EDGE, CORNER, FILL, ROW, ROW_EDGE};
use super::{
    CellAdvance, CellAt, CellDirection, CellEdit, CellGrid, CellGridWords, CellRange, CellSelection, CellSpace, GridEdit, GridMenuTarget, Positions,
    edges,
};

/// The arrows with the direction each takes in a left-to-right sheet.
const ARROWS: [(Key, CellDirection); 4] =
    [(Key::Up, CellDirection::Up), (Key::Down, CellDirection::Down), (Key::Left, CellDirection::Left), (Key::Right, CellDirection::Right)];

/// Enter and Tab (Shift: back) advance the active cell.
const ADVANCES: [(Chord, CellAdvance, bool); 4] = [
    (Chord::plain(Key::Enter), CellAdvance::Enter, false),
    (Chord::new(Mods::SHIFT, Key::Enter), CellAdvance::Enter, true),
    (Chord::plain(Key::Tab), CellAdvance::Tab, false),
    (Chord::new(Mods::SHIFT, Key::Tab), CellAdvance::Tab, true),
];

/// The modifiers `cmd` and `shift` name.
const fn mods(cmd: bool, shift: bool) -> Mods {
    Mods { cmd, ctrl: false, alt: false, shift }
}

const ESCAPE: Chord = Chord::plain(Key::Escape);
const F2: Chord = Chord::plain(Key::F2);
const HOME: Chord = Chord::plain(Key::Home);
const CMD_HOME: Chord = Chord::new(Mods::CMD, Key::Home);
const END: Chord = Chord::plain(Key::End);
const CMD_END: Chord = Chord::new(Mods::CMD, Key::End);
const NEXT_SHEET: Chord = Chord::new(Mods::CMD, Key::PageDown);
const PREVIOUS_SHEET: Chord = Chord::new(Mods::CMD, Key::PageUp);
const COLUMN_KEY: Chord = Chord::new(Mods::CMD, Key::Space);
const ROW_KEY: Chord = Chord::new(Mods::SHIFT, Key::Space);

/// The ranges a drag previews, read by the paint.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Preview {
    fill: Option<CellRange<u32>>,
    moving: Option<CellRange<u32>>,
}

impl Preview {
    pub(super) fn range(&self) -> Option<CellRange<u32>> {
        self.fill.clone().or_else(|| self.moving.clone())
    }
}

/// The gesture a drag on one of the grid's parts runs.
#[derive(Clone, Debug, PartialEq)]
enum Grab {
    /// Selecting whole columns (`rows` false) or rows from `anchor`.
    Lines { rows: bool, anchor: u32 },
    /// Resizing the line `line` of a column (`rows` false) or row axis from `from` points at 100 %.
    Resize { rows: bool, line: u32, from: f32, id: GestureId, last: f32 },
    /// Growing `source` by the fill handle; `menu` for a secondary drag.
    Fill { source: CellRange<u32>, last: CellRange<u32>, menu: bool, id: GestureId },
    /// Moving `source` by its border, grabbed `offset` (rows, columns) from its top-left cell.
    Move { source: CellRange<u32>, offset: (i64, i64), last: CellAt<u32>, id: GestureId },
}

/// The range the fill handle grows `source` to when dragged to `target`: along the axis the pointer overshoots most.
fn fill_range(source: &CellRange<u32>, target: CellAt<u32>) -> CellRange<u32> {
    let (top, bottom, left, right) = edges(source);
    let beyond = |value: u32, low: u32, high: u32| if value > high { value - high } else { low.saturating_sub(value) };
    let (rows, columns) = (beyond(target.row, top, bottom), beyond(target.column, left, right));
    if rows == 0 && columns == 0 {
        return source.clone();
    }
    if rows >= columns {
        let (first, last) = if target.row > bottom { (top, target.row) } else { (target.row, bottom) };
        CellRange::cells(CellAt::new(first, left), CellAt::new(last, right))
    } else {
        let (first, last) = if target.column > right { (left, target.column) } else { (target.column, right) };
        CellRange::cells(CellAt::new(top, first), CellAt::new(bottom, last))
    }
}

/// One arrow press and the state it runs in.
struct ArrowStep {
    direction: CellDirection,
    shift: bool,
    jump: bool,
    armed: bool,
    rtl: bool,
}

/// The grid's claimed keys that were pressed in a pass.
struct ClaimedKeys {
    escape: bool,
    advance: Option<(CellAdvance, bool)>,
    f2: bool,
    /// Each arrow: its direction, whether Shift was held and whether Cmd was held.
    arrows: Vec<(CellDirection, bool, bool)>,
    home: bool,
    end: bool,
    ready: ReadyKeys,
}

/// The claimed keys that move the selection by sheet, column or row in Ready mode.
#[derive(Clone, Copy)]
struct ReadyKeys {
    cmd_home: bool,
    cmd_end: bool,
    next_sheet: bool,
    previous_sheet: bool,
    column_key: bool,
    row_key: bool,
}

impl ClaimedKeys {
    fn take(cx: &mut Cx<'_, CellGrid<'_>>) -> Self {
        let escape = cx.take_claimed(ESCAPE);
        let advance = ADVANCES.iter().find_map(|&(chord, along, back)| cx.take_claimed(chord).then_some((along, back)));
        let f2 = cx.take_claimed(F2);
        let mut arrows = Vec::new();
        for shift in [false, true] {
            for (key, direction) in ARROWS {
                if cx.take_claimed(Chord::new(mods(false, shift), key)) {
                    arrows.push((direction, shift, false));
                }
                if cx.take_claimed(Chord::new(mods(true, shift), key)) {
                    arrows.push((direction, shift, true));
                }
            }
        }
        ClaimedKeys {
            escape,
            advance,
            f2,
            arrows,
            home: cx.take_claimed(HOME),
            end: cx.take_claimed(END),
            ready: ReadyKeys {
                cmd_home: cx.take_claimed(CMD_HOME),
                cmd_end: cx.take_claimed(CMD_END),
                next_sheet: cx.take_claimed(NEXT_SHEET),
                previous_sheet: cx.take_claimed(PREVIOUS_SHEET),
                column_key: cx.take_claimed(COLUMN_KEY),
                row_key: cx.take_claimed(ROW_KEY),
            },
        }
    }

    /// Whether a key other than an arrow or End was pressed, or an editor input arrived.
    fn ends_end_mode(&self, typed: bool) -> bool {
        self.escape
            || self.advance.is_some()
            || self.f2
            || self.home
            || self.ready.cmd_home
            || self.ready.cmd_end
            || self.ready.next_sheet
            || self.ready.previous_sheet
            || self.ready.column_key
            || self.ready.row_key
            || typed
    }
}

impl CellGrid<'_> {
    /// The cell space the rules run over: the sheet's size, merged cells, hidden lines and locked cells.
    fn space<'s>(
        &'s self,
        order: &'s Positions,
        hidden_rows: &'s dyn Fn(u32) -> bool,
        hidden_columns: &'s dyn Fn(u32) -> bool,
    ) -> CellSpace<'s, u32> {
        use craft_ui::contract::Collection as _;
        self.cell_space(CellSpace::new(order, self.model.columns().count()).hidden_rows(hidden_rows).hidden_columns(hidden_columns))
    }

    /// One arrow press: it moves the pointed reference of a formula, the caret of an edit, or the selection.
    fn step_arrow(&self, state: &mut EditState, selection: &mut CellSelection<u32>, space: &CellSpace<'_, u32>, arrow: ArrowStep) {
        let ArrowStep { direction, shift, jump, armed, rtl } = arrow;
        let model = self.model;
        let pointing = state.pointing();
        let direction = match (rtl, direction) {
            (true, CellDirection::Left) => CellDirection::Right,
            (true, CellDirection::Right) => CellDirection::Left,
            (_, direction) => direction,
        };
        if pointing {
            let at = state.point.as_ref().map(|point| CellAt::new(point.range.last, point.range.lead_column)).or(state.cell);
            let next = match at {
                Some(at) if jump || armed => Some(model.data_edge(at, direction)),
                Some(at) => space.neighbour(at, direction),
                None => None,
            };
            if let Some(next) = next {
                state.point_to(next, shift);
            }
        } else if state.mode == Mode::Edit {
            let step = match direction {
                CellDirection::Left => Some(craft_behavior::NavMove::Back),
                CellDirection::Right => Some(craft_behavior::NavMove::Forward),
                _ => None,
            };
            if let Some(step) = step {
                let by = if jump { craft_behavior::Granularity::Word } else { craft_behavior::Granularity::Unit };
                state.draft.apply(EditorInput::Move(step, craft_behavior::MoveMods::new(shift, false, by)));
            }
        } else if state.mode == Mode::Ready
            && (jump || armed)
            && let Some(active) = selection.active().copied()
        {
            let to = model.data_edge(active, direction);
            if shift {
                selection.extend_to(to, space);
            } else {
                selection.select(to, space);
            }
        }
    }

    /// The Ready-mode keys that jump the selection to the start or end of the data, change the sheet or take a whole
    /// column or row.
    fn ready_keys(
        &self,
        selection: &mut CellSelection<u32>,
        space: &CellSpace<'_, u32>,
        edits: &mut Vec<GridEdit>,
        keys: ReadyKeys,
        frozen_columns: u32,
    ) {
        let model = self.model;
        if keys.cmd_home {
            selection.select(CellAt::new(self.sheet_layout().first_navigable() as u32, frozen_columns), space);
        }
        if keys.cmd_end {
            selection.select(model.last_used(), space);
        }
        if keys.next_sheet {
            edits.push(GridEdit::NextSheet);
        }
        if keys.previous_sheet {
            edits.push(GridEdit::PreviousSheet);
        }
        if let Some(active) = selection.active().copied() {
            if keys.column_key
                && let Some(range) = space.whole_columns(active.column, active.column)
            {
                selection.select_range(range, space);
            }
            if keys.row_key {
                selection.select_range(space.whole_rows(active.row, active.row), space);
            }
        }
    }

    fn write_selection(&mut self, cx: &mut Cx<'_, Self>, before: &CellSelection<u32>, after: CellSelection<u32>, label: &str) {
        if &after != before {
            let event = EditEvent::set(Mixed::Same(before.clone()), after, Arc::from(label), None);
            let _ = self.selection.reborrow().write(cx, &event);
        }
    }

    fn next_gesture(cx: &mut Cx<'_, Self>) -> GestureId {
        {
            use craft_ui::runtime::commands::CommandAccess as _;
            craft_ui::runtime::edit::next_gesture_id(cx.command_ctx())
        }
    }

    /// Keys and pointer gestures of one pass.
    pub(super) fn run_input(&mut self, cx: &mut Cx<'_, Self>) {
        let before = cx.collection_selection().cloned().unwrap_or_else(|| self.snapshot.clone());
        let mut selection = before.clone();
        let words = cx.env.words::<CellGridWords>();
        let select_label = words.select.as_str().to_owned();
        let order = Positions::new(self.model.rows().count());
        let (rows, columns) = (self.model.rows(), self.model.columns());
        let hidden_rows = |row: u32| !rows.shown(row);
        let hidden_columns = |column: u32| !columns.shown(column);
        let rtl = cx.flow.is_rtl();
        let lead = selection.active().copied();
        let frozen = self.model.frozen();
        let mut state = cx.memory::<EditState>().clone();
        let mut grab = cx.memory::<Option<Grab>>().clone();
        let mut preview = Preview::default();

        let keys = ClaimedKeys::take(cx);
        let inputs: Vec<EditorInput> = cx.editor_input().collect();
        // Any key other than an arrow or End, and any press, ends the mode.
        let other_key = keys.ends_end_mode(!inputs.is_empty());
        let ClaimedKeys { escape, advance, f2, arrows, home, end, ready } = keys;
        // End mode (Excel): a bare End toggles it in Ready mode and while pointing at a reference. The key stage hands
        // the bare End to a grid that declares `EndKey::EndMode` as a claimed chord; in an edit it moves the caret.
        let end_mode_allowed = state.mode == Mode::Ready || state.pointing();
        let end_mode_key = end && end_mode_allowed;
        let armed = end_mode_allowed && state.end_mode;
        state.end_mode =
            if end_mode_key { !armed } else { armed && arrows.is_empty() && !other_key && !matches!(cx.press().phase, PressPhase::Down { .. }) };

        let mut deferred = Vec::new();
        let mut edits = Vec::new();
        let mut committed: Option<CellEdit> = None;
        {
            let space = self.space(&order, &hidden_rows, &hidden_columns);
            let model = self.model;
            let commit = |state: &mut EditState, committed: &mut Option<CellEdit>| {
                if let Some(cell) = state.cell {
                    *committed = Some(CellEdit::new(cell, state.draft.text()));
                }
                state.close();
            };
            if state.mode != Mode::Ready && lead.is_some_and(|lead| Some(lead) != state.cell) {
                commit(&mut state, &mut committed);
            }
            if state.mode != Mode::Ready && escape {
                state.close();
            } else if state.mode == Mode::Ready && escape && !self.copy_mark.is_empty() {
                edits.push(GridEdit::ClearCopyMark);
            }
            if let Some((along, back)) = advance {
                if state.mode != Mode::Ready {
                    commit(&mut state, &mut committed);
                }
                if state.mode == Mode::Ready && !self.copy_mark.is_empty() && !back && along == CellAdvance::Enter {
                    edits.push(GridEdit::PasteAtActive);
                } else {
                    selection.advance(along, back, &space);
                }
            }
            if f2
                && state.mode == Mode::Ready
                && let Some(cell) = lead
            {
                state.open(cell, Mode::Edit, &model.cell_text(cell));
                edits.push(GridEdit::BeginEdit { typed: None });
            }
            for &(direction, shift, jump) in &arrows {
                self.step_arrow(&mut state, &mut selection, &space, ArrowStep { direction, shift, jump, armed, rtl });
            }
            if state.mode != Mode::Ready && end && !state.pointing() {
                state.draft.apply(EditorInput::Move(
                    craft_behavior::NavMove::End,
                    craft_behavior::MoveMods::new(false, false, craft_behavior::Granularity::Line),
                ));
            }
            if state.mode == Mode::Edit && home {
                state.draft.apply(EditorInput::Move(craft_behavior::NavMove::Start, craft_behavior::MoveMods::PLAIN));
            } else if state.mode == Mode::Ready {
                if home && armed {
                    selection.select(model.last_used(), &space);
                } else if home && let Some(active) = selection.active().copied() {
                    selection.select(CellAt::new(active.row, 0), &space);
                }
                self.ready_keys(&mut selection, &space, &mut edits, ready, frozen.1);
            }
            for input in inputs {
                match (state.mode, input) {
                    (Mode::Ready, EditorInput::Delete { forward: true, .. }) => edits.push(GridEdit::Clear),
                    (Mode::Ready, EditorInput::Delete { forward: false, .. }) => {
                        if let Some(cell) = lead {
                            edits.push(GridEdit::ClearAndEdit);
                            state.open(cell, Mode::Enter, "");
                        }
                    }
                    (Mode::Ready, EditorInput::Edit(craft_behavior::EditCommand::Copy)) => edits.push(GridEdit::Copy),
                    (Mode::Ready, EditorInput::Edit(craft_behavior::EditCommand::Cut)) => edits.push(GridEdit::Cut),
                    (Mode::Ready, EditorInput::Edit(craft_behavior::EditCommand::Paste)) => deferred.push(()),
                    (Mode::Ready, EditorInput::Edit(craft_behavior::EditCommand::SelectAll)) => {
                        if let Some(all) = space.all() {
                            selection.select_range(all, &space);
                        }
                    }
                    (Mode::Ready, input @ (EditorInput::Text(_) | EditorInput::Preedit { .. })) => {
                        if let Some(cell) = lead {
                            let typed = match &input {
                                EditorInput::Text(text) => Some(text.to_string()),
                                _ => None,
                            };
                            state.open(cell, Mode::Enter, "");
                            state.draft.apply(input);
                            edits.push(GridEdit::BeginEdit { typed });
                        }
                    }
                    (Mode::Ready, _) | (_, EditorInput::Newline | EditorInput::Tab | EditorInput::Cancel) => {}
                    (_, EditorInput::Move(..)) if state.pointing() => {}
                    (_, input) => {
                        state.point = None;
                        state.draft.apply(input);
                    }
                }
            }
        }
        self.pass.edits.extend(edits);
        if committed.is_some() {
            self.pass.committed = committed;
        }
        if !deferred.is_empty()
            && let Pending::Ready(ClipContent { text: Some(text), .. }) = cx.clipboard().paste()
        {
            self.pass.edits.push(GridEdit::Paste(text.to_string()));
        }
        // A key that moves the active cell moves the collection's lead there and scrolls it into view (Excel).
        if selection.active() != before.active() {
            self.reveal = selection.active().map(|cell| cell.row);
        }
        let (pointer_edits, swapped) = self.run_pointer(cx, &mut selection, &mut grab, &mut preview, &order, &hidden_rows, &hidden_columns);
        self.pass.edits.extend(pointer_edits);
        self.pass.fill_swapped |= swapped;
        self.claim_keys(cx, &state, grab.is_some());
        if state.mode != Mode::Ready {
            self.pass.draft = Some(state.draft.text().to_owned());
        }
        *cx.memory::<EditState>() = state;
        *cx.memory::<Option<Grab>>() = grab;
        *cx.memory::<Preview>() = preview;
        self.write_selection(cx, &before, selection, &select_label);
    }

    /// The chords the grid takes from the key stage this pass.
    fn claim_keys(&self, cx: &mut Cx<'_, Self>, state: &EditState, grabbing: bool) {
        for chord in [F2, HOME, CMD_HOME, CMD_END, NEXT_SHEET, PREVIOUS_SHEET, COLUMN_KEY, ROW_KEY] {
            cx.claim(chord);
        }
        for (chord, ..) in ADVANCES {
            cx.claim(chord);
        }
        if state.mode != Mode::Ready || grabbing || !self.copy_mark.is_empty() || state.end_mode {
            cx.claim(ESCAPE);
        }
        for (key, _) in ARROWS {
            cx.claim(Chord::new(Mods::CMD, key));
            cx.claim(Chord::new(mods(true, true), key));
        }
        let horizontal = state.mode == Mode::Edit;
        for (key, direction) in ARROWS {
            let takes = state.pointing()
                || (state.end_mode && state.mode == Mode::Ready)
                || horizontal && matches!(direction, CellDirection::Left | CellDirection::Right);
            if takes {
                cx.claim(Chord::plain(key));
                cx.claim(Chord::new(Mods::SHIFT, key));
            }
        }
    }

    /// Header, edge, corner, fill and border presses.
    #[allow(clippy::too_many_arguments)]
    fn run_pointer(
        &self,
        cx: &mut Cx<'_, Self>,
        selection: &mut CellSelection<u32>,
        grab: &mut Option<Grab>,
        preview: &mut Preview,
        order: &Positions,
        hidden_rows: &dyn Fn(u32) -> bool,
        hidden_columns: &dyn Fn(u32) -> bool,
    ) -> (Vec<GridEdit>, bool) {
        let mut edits = Vec::new();
        let mut swapped = false;
        let Some(frame) = self.frame.clone() else { return (edits, swapped) };
        let geometry = &frame.geometry;
        let words = cx.env.words::<CellGridWords>();
        let zoom = geometry.zoom();
        let rtl = cx.flow.is_rtl();
        let logical = |pos: Pos2| geometry.mirror_pos(pos);
        let model = self.model;
        let range_label = |text: &str| Arc::<str>::from(text);
        let space = self.space(order, hidden_rows, hidden_columns);

        if self.headings {
            let result = cx.part(CORNER).clone();
            if result.press.phase.is_click()
                && let Some(all) = space.all()
            {
                selection.select_range(all, &space);
            }
            for &column in &frame.columns {
                let press = cx.part(COLUMN.at(u64::from(column))).press;
                Self::line_press(&mut edits, selection, grab, &space, geometry, press, (false, column), Self::column_header(geometry, column));
                let edge = cx.part(COLUMN_EDGE.at(u64::from(column))).press;
                let size = model.columns().size(column) as f32;
                Self::edge_press(cx, &mut edits, grab, &range_label(words.resize_column.as_str()), (false, column), size, zoom, rtl, edge);
                if edge.double_pressed() {
                    let next_hidden = column + 1 < model.columns().count() && !model.columns().shown(column + 1);
                    edits.push(if next_hidden { GridEdit::UnhideColumn { column: column + 1 } } else { GridEdit::AutoFitColumn { column } });
                }
            }
            for &row in &frame.rows {
                let press = cx.part(ROW.at(u64::from(row))).press;
                Self::line_press(&mut edits, selection, grab, &space, geometry, press, (true, row), Self::row_header(geometry, row));
                let edge = cx.part(ROW_EDGE.at(u64::from(row))).press;
                let size = model.rows().size(row) as f32;
                Self::edge_press(cx, &mut edits, grab, &range_label(words.resize_row.as_str()), (true, row), size, zoom, rtl, edge);
                if edge.double_pressed() {
                    let next_hidden = row + 1 < model.rows().count() && !model.rows().shown(row + 1);
                    edits.push(if next_hidden { GridEdit::UnhideRow { row: row + 1 } } else { GridEdit::AutoFitRow { row } });
                }
            }
        }

        let current = selection.current().cloned();
        let fill = cx.part_if_declared(FILL).map_or(PressState::IDLE, |result| result.press);
        match (fill.phase, current.clone()) {
            (PressPhase::DragStart { start, .. }, Some(source)) => {
                let id = Self::next_gesture(cx);
                *grab = Some(Grab::Fill { last: source.clone(), menu: start.button == PointerButton::Secondary, source: source.clone(), id });
                let label = range_label(words.fill.as_str());
                edits.push(GridEdit::Fill { range: EditEvent::begin(id, Mixed::Same(source), label, None) });
            }
            (PressPhase::Drag { pos, .. }, _) => {
                if let Some(Grab::Fill { source, last, id, .. }) = grab {
                    let range = fill_range(source, geometry.cell_at(logical(pos)));
                    *last = range.clone();
                    edits.push(GridEdit::Fill { range: EditEvent::update(*id, range) });
                }
            }
            (PressPhase::Release { pos, click: false, .. }, _) => {
                if let Some(Grab::Fill { source, last, menu, id }) = grab.take() {
                    if menu {
                        edits.push(GridEdit::FillMenu { range: last, anchor: pos });
                    } else {
                        swapped = fill.mods.cmd;
                        edits.push(GridEdit::Fill { range: EditEvent::commit(id, Mixed::Same(source), last.clone()) });
                        selection.select_range(last, &space);
                    }
                }
            }
            (PressPhase::Cancel { .. }, _) => {
                if let Some(Grab::Fill { id, .. }) = grab.take() {
                    edits.push(GridEdit::Fill { range: EditEvent::cancel(id, craft_behavior::CancelReason::Escape, None) });
                }
            }
            _ => {}
        }

        for side in 0..4u64 {
            let press = cx.part_if_declared(BORDER.at(side)).map_or(PressState::IDLE, |result| result.press);
            match (press.phase, current.clone()) {
                (PressPhase::DragStart { start, .. }, Some(source)) => {
                    let (top, _, left, _) = edges(&source);
                    let origin = geometry.cell_at(logical(start.origin));
                    let offset = (i64::from(origin.row) - i64::from(top), i64::from(origin.column) - i64::from(left));
                    let id = Self::next_gesture(cx);
                    *grab = Some(Grab::Move { last: CellAt::new(top, left), source: source.clone(), offset, id });
                    let label = range_label(words.move_cells.as_str());
                    edits.push(GridEdit::MoveCells { to: EditEvent::begin(id, Mixed::Same(CellAt::new(top, left)), label, None) });
                }
                (PressPhase::Drag { pos, .. }, _) => {
                    if let Some(Grab::Move { source, offset, last, id }) = grab {
                        let at = geometry.cell_at(logical(pos));
                        let shift = |value: u32, by: i64| (i64::from(value) - by).clamp(0, i64::from(u32::MAX)) as u32;
                        let to = CellAt::new(shift(at.row, offset.0), shift(at.column, offset.1));
                        *last = to;
                        edits.push(GridEdit::MoveCells { to: EditEvent::update(*id, to) });
                        let (top, bottom, left, right) = edges(source);
                        preview.moving = Some(CellRange::cells(to, CellAt::new(to.row + (bottom - top), to.column + (right - left))));
                    }
                }
                (PressPhase::Release { click: false, .. }, _) => {
                    if let Some(Grab::Move { source, last, id, .. }) = grab.take() {
                        let (top, bottom, left, right) = edges(&source);
                        edits.push(GridEdit::MoveCells { to: EditEvent::commit(id, Mixed::Same(CellAt::new(top, left)), last) });
                        let moved = CellRange::cells(last, CellAt::new(last.row + (bottom - top), last.column + (right - left)));
                        selection.select_range(moved, &space);
                    }
                }
                (PressPhase::Release { pos, click: true, .. }, _) => {
                    selection.select(geometry.cell_at(logical(pos)), &space);
                }
                (PressPhase::Cancel { .. }, _) => {
                    if let Some(Grab::Move { id, .. }) = grab.take() {
                        edits.push(GridEdit::MoveCells { to: EditEvent::cancel(id, craft_behavior::CancelReason::Escape, None) });
                    }
                }
                _ => {}
            }
        }
        if let Some(Grab::Fill { last, .. }) = grab {
            preview.fill = Some(last.clone());
        }
        if grab.is_some() && cx.take_claimed(ESCAPE) {
            cx.cancel_press(PressCancel::Escape);
        }
        (edits, swapped)
    }

    /// A press on a column (`rows` false) or row header: selects the line, a drag selects several, a secondary release
    /// asks for the header's menu.
    #[allow(clippy::too_many_arguments)]
    fn line_press(
        edits: &mut Vec<GridEdit>,
        selection: &mut CellSelection<u32>,
        grab: &mut Option<Grab>,
        space: &CellSpace<'_, u32>,
        geometry: &super::SheetGeometry,
        press: craft_behavior::PressState,
        (rows, line): (bool, u32),
        header: Rect,
    ) {
        let range = |a: u32, b: u32| if rows { Some(space.whole_rows(a.min(b), a.max(b))) } else { space.whole_columns(a.min(b), a.max(b)) };
        match press.phase {
            PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                *grab = Some(Grab::Lines { rows, anchor: line });
                if let Some(range) = range(line, line) {
                    if press.mods.cmd {
                        selection.add(CellAt::new(range.first, range.columns.start), space);
                    } else {
                        selection.select_range(range, space);
                    }
                }
            }
            PressPhase::Drag { pos, .. } => {
                if let Some(Grab::Lines { rows: dragging, anchor }) = grab
                    && *dragging == rows
                {
                    let at = geometry.mirror_pos(pos);
                    let to = if rows { geometry.row_at(at.y) } else { geometry.column_at(at.x) };
                    if let Some(range) = range(*anchor, to) {
                        selection.select_range(range, space);
                    }
                }
            }
            PressPhase::Release { pos, click, .. } => {
                if matches!(grab, Some(Grab::Lines { rows: dragging, .. }) if *dragging == rows) {
                    *grab = None;
                }
                let secondary = press.phase.start().is_some_and(|start| start.button == PointerButton::Secondary);
                if click && secondary {
                    let held = selection.ranges().iter().any(|held| {
                        let (top, bottom, left, right) = edges(held);
                        if rows { (top..=bottom).contains(&line) } else { (left..=right).contains(&line) }
                    });
                    if !held && let Some(range) = range(line, line) {
                        selection.select_range(range, space);
                    }
                    let target = if rows { GridMenuTarget::RowHeader(line) } else { GridMenuTarget::ColumnHeader(line) };
                    edits.push(GridEdit::ContextMenu { anchor: header, pointer: Some(pos), target });
                }
            }
            _ => {}
        }
    }

    /// A drag of a header edge: one resize gesture; Esc cancels.
    #[allow(clippy::too_many_arguments)]
    fn edge_press(
        cx: &mut Cx<'_, Self>,
        edits: &mut Vec<GridEdit>,
        grab: &mut Option<Grab>,
        label: &Arc<str>,
        (rows, line): (bool, u32),
        size: f32,
        zoom: f32,
        rtl: bool,
        press: craft_behavior::PressState,
    ) {
        let emit = |edits: &mut Vec<GridEdit>, event: EditEvent<f32>| {
            edits.push(if rows { GridEdit::ResizeRow { row: line, height: event } } else { GridEdit::ResizeColumn { column: line, width: event } });
        };
        match press.phase {
            PressPhase::DragStart { .. } => {
                let id = Self::next_gesture(cx);
                *grab = Some(Grab::Resize { rows, line, from: size, id, last: size });
                emit(edits, EditEvent::begin(id, Mixed::Same(size), label.clone(), None));
            }
            PressPhase::Drag { start, pos, .. } => {
                if let Some(Grab::Resize { rows: along_rows, line: held, from, id, last }) = grab
                    && *along_rows == rows
                    && *held == line
                {
                    let moved = if rows {
                        pos.y - start.origin.y
                    } else if rtl {
                        start.origin.x - pos.x
                    } else {
                        pos.x - start.origin.x
                    };
                    let next = (*from + moved / zoom).max(0.0);
                    *last = next;
                    emit(edits, EditEvent::update(*id, next));
                }
            }
            PressPhase::Release { click: false, .. } => {
                if let Some(Grab::Resize { rows: along_rows, line: held, from, id, last }) =
                    grab.take_if(|grab| matches!(grab, Grab::Resize { rows: r, line: l, .. } if *r == rows && *l == line))
                {
                    let _ = (along_rows, held);
                    emit(edits, EditEvent::commit(id, Mixed::Same(from), last));
                }
            }
            PressPhase::Cancel { .. } => {
                if let Some(Grab::Resize { id, .. }) =
                    grab.take_if(|grab| matches!(grab, Grab::Resize { rows: r, line: l, .. } if *r == rows && *l == line))
                {
                    emit(edits, EditEvent::cancel(id, craft_behavior::CancelReason::Escape, None));
                }
            }
            _ => {}
        }
    }
}
