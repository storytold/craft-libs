//! `ComparePair`, `survey_columns` and `survey_cell`: the two-pane and survey arrangements, as geometry.

use egui::{Rect, pos2, vec2};

/// The space round the panes and between them, in points: the caller's named layout sizes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ArrangeSpace {
    /// The margin round the panes.
    pub margin: f32,
    /// The gap between two panes.
    pub gap: f32,
}

impl ArrangeSpace {
    /// A margin of `margin` round the panes and `gap` between them.
    pub const fn new(margin: f32, gap: f32) -> Self {
        ArrangeSpace { margin, gap }
    }
}

/// The two panes of Compare: equal rects in the area inset by the margin and split by the gap. In a right-to-left
/// interface the Select pane is the right one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComparePair {
    space: ArrangeSpace,
    rtl: bool,
}

impl ComparePair {
    /// Left to right, with `space` round and between the panes.
    pub const fn new(space: ArrangeSpace) -> Self {
        ComparePair { space, rtl: false }
    }

    /// Mirrors the panes: the Select pane is at the reading start.
    #[must_use]
    pub const fn rtl(self, rtl: bool) -> Self {
        ComparePair { rtl, ..self }
    }

    /// The Select pane and the Candidate pane, in that order: each `(area - 2 x margin - gap) / 2` wide.
    pub fn rects(&self, area: Rect) -> [Rect; 2] {
        let inner = area.shrink(self.space.margin);
        let side = ((inner.width() - self.space.gap) / 2.0).max(0.0);
        let left = Rect::from_min_size(inner.min, vec2(side, inner.height()));
        let right = Rect::from_min_size(pos2(inner.max.x - side, inner.min.y), vec2(side, inner.height()));
        if self.rtl { [right, left] } else { [left, right] }
    }
}

/// How many columns show `n` survey tiles largest in `area`, for pictures `aspect` (width over height) wide: the
/// column count in `1..=n` whose tile `min(width / columns, height / rows * aspect)` is largest (the first on a tie).
pub fn survey_columns(n: usize, area: Rect, aspect: f32) -> usize {
    let mut best = (1, 0.0f32);
    for columns in 1..=n.max(1) {
        let rows = n.div_ceil(columns);
        let size = (area.width() / columns as f32).min(area.height() / rows as f32 * aspect);
        if size > best.1 {
            best = (columns, size);
        }
    }
    best.0
}

/// The cell of tile `index` among `count` survey tiles in `area`, for pictures `aspect` wide: `survey_columns` columns
/// of equal cells fill the area, the tiles run along the rows from the reading start (the right edge when `rtl`).
pub fn survey_cell(count: usize, index: usize, area: Rect, aspect: f32, rtl: bool) -> Rect {
    let columns = survey_columns(count, area, aspect);
    let rows = count.max(1).div_ceil(columns);
    let cell = vec2(area.width() / columns as f32, area.height() / rows as f32);
    let (column, row) = ((index % columns) as f32, (index / columns) as f32);
    let x = if rtl { area.max.x - cell.x * (column + 1.0) } else { area.min.x + cell.x * column };
    Rect::from_min_size(pos2(x, area.min.y + cell.y * row), cell)
}
