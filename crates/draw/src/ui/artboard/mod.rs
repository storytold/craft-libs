//! The Artboard tool: [`ArtboardOverlay`] (the boards' bodies and the active board's handles, a canvas control),
//! [`ArtboardTool`] (the canvas tool for the pasteboard), [`ArtboardBar`] (the options bar's members) and
//! [`ArtboardProperties`] (the Properties section). Frames and names stay the `Pasteboard`'s; the "+" buttons are
//! [`AddBesideHandles`](crate::ui::add_beside::AddBesideHandles), shown over the active board while
//! [`ArtboardOverlayOutput::dragging`] is false and answered with [`ArtboardTool::add_beside`].
//!
//! The control and the tool report the same [`ArtboardEvent`], so the app has one handler.

use crate::geometry::artboard::{resized_keep_ratio, resized_rect};
use crate::ui::add_beside::AddSide;
use crate::ui::slice::Run;
use craft_behavior::EditEvent;
use craft_behavior::kurbo::{Point, Rect, Size, Vec2};
use craft_keymap::Mods;
use craft_theme::Srgba;
use std::sync::Arc;

mod bar;
mod overlay;
mod properties;
#[cfg(feature = "screen-testing")]
mod stories;
mod tool;
mod words;

pub use bar::{ArtboardBar, ArtboardBarOutput, ArtboardCommand};
pub use overlay::{ArtboardOverlay, ArtboardOverlayOutput};
pub use properties::{ArtboardProperties, ArtboardPropertiesOutput};
pub use tool::ArtboardTool;
pub use words::{ArtboardToolWords, ArtboardWords};

/// An artboard's background.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum ArtboardBackground {
    /// An opaque white board.
    #[default]
    White,
    /// An opaque black board.
    Black,
    /// No fill; the checkerboard or what lies under shows through.
    Transparent,
    /// Other…: a colour asked for with a `ColorRequest`.
    Custom(Srgba),
}

/// One artboard as the control and the tool see it this pass: a read-only snapshot.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ArtboardItem {
    /// The board's name, as shown on its label.
    pub name: String,
    /// In document pixels.
    pub rect: Rect,
    /// A locked board selects but never moves or resizes (Illustrator).
    pub locked: bool,
    /// The board's fill.
    pub background: ArtboardBackground,
}

impl ArtboardItem {
    /// An unlocked white board named `name` at `rect`.
    pub fn new(name: impl Into<String>, rect: Rect) -> Self {
        ArtboardItem { name: name.into(), rect, locked: false, background: ArtboardBackground::White }
    }
}

/// A size preset of the options bar and the Properties section: the app's list; the library has no preset data.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ArtboardPreset {
    /// The name the preset menu shows.
    pub name: String,
    /// In document pixels.
    pub size: Size,
}

impl ArtboardPreset {
    /// A preset named `name` of `size`.
    pub fn new(name: impl Into<String>, size: Size) -> Self {
        ArtboardPreset { name: name.into(), size }
    }
}

/// Which product's rules the control and the tool follow.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ArtboardRules {
    /// The "+" buttons beside the selected board.
    pub plus_buttons: bool,
    /// Shift-click toggles a board in the selection; Shift-drag on the pasteboard is a marquee over boards; Shift-drag
    /// inside a board draws a new one.
    pub multi_select: bool,
    /// A double click inside a board reports `OpenOptions` (Illustrator's Artboard Options).
    pub open_on_double_click: bool,
    /// A click on an object outside the boards fits an artboard to it (`FitObject`).
    pub fit_object: bool,
    /// Alt-drag inside a board moves a copy.
    pub alt_copies: bool,
    /// Esc with no drag leaves the tool (`Leave`).
    pub escape_leaves: bool,
    /// The arrow step in document pixels (the app's keyboard increment); Shift ten times.
    pub arrow_step: f64,
}

impl ArtboardRules {
    /// Photoshop's: "+" buttons, one board selected.
    pub const PHOTOSHOP: ArtboardRules = ArtboardRules {
        plus_buttons: true,
        multi_select: false,
        open_on_double_click: false,
        fit_object: false,
        alt_copies: false,
        escape_leaves: false,
        arrow_step: 1.0,
    };
    /// Illustrator's: no "+" buttons, multi-select and marquee, a double click opens Options, fit to an object, Alt
    /// copies, Esc leaves.
    pub const ILLUSTRATOR: ArtboardRules = ArtboardRules {
        plus_buttons: false,
        multi_select: true,
        open_on_double_click: true,
        fit_object: true,
        alt_copies: true,
        escape_leaves: true,
        arrow_step: 1.0,
    };

    /// These rules with the app's keyboard increment.
    #[must_use]
    pub fn arrow_step(mut self, step: f64) -> Self {
        self.arrow_step = step;
        self
    }
}

impl Default for ArtboardRules {
    fn default() -> Self {
        ArtboardRules::PHOTOSHOP
    }
}

