//! Outline navigation: one step through lists, trees, ragged rows and grids from a key move.

use crate::intent::{MoveMods, NavMove};
use emath::Rect;

/// The geometry a collection's keyboard navigation follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NavShape {
    /// One row after another.
    Flat,
    /// Rows that expand and collapse.
    Tree,
    /// Equal-width cells that wrap at `columns`.
    #[non_exhaustive]
    WrappedGrid {
        /// Cells per row.
        columns: usize,
    },
    /// Equal-width cells in `columns` columns whose Left and Right stop at the row ends; Up and Down stop at the first
    /// and last rows.
    #[non_exhaustive]
    BoundedGrid {
        /// Cells per row.
        columns: usize,
    },
    /// Rows of different lengths.
    RaggedGrid,
    /// Cells of a sheet.
    Cell,
    /// Items with free positions; arrows go to the nearest item in that direction.
    Spatial,
}

impl NavShape {
    /// Equal-width cells that wrap at `columns`.
    pub const fn wrapped_grid(columns: usize) -> Self {
        Self::WrappedGrid { columns }
    }

    /// Equal-width cells in `columns` columns whose Left and Right stop at the row ends.
    pub const fn bounded_grid(columns: usize) -> Self {
        Self::BoundedGrid { columns }
    }
}

/// Positions, not keys: the key-free part of `DisplayOrder`.
pub trait NavOutline {
    /// The number of positions.
    fn len(&self) -> usize;
    /// The nesting depth of the position, 0 for a root.
    fn depth(&self, at: usize) -> usize;
    /// Whether the position has children.
    fn has_children(&self, at: usize) -> bool;
    /// Whether the position's children show.
    fn open(&self, at: usize) -> bool;
    /// Whether the position is a section header, which a move skips.
    fn is_section(&self, at: usize) -> bool;
    /// Whether the position can take the lead.
    fn enabled(&self, at: usize) -> bool;

    /// `RaggedGrid`: the number of positions in `row`; rows hold consecutive positions. `None` past the last row.
    fn row_len(&self, _row: usize) -> Option<usize> {
        None
    }

    /// `Cell`: the cells of every position (a row of a sheet).
    fn columns(&self) -> usize {
        1
    }

    /// `WrappedGrid`: the column of position 0 when the grid has blank cells before it (the leading days of a
    /// calendar month), `None` for a grid that fills from its first column. A grid with a first column keeps its rows
    /// whole: Home and End go to the ends of the lead's row.
    fn first_column(&self) -> Option<usize> {
        None
    }

    /// `Spatial`: where the position sits, in any one coordinate space.
    fn rect(&self, _at: usize) -> Option<Rect> {
        None
    }

    /// Whether there are no positions.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Where a navigation stands between moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NavState {
    /// The lead position, `None` before the first move.
    pub lead: Option<usize>,
    /// The fixed end of an extended selection.
    pub anchor: Option<usize>,
    /// Positions a page move covers.
    pub page: usize,
    /// Whether a move past an end continues at the other end.
    pub wrap: bool,
    /// `Cell`: the column of the lead cell.
    pub column: usize,
}

impl NavState {
    /// A state with no lead, a page of ten and no wrap.
    pub const fn new() -> Self {
        NavState { lead: None, anchor: None, page: 10, wrap: false, column: 0 }
    }

    /// A state at `lead`.
    pub const fn at(lead: usize) -> Self {
        NavState { lead: Some(lead), anchor: None, page: 10, wrap: false, column: 0 }
    }

    /// Sets the page size.
    pub const fn page(mut self, rows: usize) -> Self {
        self.page = if rows == 0 { 1 } else { rows };
        self
    }

    /// Sets whether moves wrap.
    pub const fn wrap(mut self, on: bool) -> Self {
        self.wrap = on;
        self
    }

    /// Sets the lead cell's column.
    pub const fn column(mut self, column: usize) -> Self {
        self.column = column;
        self
    }
}

impl Default for NavState {
    fn default() -> Self {
        NavState::new()
    }
}

/// What one move did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NavStep {
    /// Nothing changed.
    Stay,
    /// The lead moved to this position; `state` holds the new lead and anchor.
    Moved(usize),
    /// A tree row opens.
    Expand(usize),
    /// A tree row closes.
    Collapse(usize),
    /// The lead cell moved to this column of the same row; `state.column` holds it.
    Column(usize),
    /// The shape has no stepping rule for this move; the caller handles it.
    Unhandled,
}

