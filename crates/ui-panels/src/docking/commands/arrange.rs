//! The Window › Arrange commands and how they place the document windows.

use crate::runtime::commands::CommandState;
use craft_behavior::dock::{Arrangement, DocumentWindows, arrange_rects};
use craft_behavior::placement::cascade_rects;
use egui::{Rect, Vec2};
use std::hash::Hash;

/// The Window › Arrange commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui_macros::CommandSet)]
#[command(prefix = "arrange")]
#[non_exhaustive]
pub enum ArrangeCommands {
    /// Tile every document window across the document area, side by side.
    #[command(label = "Tile All Vertically")]
    TileAllVertically,
    /// Tile every document window across the document area, stacked.
    #[command(label = "Tile All Horizontally")]
    TileAllHorizontally,
    /// Two document groups, side by side.
    #[command(label = "2-up Vertical")]
    TwoUpVertical,
    /// Two document groups, stacked.
    #[command(label = "2-up Horizontal")]
    TwoUpHorizontal,
    /// Three document groups in a row.
    #[command(label = "3-up Vertical")]
    ThreeUpVertical,
    /// Three document groups in a column.
    #[command(label = "3-up Horizontal")]
    ThreeUpHorizontal,
    /// One large document group beside two stacked ones.
    #[command(label = "3-up Stacked")]
    ThreeUpStacked,
    /// Four document groups in a 2 by 2 grid.
    #[command(label = "4-up")]
    FourUp,
    /// Six document groups in a 3 by 2 grid.
    #[command(label = "6-up")]
    SixUp,
    /// Every document back into one tab group.
    #[command(label = "Consolidate All to Tabs")]
    ConsolidateAllToTabs,
    /// Floating windows stepped from the top left.
    #[command(label = "Cascade")]
    Cascade,
    /// Floating windows in an even grid.
    #[command(label = "Tile")]
    Tile,
    /// The active document into a floating window.
    #[command(label = "Float in Window")]
    FloatInWindow,
    /// Every document into its own floating window.
    #[command(label = "Float All in Windows")]
    FloatAllInWindows,
    /// A second view of the active document.
    #[command(label = "New Window for Document")]
    NewWindowForDocument,
    /// Set every document view to the active view's zoom.
    #[command(label = "Match Zoom")]
    MatchZoom,
    /// Scroll every document view to the active view's location.
    #[command(label = "Match Location")]
    MatchLocation,
    /// Set every document view to the active view's rotation.
    #[command(label = "Match Rotation")]
    MatchRotation,
    /// Match zoom, location and rotation together.
    #[command(label = "Match All")]
    MatchAll,
}

impl ArrangeCommands {
    /// The data entry this command runs.
    pub const fn arrangement(self) -> Arrangement {
        match self {
            ArrangeCommands::TileAllVertically => Arrangement::TileAllVertically,
            ArrangeCommands::TileAllHorizontally => Arrangement::TileAllHorizontally,
            ArrangeCommands::TwoUpVertical => Arrangement::TwoUpVertical,
            ArrangeCommands::TwoUpHorizontal => Arrangement::TwoUpHorizontal,
            ArrangeCommands::ThreeUpVertical => Arrangement::ThreeUpVertical,
            ArrangeCommands::ThreeUpHorizontal => Arrangement::ThreeUpHorizontal,
            ArrangeCommands::ThreeUpStacked => Arrangement::ThreeUpStacked,
            ArrangeCommands::FourUp => Arrangement::FourUp,
            ArrangeCommands::SixUp => Arrangement::SixUp,
            ArrangeCommands::ConsolidateAllToTabs => Arrangement::ConsolidateAllToTabs,
            ArrangeCommands::Cascade => Arrangement::Cascade,
            ArrangeCommands::Tile => Arrangement::Tile,
            ArrangeCommands::FloatInWindow => Arrangement::FloatInWindow,
            ArrangeCommands::FloatAllInWindows => Arrangement::FloatAllInWindows,
            ArrangeCommands::NewWindowForDocument => Arrangement::NewWindowForDocument,
            ArrangeCommands::MatchZoom => Arrangement::MatchZoom,
            ArrangeCommands::MatchLocation => Arrangement::MatchLocation,
            ArrangeCommands::MatchRotation => Arrangement::MatchRotation,
            ArrangeCommands::MatchAll => Arrangement::MatchAll,
        }
    }
}

/// The state of each Arrange command with `documents` open, in `groups` docked groups and `floating` floating
/// windows.
pub fn arrange_states(documents: usize, groups: usize, floating: usize) -> Vec<(ArrangeCommands, CommandState)> {
    ALL_ARRANGE
        .into_iter()
        .map(|command| (command, CommandState::new().enabled(command.arrangement().enabled(documents, groups, floating))))
        .collect()
}