/// What the control and the tool report.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ArtboardEvent {
    /// A new board: `Begin` past the click slop, `Update`, `Commit` with its pixel rect, `Cancel`.
    Draw(EditEvent<Rect>),
    /// The selected boards moving by an offset in whole pixels (with their contents); `copy` for an Alt-drag.
    #[non_exhaustive]
    Move {
        /// Indices of the moved boards.
        indices: Vec<usize>,
        /// The gesture and the offset in whole pixels.
        event: EditEvent<Vec2>,
        /// Whether the boards are copied rather than moved.
        copy: bool,
    },
    /// Board `index` resized by a handle (contents stay): its new pixel rect.
    #[non_exhaustive]
    Resize {
        /// Index of the resized board.
        index: usize,
        /// The gesture and the new pixel rect.
        event: EditEvent<Rect>,
    },
    /// A "+" button: a board of the same size beside board `index`; `copy` (Alt) with
    /// its contents.
    #[non_exhaustive]
    AddBeside {
        /// Index of the board the new one goes beside.
        index: usize,
        /// Which side of that board.
        side: AddSide,
        /// Whether the contents are copied too.
        copy: bool,
    },
    /// The selection changed to `indices` (empty: none).
    #[non_exhaustive]
    Select {
        /// The newly selected boards.
        indices: Vec<usize>,
        /// The modifiers held at the click.
        mods: Mods,
    },
    /// Delete removed the selected boards.
    #[non_exhaustive]
    Delete {
        /// Indices of the removed boards.
        indices: Vec<usize>,
    },
    /// Enter, or a double click inside a board under [`ArtboardRules::open_on_double_click`]: Artboard Options.
    #[non_exhaustive]
    OpenOptions {
        /// Index of the board whose options open.
        index: usize,
    },
    /// Illustrator: a click on an object outside the boards: fit a board to the object at `at`.
    #[non_exhaustive]
    FitObject {
        /// The clicked point, in document space.
        at: Point,
    },
    /// Illustrator: Esc with no drag: the app returns to its selection tool.
    Leave,
}

/// The board state the control and the tool act on for a key.
pub(crate) struct Boards<'a> {
    pub(crate) items: &'a [ArtboardItem],
    pub(crate) selected: &'a [usize],
}

impl Boards<'_> {
    /// The active board: the first selected one this pass's boards still have.
    pub(crate) fn active(&self) -> Option<usize> {
        self.selected.first().copied().filter(|&i| i < self.items.len())
    }

    /// The selected boards that exist and are not locked.
    pub(crate) fn movable(&self) -> Vec<usize> {
        self.selected.iter().copied().filter(|&i| self.items.get(i).is_some_and(|b| !b.locked)).collect()
    }
}

pub(crate) fn overlaps(a: Rect, b: Rect) -> bool {
    let (a, b) = (a.abs(), b.abs());
    a.x0 <= b.x1 && a.x1 >= b.x0 && a.y0 <= b.y1 && a.y1 >= b.y0
}

/// What a key asks of the selected boards.
pub(crate) enum KeyAct {
    /// An arrow key: the direction in document space (a unit step per axis) and the modifiers.
    Nudge {
        direction: Vec2,
        mods: Mods,
    },
    Delete,
    Submit,
}

/// The events of a key on the selected boards: the one rule the control's focused board and the tool share. Arrows
/// move the selected boards by the rules' step (Shift ten times), Alt and the arrows resize from the end edges, Delete
/// removes the selection, Enter opens the options.
pub(crate) fn key_events(boards: &Boards<'_>, rules: &ArtboardRules, act: KeyAct, words: &ArtboardToolWords) -> Vec<ArtboardEvent> {
    let Some(active) = boards.active() else { return Vec::new() };
    match act {
        KeyAct::Delete => {
            let indices = boards.selected.iter().copied().filter(|&i| i < boards.items.len()).collect();
            vec![ArtboardEvent::Delete { indices }]
        }
        KeyAct::Submit => vec![ArtboardEvent::OpenOptions { index: active }],
        KeyAct::Nudge { direction, mods } => {
            let step = rules.arrow_step * if mods.shift { 10.0 } else { 1.0 };
            let by = direction * step;
            if mods.alt {
                // The end edges (right, bottom) move; a locked board stays.
                let board = &boards.items[active];
                if board.locked {
                    return Vec::new();
                }
                let handle = if by.x == 0.0 { [0, 1] } else { [1, 0] };
                let rect = resized_rect(board.rect, handle, by, false);
                let event = Run::set(words.resize_board.as_str().into(), board.rect, rect);
                vec![ArtboardEvent::Resize { index: active, event }]
            } else {
                let indices = boards.movable();
                if indices.is_empty() {
                    return Vec::new();
                }
                let event = Run::set(words.move_board.as_str().into(), Vec2::ZERO, by);
                vec![ArtboardEvent::Move { indices, event, copy: false }]
            }
        }
    }
}

/// `rect` resized by `handle` dragged by `by`: Shift on a corner keeps the proportions, Alt resizes about the centre.
pub(crate) fn resized(rect: Rect, handle: [i8; 2], by: Vec2, mods: Mods) -> Rect {
    if mods.shift { resized_keep_ratio(rect, handle, by, mods.alt) } else { resized_rect(rect, handle, by, mods.alt) }
}

/// The label of an undo step.
pub(crate) fn label(word: &craft_ui::runtime::env::Label) -> Arc<str> {
    word.as_str().into()
}