/// Steps the lead of a navigation of `shape` over `outline` by `mv`. `mods.extend` keeps the anchor at the
/// old lead; any other move clears it. `RaggedGrid` reads `row_len`, `Cell` reads `columns` and `Spatial` reads
/// `rect` from the outline.
pub fn step(shape: NavShape, outline: &dyn NavOutline, state: &mut NavState, mv: NavMove, mods: MoveMods) -> NavStep {
    let len = outline.len();
    if len == 0 {
        return NavStep::Stay;
    }
    let Some(lead) = state.lead else {
        return first_move(outline, state, mv, mods);
    };
    let lead = lead.min(len - 1);
    if let Some(step) = header_rule(shape, outline, lead, mv) {
        return step;
    }
    let result = match shape {
        NavShape::Flat => linear(outline, state, lead, mv),
        NavShape::Tree => tree(outline, state, lead, mv),
        NavShape::WrappedGrid { columns } => grid(outline, state, lead, mv, mods, columns.max(1), true),
        NavShape::BoundedGrid { columns } => grid(outline, state, lead, mv, mods, columns.max(1), false),
        NavShape::RaggedGrid => ragged(outline, state, lead, mv),
        NavShape::Cell => cell(outline, state, lead, mv, mods),
        NavShape::Spatial => spatial(outline, lead, mv),
    };
    if let NavStep::Column(_) = result {
        state.anchor = if mods.extend { state.anchor.or(Some(lead)) } else { None };
    }
    if let NavStep::Moved(to) = result {
        state.anchor = if mods.extend { state.anchor.or(Some(lead)) } else { None };
        state.lead = Some(to);
        if to == lead {
            return NavStep::Stay;
        }
    }
    result
}

/// A lead on a group header (a section with children, which moves skip but a press can reach): Forward opens it and
/// Back closes it, the tree rule in every shape that reads Left and Right as steps.
fn header_rule(shape: NavShape, outline: &dyn NavOutline, lead: usize, mv: NavMove) -> Option<NavStep> {
    if matches!(shape, NavShape::Cell | NavShape::Spatial) || !outline.is_section(lead) || !outline.has_children(lead) {
        return None;
    }
    match mv {
        NavMove::Forward if !outline.open(lead) => Some(NavStep::Expand(lead)),
        NavMove::Back if outline.open(lead) => Some(NavStep::Collapse(lead)),
        _ => None,
    }
}

/// The first move from no lead: forward moves land on the first position, backward moves on the last.
fn first_move(outline: &dyn NavOutline, state: &mut NavState, mv: NavMove, mods: MoveMods) -> NavStep {
    let backward = matches!(mv, NavMove::Back | NavMove::Up | NavMove::PageUp | NavMove::End);
    let to = if backward { last_enabled(outline) } else { first_enabled(outline) };
    match to {
        Some(at) => {
            state.lead = Some(at);
            state.anchor = mods.extend.then_some(at);
            NavStep::Moved(at)
        }
        None => NavStep::Stay,
    }
}

fn selectable(outline: &dyn NavOutline, at: usize) -> bool {
    outline.enabled(at) && !outline.is_section(at)
}

fn first_enabled(outline: &dyn NavOutline) -> Option<usize> {
    (0..outline.len()).find(|&at| selectable(outline, at))
}

fn last_enabled(outline: &dyn NavOutline) -> Option<usize> {
    (0..outline.len()).rev().find(|&at| selectable(outline, at))
}

/// The next selectable position from `from` in `direction` (+1 or -1), wrapping when the state says so.
fn neighbour(outline: &dyn NavOutline, state: &NavState, from: usize, forward: bool) -> Option<usize> {
    let len = outline.len();
    let mut at = from;
    for _ in 0..len {
        at = match (forward, at) {
            (true, at) if at + 1 < len => at + 1,
            (true, _) if state.wrap => 0,
            (false, at) if at > 0 => at - 1,
            (false, _) if state.wrap => len - 1,
            _ => return None,
        };
        if selectable(outline, at) {
            return Some(at);
        }
    }
    None
}

