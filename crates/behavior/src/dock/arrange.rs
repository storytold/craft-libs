//! Window › Arrange: the document arrangements over the floating document windows and
//! the document tab groups, and the rects they produce.

use emath::{Rect, vec2};

/// One Window › Arrange entry. The n-up entries split the document area into that many
/// cells and put one document group in each; the rest act on windows and views.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Arrangement {
    /// Tiles every document window across the document area, side by side.
    TileAllVertically,
    /// Tiles every document window across the document area, stacked.
    TileAllHorizontally,
    /// Two cells, side by side.
    TwoUpVertical,
    /// Two cells, stacked.
    TwoUpHorizontal,
    /// Three cells in a row.
    ThreeUpVertical,
    /// Three cells in a column.
    ThreeUpHorizontal,
    /// Three cells: one large cell beside two stacked cells.
    ThreeUpStacked,
    /// Four cells in a 2 by 2 grid.
    FourUp,
    /// Six cells in a 3 by 2 grid.
    SixUp,
    /// Every document back into one tab group.
    ConsolidateAllToTabs,
    /// Floating windows stepped from the top left ([`cascade_rects`](crate::placement::cascade_rects)).
    Cascade,
    /// Floating windows in an even grid.
    Tile,
    /// The active document into a floating window.
    FloatInWindow,
    /// Every document into its own floating window.
    FloatAllInWindows,
    /// A second view of the active document.
    NewWindowForDocument,
    /// Sets every document view to the active view's zoom.
    MatchZoom,
    /// Scrolls every document view to the active view's location.
    MatchLocation,
    /// Sets every document view to the active view's rotation.
    MatchRotation,
    /// Matches zoom, location and rotation together.
    MatchAll,
}

impl Arrangement {
    /// How many document groups an n-up or tile arrangement wants, `None` for the window and
    /// view entries.
    pub fn group_count(self, documents: usize) -> Option<usize> {
        match self {
            Self::TileAllVertically | Self::TileAllHorizontally | Self::Tile => Some(documents),
            Self::TwoUpVertical | Self::TwoUpHorizontal => Some(2),
            Self::ThreeUpVertical | Self::ThreeUpHorizontal | Self::ThreeUpStacked => Some(3),
            Self::FourUp => Some(4),
            Self::SixUp => Some(6),
            _ => None,
        }
    }

    /// Whether the entry is enabled with `documents` open and `floating` floating windows:
    /// n-up needs at least two documents, Float in Window a docked one, Consolidate a second
    /// group or a floating window, the Match entries at least two documents.
    pub fn enabled(self, documents: usize, groups: usize, floating: usize) -> bool {
        match self {
            Self::ConsolidateAllToTabs => groups > 1 || floating > 0,
            Self::FloatInWindow => documents > floating,
            Self::NewWindowForDocument => documents > 0,
            Self::TileAllVertically | Self::TileAllHorizontally | Self::Cascade | Self::Tile | Self::FloatAllInWindows => documents > 0,
            _ => documents > 1,
        }
    }
}

/// The cells of an n-up or Tile All arrangement inside `area`, in reading order. `Tile All
/// Vertically` gives `n` columns, `Horizontally` `n` rows, 3-up Stacked one left cell and two
/// stacked on the right, 4-up and 6-up grids of 2×2 and 3×2.
pub fn arrange_rects(arrangement: Arrangement, area: Rect, n: usize) -> Option<Vec<Rect>> {
    if n < 2 {
        return None;
    }
    let grid = |cols: usize, rows: usize, count: usize| -> Vec<Rect> {
        let (w, h) = (area.width() / cols as f32, area.height() / rows as f32);
        (0..count.min(cols * rows)).map(|i| Rect::from_min_size(area.min + vec2((i % cols) as f32 * w, (i / cols) as f32 * h), vec2(w, h))).collect()
    };
    Some(match arrangement {
        Arrangement::TileAllVertically => grid(n, 1, n),
        Arrangement::TileAllHorizontally => grid(1, n, n),
        Arrangement::TwoUpVertical => grid(2, 1, 2),
        Arrangement::TwoUpHorizontal => grid(1, 2, 2),
        Arrangement::ThreeUpVertical => grid(3, 1, 3),
        Arrangement::ThreeUpHorizontal => grid(1, 3, 3),
        Arrangement::ThreeUpStacked => {
            let (w, h) = (area.width() / 2.0, area.height() / 2.0);
            let right = area.min + vec2(w, 0.0);
            vec![
                Rect::from_min_size(area.min, vec2(w, area.height())),
                Rect::from_min_size(right, vec2(w, h)),
                Rect::from_min_size(right + vec2(0.0, h), vec2(w, h)),
            ]
        }
        Arrangement::FourUp => grid(2, 2, 4),
        Arrangement::SixUp => grid(3, 2, 6),
        Arrangement::Tile => {
            let cols = (n as f32).sqrt().ceil() as usize;
            grid(cols, n.div_ceil(cols), n)
        }
        _ => return None,
    })
}