const ALL_ARRANGE: [ArrangeCommands; 19] = [
    ArrangeCommands::TileAllVertically,
    ArrangeCommands::TileAllHorizontally,
    ArrangeCommands::TwoUpVertical,
    ArrangeCommands::TwoUpHorizontal,
    ArrangeCommands::ThreeUpVertical,
    ArrangeCommands::ThreeUpHorizontal,
    ArrangeCommands::ThreeUpStacked,
    ArrangeCommands::FourUp,
    ArrangeCommands::SixUp,
    ArrangeCommands::ConsolidateAllToTabs,
    ArrangeCommands::Cascade,
    ArrangeCommands::Tile,
    ArrangeCommands::FloatInWindow,
    ArrangeCommands::FloatAllInWindows,
    ArrangeCommands::NewWindowForDocument,
    ArrangeCommands::MatchZoom,
    ArrangeCommands::MatchLocation,
    ArrangeCommands::MatchRotation,
    ArrangeCommands::MatchAll,
];

/// The document area an arrangement works in.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ArrangeSpace<D> {
    /// The document area.
    pub area: Rect,
    /// The size of a new document window.
    pub size: Vec2,
    /// The cascade step (`size.window.cascade`).
    pub step: f32,
    /// The active document.
    pub active: Option<D>,
}

impl<D> ArrangeSpace<D> {
    /// The document area `area`, new windows of `size` stepped by `step`, and the `active` document.
    pub fn new(area: Rect, size: Vec2, step: f32, active: Option<D>) -> Self {
        ArrangeSpace { area, size, step, active }
    }
}

/// What the host does after an arrangement changed the windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Arranged {
    /// The document windows changed: the host applies `DocumentWindows::to_actions`, and lays the docked groups out
    /// with `arrange_rects` for an n-up arrangement.
    Windows,
    /// Every document view takes the active view's zoom, location and rotation, as flagged.
    #[non_exhaustive]
    Views {
        /// Match the zoom.
        zoom: bool,
        /// Match the scroll location.
        location: bool,
        /// Match the rotation.
        rotation: bool,
    },
}

/// Runs an Arrange command on the document windows.
pub fn arrange<D: Clone + Eq + Hash>(windows: &mut DocumentWindows<D>, command: ArrangeCommands, space: &ArrangeSpace<D>) -> Arranged {
    let arrangement = command.arrangement();
    let ArrangeSpace { area, size, step, active } = space;
    let next_slot = |windows: &DocumentWindows<D>| cascade_rects(*area, *size, windows.floating.len() + 1, *step).pop();
    match arrangement {
        Arrangement::TileAllVertically | Arrangement::TileAllHorizontally => {
            windows.float_all(*area, *size, *step);
            tile_floating(windows, arrangement, *area);
        }
        Arrangement::Tile => tile_floating(windows, arrangement, *area),
        Arrangement::TwoUpVertical
        | Arrangement::TwoUpHorizontal
        | Arrangement::ThreeUpVertical
        | Arrangement::ThreeUpHorizontal
        | Arrangement::ThreeUpStacked
        | Arrangement::FourUp
        | Arrangement::SixUp => {
            windows.consolidate(active.as_ref());
            windows.split_into(arrangement.group_count(0).unwrap_or(0));
        }
        Arrangement::ConsolidateAllToTabs => windows.consolidate(active.as_ref()),
        Arrangement::Cascade => {
            let rects = cascade_rects(*area, *size, windows.floating.len(), *step);
            for ((_, rect), cascaded) in windows.floating.iter_mut().zip(rects) {
                *rect = cascaded;
            }
        }
        Arrangement::FloatInWindow => {
            if let (Some(document), Some(rect)) = (active, next_slot(windows)) {
                windows.float(document, rect);
            }
        }
        Arrangement::FloatAllInWindows => windows.float_all(*area, *size, *step),
        Arrangement::NewWindowForDocument => {
            if let (Some(document), Some(rect)) = (active, next_slot(windows)) {
                windows.new_window_for(document, rect);
            }
        }
        Arrangement::MatchZoom => return Arranged::Views { zoom: true, location: false, rotation: false },
        Arrangement::MatchLocation => return Arranged::Views { zoom: false, location: true, rotation: false },
        Arrangement::MatchRotation => return Arranged::Views { zoom: false, location: false, rotation: true },
        Arrangement::MatchAll => return Arranged::Views { zoom: true, location: true, rotation: true },
        _ => {}
    }
    Arranged::Windows
}

fn tile_floating<D: Clone + Eq + Hash>(windows: &mut DocumentWindows<D>, arrangement: Arrangement, area: Rect) {
    let Some(rects) = arrange_rects(arrangement, area, windows.floating.len()) else { return };
    for ((_, rect), tiled) in windows.floating.iter_mut().zip(rects) {
        *rect = tiled;
    }
}