/// `count` selectable positions from `from`, stopping at the last one reached. A page move that starts on the last
/// position of its direction wraps like a single step does, when the state wraps.
fn advance(outline: &dyn NavOutline, state: &NavState, from: usize, forward: bool, count: usize) -> usize {
    let stop = NavState { wrap: false, ..*state };
    let mut at = from;
    for _ in 0..count {
        match neighbour(outline, &stop, at, forward) {
            Some(next) => at = next,
            None => break,
        }
    }
    if at == from && state.wrap {
        return neighbour(outline, state, from, forward).unwrap_or(from);
    }
    at
}

fn moved(to: Option<usize>) -> NavStep {
    to.map_or(NavStep::Stay, NavStep::Moved)
}

fn linear(outline: &dyn NavOutline, state: &NavState, lead: usize, mv: NavMove) -> NavStep {
    match mv {
        NavMove::Back | NavMove::Up => moved(neighbour(outline, state, lead, false)),
        NavMove::Forward | NavMove::Down => moved(neighbour(outline, state, lead, true)),
        NavMove::PageUp => NavStep::Moved(advance(outline, state, lead, false, state.page)),
        NavMove::PageDown => NavStep::Moved(advance(outline, state, lead, true, state.page)),
        NavMove::Start => moved(first_enabled(outline)),
        NavMove::End => moved(last_enabled(outline)),
    }
}

fn tree(outline: &dyn NavOutline, state: &NavState, lead: usize, mv: NavMove) -> NavStep {
    let expandable = outline.has_children(lead);
    match mv {
        NavMove::Forward if expandable && !outline.open(lead) => NavStep::Expand(lead),
        NavMove::Forward if expandable => moved(neighbour(outline, state, lead, true)),
        NavMove::Forward => NavStep::Stay,
        NavMove::Back if expandable && outline.open(lead) => NavStep::Collapse(lead),
        NavMove::Back => {
            let depth = outline.depth(lead);
            let parent = (0..lead).rev().find(|&at| outline.depth(at) + 1 == depth && selectable(outline, at));
            if depth == 0 { NavStep::Stay } else { moved(parent) }
        }
        _ => linear(outline, state, lead, mv),
    }
}

/// The row and column of position `at` in a ragged grid, with the first position of its row.
fn locate(outline: &dyn NavOutline, at: usize) -> Option<(usize, usize, usize)> {
    let mut start = 0;
    for row in 0.. {
        let len = outline.row_len(row)?;
        if at < start + len {
            return Some((row, at - start, start));
        }
        start += len;
    }
    None
}

/// The position at `column` of `row`, clamped to the row's last position.
fn ragged_at(outline: &dyn NavOutline, row: usize, column: usize) -> Option<usize> {
    let start: usize = (0..row).map(|earlier| outline.row_len(earlier).unwrap_or(0)).sum();
    let len = outline.row_len(row).filter(|&len| len > 0)?;
    let at = start + column.min(len - 1);
    selectable(outline, at).then_some(at)
}

fn ragged(outline: &dyn NavOutline, state: &NavState, lead: usize, mv: NavMove) -> NavStep {
    let Some((row, column, _)) = locate(outline, lead) else { return NavStep::Unhandled };
    let rows = (0..).take_while(|&r| outline.row_len(r).is_some()).count();
    // A row without a position that takes the lead (a closed group's) is passed over in the move's direction.
    let toward = |rows: Vec<usize>| moved(rows.into_iter().find_map(|row| ragged_at(outline, row, column)));
    let vertical = |to: Option<usize>| moved(to.and_then(|row| ragged_at(outline, row, column)));
    match mv {
        NavMove::Back => moved(neighbour(outline, state, lead, false)),
        NavMove::Forward => moved(neighbour(outline, state, lead, true)),
        NavMove::Up => toward((0..row).rev().collect()),
        NavMove::Down => toward((row + 1..rows).collect()),
        NavMove::PageUp => vertical(Some(row.saturating_sub(state.page))),
        NavMove::PageDown => vertical(Some((row + state.page).min(rows - 1))),
        NavMove::Start => moved(first_enabled(outline)),
        NavMove::End => moved(last_enabled(outline)),
    }
}

