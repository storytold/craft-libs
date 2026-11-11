//! The Slice and Slice Select tools: [`SliceOverlay`] (the slices on the canvas, a canvas control), [`SliceTool`] (the
//! canvas tool that draws a new slice and takes the presses away from every slice), [`SliceBar`] (the options bar's
//! members) and the Slice Options and Divide Slices dialogs.
//!
//! The control and the tool report the same [`SliceEvent`], so the app has one handler.
//!
//! ```no_run
//! use craft_draw::ui::slice::{SliceCommand, SliceEvent, SliceItem, SliceOverlay, SliceTool};
//! use craft_ui::canvas::{Canvas, ToolHost, ToolPath};
//! use craft_ui::contract::{Show as _, ShowContainer as _};
//! # fn demo(ui: &mut egui::Ui, tool: &mut SliceTool, slices: &[SliceItem], selected: Option<usize>, locked: bool) {
//! let shown = Canvas::new("Document").show(ui, |ui, body| {
//!     SliceOverlay::new("Slices", slices).selected(selected).pick(true).locked(locked).show_at(ui, body.rect)
//! });
//! let _ = ToolHost::new().run(ui, &shown.output, tool);
//! let mut events = tool.take_events();
//! events.extend(shown.inner.map(|output| output.events).unwrap_or_default());
//! for event in events {
//!     match event {
//!         SliceEvent::Command(SliceCommand::Delete) => { /* slice.delete */ }
//!         SliceEvent::Refused(_) => { /* "Slices are locked (View › Lock Slices)" in the status bar */ }
//!         _ => {}
//!     }
//! }
//! # }
//! ```

use craft_behavior::geometry::Constraint;
use craft_behavior::kurbo::{Point, Rect, Vec2};
use craft_behavior::{Axis, CancelReason, EditEvent, GestureId, Mixed};
use craft_keymap::Mods;
use std::sync::Arc;

mod bar;
mod dialogs;
mod overlay;
#[cfg(feature = "screen-testing")]
mod stories;
mod tool;
mod words;

pub use bar::{SliceBar, SliceBarOutput};
pub use dialogs::{DivideBy, DivideSlices, DivideSlicesDialog, SliceKind, SliceOptions, SliceOptionsDialog};
pub use overlay::{SliceOverlay, SliceOverlayOutput};
pub use tool::SliceTool;
pub use words::{SliceToolWords, SliceWords};

/// Slice draws new slices; Select picks, moves and resizes them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SliceMode {
    /// Draws new slices.
    #[default]
    Slice,
    /// Picks, moves and resizes slices.
    Select,
}

/// Where a slice comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SliceOrigin {
    /// Drawn by the user (or promoted): moves and resizes.
    #[default]
    User,
    /// Made by the app to fill the space between user slices: drawn dashed, never moves, hidden with Hide Auto
    /// Slices.
    Auto,
    /// Follows a layer's bounds: never moves; its badge carries a small painted square.
    Layer,
}

impl SliceOrigin {
    /// Whether the app stores the slice (user and layer slices, which have an id) and so can delete it. An auto
    /// slice goes when the slices around it do.
    pub fn stored(self) -> bool {
        !matches!(self, SliceOrigin::Auto)
    }
}

/// One slice as the control and the tool see it this pass: a read-only snapshot of the app's resolved slices.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SliceItem {
    /// The slice's display number (1, 2, …), how events and the badge name it.
    pub number: usize,
    /// The id of the stored slice, for user and layer slices alike (a layer slice is a stored slice that follows a
    /// layer); `None` for an auto slice.
    pub id: Option<u32>,
    /// In document pixels.
    pub rect: Rect,
    /// Where the slice comes from.
    pub origin: SliceOrigin,
}

impl SliceItem {
    /// The slice numbered `number` at `rect`.
    pub fn new(number: usize, id: Option<u32>, rect: Rect, origin: SliceOrigin) -> Self {
        SliceItem { number, id, rect, origin }
    }
}

/// Which product's rules the control and the tool follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SliceRules {
    /// Shift makes a new slice square and Alt draws it from the centre.
    pub modifiers: bool,
    /// Shift-click in Select adds or removes a slice from the selection.
    pub multi_select: bool,
}

impl SliceRules {
    /// Photoshop's: no modifiers, one selected slice.
    pub const PHOTOSHOP: SliceRules = SliceRules { modifiers: false, multi_select: false };
    /// Illustrator's: Shift square, Alt from the centre, Shift-click multi-select.
    pub const ILLUSTRATOR: SliceRules = SliceRules { modifiers: true, multi_select: true };
}

impl Default for SliceRules {
    fn default() -> Self {
        SliceRules::PHOTOSHOP
    }
}

/// A command the Slice tools, [`SliceBar`] and the keyboard ask the app for. The app owns the slices and the dialogs,
/// gates the commands with [`SliceCommand::enabled`] as command state, and runs them when they fire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui::contract::CommandSet)]
#[command(prefix = "sliceCommand")]
#[non_exhaustive]
pub enum SliceCommand {
    /// Delete the selected stored (user or layer) slice.
    #[command(label = "Delete")]
    Delete,
    /// Open [`SliceOptionsDialog`] for the selected slice.
    #[command(label = "Slice Options…")]
    Options,
    /// Make the selected auto or layer slice a user slice.
    #[command(label = "Promote")]
    Promote,
    /// Open [`DivideSlicesDialog`] for the selected slice.
    #[command(label = "Divide…")]
    Divide,
    /// Make slices from the guides.
    #[command(label = "Slices From Guides")]
    FromGuides,
}

impl SliceCommand {
    /// Whether the command is available: with `selected` the origin of the selected slice, `locked` for View › Lock
    /// Slices (every command is off) and `has_document` for an open document (Slices From Guides needs one).
    pub fn enabled(self, selected: Option<SliceOrigin>, locked: bool, has_document: bool) -> bool {
        if locked {
            return false;
        }
        match self {
            SliceCommand::Promote => matches!(selected, Some(SliceOrigin::Auto | SliceOrigin::Layer)),
            SliceCommand::Divide | SliceCommand::Options => selected.is_some(),
            SliceCommand::Delete => selected.is_some_and(SliceOrigin::stored),
            SliceCommand::FromGuides => has_document,
        }
    }
}

/// Why a press did nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SliceRefusal {
    /// View › Lock Slices is on: the app shows the locked words in its status bar.
    Locked,
}

/// What the control and the tool report.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SliceEvent {
    /// A new slice: `Begin` past the click slop (or at the press for Fixed Size), `Update`, `Commit` with the slice's
    /// pixel rectangle, `Cancel`.
    Draw(EditEvent<Rect>),
    /// The slice `number` (the whole slice, also when the press hit a covered piece of it) moving to a new rectangle.
    #[non_exhaustive]
    Move {
        /// The slice's display number.
        number: usize,
        /// The gesture and the new rectangle, in document pixels.
        event: EditEvent<Rect>,
    },
    /// The user slice `number` resized by a handle.
    #[non_exhaustive]
    Resize {
        /// The slice's display number.
        number: usize,
        /// The gesture and the new rectangle, in document pixels.
        event: EditEvent<Rect>,
    },
    /// The selection changed: `number` picked (`None`: nothing), `add` for a Shift-click that adds or removes it.
    #[non_exhaustive]
    Select {
        /// The picked slice's display number, or `None` for nothing.
        number: Option<usize>,
        /// Whether the pick adds to or removes from the selection.
        add: bool,
        /// The modifiers held at the click.
        mods: Mods,
    },
    /// A slice command the keyboard or a double click asked for.
    Command(SliceCommand),
    /// A press the control or the tool refused.
    Refused(SliceRefusal),
}

/// The Slice tool's persisted options: its Style, Width and Height (a marquee's [`Constraint`], pixels for Fixed
/// Size) and Hide Auto Slices.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct SliceToolSettings {
    /// The marquee-style constraint (Free, Ratio or Size).
    pub style: Constraint,
    /// Whether auto slices are hidden.
    pub hide_auto: bool,
}

impl SliceToolSettings {
    /// Settings with `style`, auto slices shown.
    pub fn new(style: Constraint) -> Self {
        SliceToolSettings { style, hide_auto: false }
    }
}

/// A drag no wider and no taller than this, in document pixels, is a click.
pub(crate) const CLICK_SLOP: f64 = 2.0;

/// The running gesture of a drag. It names the undo step with `label`, reports `Begin` before the first value, then
/// each changed value, and ends in a `Commit` or a `Cancel`. The control and the tool of Slice and Artboard share it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Run<T> {
    id: GestureId,
    label: Arc<str>,
    before: T,
    began: bool,
    last: Option<T>,
}