/// A sheet: positions are rows and `state.column` is the lead cell's column. Home and End go to the row's ends,
/// with Ctrl (`mods.lead`) to the sheet's first and last row.
fn cell(outline: &dyn NavOutline, state: &mut NavState, lead: usize, mv: NavMove, mods: MoveMods) -> NavStep {
    let last_column = outline.columns().saturating_sub(1);
    let column = state.column.min(last_column);
    let across = |state: &mut NavState, to: usize| {
        state.column = to;
        if to == column { NavStep::Stay } else { NavStep::Column(to) }
    };
    match mv {
        NavMove::Back => across(state, column.saturating_sub(1)),
        NavMove::Forward => across(state, (column + 1).min(last_column)),
        NavMove::Start if mods.lead => moved(first_enabled(outline)),
        NavMove::End if mods.lead => moved(last_enabled(outline)),
        NavMove::Start => across(state, 0),
        NavMove::End => across(state, last_column),
        NavMove::Up | NavMove::Down | NavMove::PageUp | NavMove::PageDown => linear(outline, state, lead, mv),
    }
}

type Axis = fn(emath::Vec2) -> f32;

/// Items at free positions: the nearest item whose centre lies beyond the lead's in the move's direction, with
/// the off-axis distance counting double. Back and Forward are toward lower and higher x.
fn spatial(outline: &dyn NavOutline, lead: usize, mv: NavMove) -> NavStep {
    let Some(from) = outline.rect(lead).map(|rect| rect.center()) else { return NavStep::Unhandled };
    let (along, across): (Axis, Axis) = match mv {
        NavMove::Back => (|d| -d.x, |d| d.y),
        NavMove::Forward => (|d| d.x, |d| d.y),
        NavMove::Up | NavMove::PageUp => (|d| -d.y, |d| d.x),
        NavMove::Down | NavMove::PageDown => (|d| d.y, |d| d.x),
        NavMove::Start => return moved(first_enabled(outline)),
        NavMove::End => return moved(last_enabled(outline)),
    };
    let nearest = (0..outline.len())
        .filter(|&at| at != lead && selectable(outline, at))
        .filter_map(|at| outline.rect(at).map(|rect| (at, rect.center() - from)))
        .filter(|(_, delta)| along(*delta) > 0.0)
        .min_by(|(_, a), (_, b)| (along(*a) + 2.0 * across(*a).abs()).total_cmp(&(along(*b) + 2.0 * across(*b).abs())));
    moved(nearest.map(|(at, _)| at))
}

fn grid(outline: &dyn NavOutline, state: &NavState, lead: usize, mv: NavMove, mods: MoveMods, columns: usize, wraps: bool) -> NavStep {
    let len = outline.len();
    let blanks = outline.first_column().map_or(0, |column| column % columns);
    // Members sit after `blanks` blank cells of the first row.
    let column = lead.saturating_add(blanks) % columns;
    let vertical = |down: bool, rows: usize| {
        let delta = columns * rows;
        let to = if down { lead.checked_add(delta).filter(|&at| at < len) } else { lead.checked_sub(delta) };
        // A step past the last row's end lands on the last position of a shorter row only for pages.
        to.filter(|&at| selectable(outline, at))
    };
    let row_start = (lead + blanks) / columns * columns;
    let beside = |forward: bool| {
        let at_row_end = if forward { column + 1 == columns } else { column == 0 };
        if !wraps && at_row_end { None } else { neighbour(outline, state, lead, forward) }
    };
    match mv {
        NavMove::Back => moved(beside(false)),
        NavMove::Forward => moved(beside(true)),
        NavMove::Up => moved(vertical(false, 1)),
        NavMove::Down => moved(vertical(true, 1)),
        NavMove::PageUp => {
            let top = if column >= blanks { column - blanks } else { column + columns - blanks };
            moved(vertical(false, state.page).or_else(|| Some(top).filter(|&at| at < len && selectable(outline, at))))
        }
        NavMove::PageDown => {
            let last = len - 1 + blanks;
            let at = ((last / columns * columns + column).min(last)).saturating_sub(blanks);
            moved(vertical(true, state.page).or_else(|| Some(at).filter(|&at| selectable(outline, at))))
        }
        NavMove::Start if outline.first_column().is_some() && !mods.lead => {
            moved(Some(row_start.saturating_sub(blanks)).filter(|&at| selectable(outline, at)))
        }
        NavMove::End if outline.first_column().is_some() && !mods.lead => {
            let end = (row_start + columns - 1).min(len - 1 + blanks) - blanks;
            moved(Some(end).filter(|&at| selectable(outline, at)))
        }
        NavMove::Start => moved(first_enabled(outline)),
        NavMove::End => moved(last_enabled(outline)),
    }
}