impl<T: Clone + PartialEq> Run<T> {
    pub(crate) fn new(id: GestureId, label: Arc<str>, before: T) -> Self {
        Run { id, label, before, began: false, last: None }
    }

    pub(crate) fn began(&self) -> bool {
        self.began
    }

    fn begin(&mut self, events: &mut Vec<EditEvent<T>>) {
        if !self.began {
            self.began = true;
            events.push(EditEvent::begin(self.id, Mixed::Same(self.before.clone()), Arc::clone(&self.label), None));
        }
    }

    /// `Begin` on the first call, then an `Update` when `value` differs from the last one.
    pub(crate) fn follow(&mut self, value: T) -> Vec<EditEvent<T>> {
        let mut events = Vec::new();
        self.begin(&mut events);
        if self.last.as_ref() != Some(&value) {
            events.push(EditEvent::update(self.id, value.clone()));
            self.last = Some(value);
        }
        events
    }

    /// The end with `value`: a `Begin` first when none was reported, then the `Commit`.
    pub(crate) fn commit(mut self, value: T) -> Vec<EditEvent<T>> {
        let mut events = Vec::new();
        self.begin(&mut events);
        events.push(EditEvent::commit(self.id, Mixed::Same(self.before), value));
        events
    }

    /// The end without a commit: a `Cancel` when the gesture had begun.
    pub(crate) fn cancel(self, reason: CancelReason) -> Option<EditEvent<T>> {
        self.began.then(|| EditEvent::cancel(self.id, reason, Some(Mixed::Same(self.before))))
    }

    /// A one-step change to `value`.
    pub(crate) fn set(label: Arc<str>, before: T, value: T) -> EditEvent<T> {
        EditEvent::set(Mixed::Same(before), value, label, None)
    }
}

/// The slice a Select press at `at` picks: the topmost stored slice containing the point, else the topmost auto slice
/// unless `hide_auto`. Edges are inclusive low and exclusive high.
pub(crate) fn slice_at(items: &[SliceItem], at: Point, hide_auto: bool) -> Option<&SliceItem> {
    let hit = |item: &&SliceItem| item.rect.x0 <= at.x && at.x < item.rect.x1 && item.rect.y0 <= at.y && at.y < item.rect.y1;
    let stored = items.iter().rev().filter(hit).find(|item| item.origin != SliceOrigin::Auto);
    stored.or_else(|| items.iter().rev().find(hit).filter(|_| !hide_auto))
}

/// An offset rounded to whole document pixels.
pub(crate) fn whole(by: Vec2) -> Vec2 {
    Vec2::new(by.x.round(), by.y.round())
}

/// The position of `handle` (−1, 0 or 1 per axis, as [`crate::geometry::slice::HANDLES`]) on `rect`.
pub(crate) fn handle_point(rect: Rect, handle: [i8; 2]) -> Point {
    let at = |lo: f64, hi: f64, h: i8| match h {
        -1 => lo,
        1 => hi,
        _ => (lo + hi) / 2.0,
    };
    Point::new(at(rect.x0, rect.x1, handle[0]), at(rect.y0, rect.y1, handle[1]))
}

/// The offset a rectangle dragged by `by` moves with after snapping: each pair of edges snaps per axis through `snap`
/// (the position along the axis the guide would run along, as `Snap::apply` takes it), and the nearer snap of the two
/// edges wins.
pub(crate) fn snapped_move(rect: Rect, by: Vec2, snap: &dyn Fn(Axis, f64) -> f64) -> Vec2 {
    let nearest = |along: Axis, lo: f64, hi: f64, by: f64| {
        let (a, b) = (lo + by, hi + by);
        let (da, db) = (snap(along, a) - a, snap(along, b) - b);
        by + if da.abs() <= db.abs() { da } else { db }
    };
    Vec2::new(nearest(Axis::Y, rect.x0, rect.x1, by.x), nearest(Axis::X, rect.y0, rect.y1, by.y))
}

/// The reason a gesture ends for the press cancel that ended its press.
pub(crate) fn cancel_reason(cancel: craft_behavior::PressCancel) -> CancelReason {
    match cancel {
        craft_behavior::PressCancel::Escape => CancelReason::Escape,
        craft_behavior::PressCancel::FocusLost => CancelReason::Blur,
        _ => CancelReason::Removed,
    }
}
