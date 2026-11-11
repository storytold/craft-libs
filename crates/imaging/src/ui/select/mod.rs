//! The marquee and lasso selection tools.
//!
//! Each tool is a [`CanvasTool`] on the canvas's `ToolHost`: it draws its outline while the user works and reports
//! [`SelectEvent`]s, which the app drains with `take_events` after `ToolHost::run` and turns into its own commands.
//! No tool changes the document.
//!
//! - [`MarqueeTool`]: the Rectangular, Elliptical, Single Row and Single Column Marquees.
//! - [`LassoTool`]: freehand, with Alt-latched straight segments.
//! - [`PolygonLassoTool`]: click by click, Shift at 45 degrees.
//! - [`MagneticLassoTool`]: an outline that snaps to edges the app's [`EdgeTracer`] finds.
//!
//! Every tool keeps its state in itself between passes and asks the app's [`SelectionProbe`] about the selection under
//! the pointer. The app wraps the tool and the probe in a [`SelectToolPass`] for each pass:
//!
//! ```ignore
//! let probe = DocSelection { doc: &doc };                  // the app's SelectionProbe
//! self.lasso.operation(bar.operation);                     // the options bar's mode
//! let mut pass = self.lasso.pass(&probe);
//! let _ = ToolHost::new().run(ui, &canvas, &mut pass);
//! for event in self.lasso.take_events() {
//!     match event {
//!         SelectEvent::Commit { shape, operation } => app.run_select(shape, operation), // one undo step
//!         SelectEvent::Deselect => app.run("select.deselect"),
//!         SelectEvent::DragSelection(edit) => app.drag_selection(edit),                // EditEvent<SelectionDrag>
//!         _ => {}
//!     }
//! }
//! ```
//!
//! Every tool has the setter `operation(..)`, and the marquee `kind(..)` and `constraint(..)`; each returns the tool.
//! The Polygonal Lasso takes the secondary button while its outline is open and retracts the last vertex on it, so
//! the app needs no call for the right click.
//!
//! A press inside the selection drags it instead of drawing: the outline with a plain press and the operation New,
//! the selected pixels with Cmd (Cmd+Alt a copy), a floating piece with a plain press. Click-driven tools (the
//! Polygonal Lasso) need Cmd for any of it, and never while an outline is open. A Cmd press outside the selection is
//! [`ToolAction::Ignored`](craft_ui::canvas::ToolAction) so the app's Move tool takes it.
//!
//! [`CanvasTool`]: craft_ui::canvas::CanvasTool

use craft_behavior::geometry::SelectionOperation;
use craft_behavior::{EditEvent, GestureId, PixelSize};
use craft_ui::canvas::{CanvasSpace, ToolContext};
use egui::Pos2;
use kurbo::{Point, Rect};

mod drag;
mod lasso;
mod magnetic;
mod marquee;
mod polygon;
#[cfg(feature = "screen-testing")]
pub(crate) mod stories;

pub use drag::{InsideDrag, PressRule, cursor_over, drag_kind};
pub use lasso::LassoTool;
pub use magnetic::{EdgeSearch, EdgeTracer, MagneticLassoTool, MagneticSettings};
pub use marquee::{MarqueeKind, MarqueeTool};
pub use polygon::PolygonLassoTool;

craft_ui::words! {
    /// The words of the selection tools.
    #[non_exhaustive]
    pub struct SelectWords {
        /// The accessible name of an outline being drawn.
        outline: "select.outline" = "Selection outline";
        /// The marquee readout's width row.
        width: "select.width" = "W:";
        /// The marquee readout's height row.
        height: "select.height" = "H:";
        /// The marquee's accessible value: its width and height, each already with its unit.
        size: template "select.size" = "{w} × {h}";
        /// The name of the undo step of a selection drag.
        move_selection: "select.moveSelection" = "Move Selection";
        /// The point count of an outline being drawn, for its accessible value.
        outline_points: count "select.outlinePoints" = "{n} point" | "{n} points";
    }
}

/// The selection a tool made, in document units. Anti-alias and Feather are the app's options and ride with its
/// command, not with the shape.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SelectionShape {
    /// A rectangle on whole pixels: the Rectangular Marquee, and the Single Row and Single Column Marquees' one-pixel
    /// line across the document.
    Rectangle(Rect),
    /// The ellipse inscribed in a rectangle on whole pixels: the Elliptical Marquee.
    Ellipse(Rect),
    /// A closed polygon of 3 or more points (the last joins the first): the Lasso, the Polygonal Lasso and the
    /// Magnetic Lasso.
    Polygon(Vec<Point>),
}

/// What a press inside the selection drags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DragKind {
    /// The outline alone.
    Outline,
    /// The selected pixels, or the floating piece: Cmd cuts them out first, Cmd+Alt (`copy`) lifts a copy, a floating
    /// piece moves with a plain press.
    #[non_exhaustive]
    Pixels {
        /// Lift a copy instead of cutting.
        copy: bool,
    },
}

impl DragKind {
    /// Drags the selected pixels, lifting a copy when `copy` is set.
    pub fn pixels(copy: bool) -> Self {
        Self::Pixels { copy }
    }
}

/// The value of a selection drag: what moves and by how much.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct SelectionDrag {
    /// What the drag moves.
    pub kind: DragKind,
    /// The offset from the press in whole document pixels (rounded, each within 10^7).
    pub delta: [i32; 2],
}

impl SelectionDrag {
    /// A drag of `kind` by `delta` whole document pixels.
    pub fn new(kind: DragKind, delta: [i32; 2]) -> Self {
        Self { kind, delta }
    }
}

/// What a selection tool reports. The app drains them after `ToolHost::run`, in order.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SelectEvent {
    /// An outline closed or a marquee was released: select `shape` with `operation` (latched at the press,
    /// `SelectionOperation::resolve`). One undo step.
    #[non_exhaustive]
    Commit {
        /// What to select.
        shape: SelectionShape,
        /// How it combines with the current selection.
        operation: SelectionOperation,
    },
    /// A click that drew nothing (a marquee released within its click slop): deselect. One undo step.
    Deselect,
    /// A press inside the selection dragged it: `Begin` at the press, `Update` as it moves, `Commit` at the release
    /// (one undo step; a delta of 0 is no change, where the app deselects, or drops a floating piece that never
    /// moved), `Cancel` on Escape.
    DragSelection(EditEvent<SelectionDrag>),
}

impl SelectEvent {
    /// Select `shape` with `operation`.
    pub fn commit(shape: SelectionShape, operation: SelectionOperation) -> Self {
        Self::Commit { shape, operation }
    }
}

/// What a selection tool is doing, for the status line and a test.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum SelectPhase {
    /// Nothing under way.
    #[default]
    Idle,
    /// A marquee being dragged: the rectangle the release would select (whole pixels), `None` while the drag is still
    /// a click.
    #[non_exhaustive]
    Marquee {
        /// The pixel rectangle.
        rect: Option<Rect>,
    },
    /// An outline being drawn with `points` points so far.
    #[non_exhaustive]
    Outline {
        /// The point count.
        points: usize,
    },
    /// The selection being dragged.
    Dragging(DragKind),
}

impl SelectPhase {
    /// A marquee being dragged, with the rectangle a release would select.
    pub fn marquee(rect: Option<Rect>) -> Self {
        Self::Marquee { rect }
    }

    /// An outline with `points` points so far.
    pub fn outline(points: usize) -> Self {
        Self::Outline { points }
    }
}

/// What a selection tool asks the app about the active document. The app implements it over its document for one
/// pass; the library never names a document type.
pub trait SelectionProbe {
    /// The active document's identity: an outline being drawn is dropped when it changes.
    fn document(&self) -> u64;

    /// The document's size in pixels: the Single Row and Single Column Marquees span it.
    fn size(&self) -> PixelSize;

    /// Whether anything is selected: Alt with nothing selected is not subtract.
    fn has_selection(&self) -> bool;

    /// Whether document point `at` is inside the selection (coverage above 0).
    fn inside_selection(&self, at: Point) -> bool;

    /// Whether a floating piece (pixels lifted by a selection drag, not yet dropped) is on the canvas. Default: none.
    fn floating(&self) -> bool {
        false
    }

    /// Whether `at` is on the floating piece where it is shown now (the app subtracts the piece's offset). A press
    /// there drags the piece; one in the margin the cut left starts a new selection. Default:
    /// [`inside_selection`](Self::inside_selection).
    fn on_floating(&self, at: Point) -> bool {
        self.inside_selection(at)
    }
}

/// A selection tool with this pass's [`SelectionProbe`]: the `CanvasTool` the app hands to `ToolHost::run`. Built
/// every pass; the tool keeps its state, the probe borrows the document.
///
/// ```
/// use craft_behavior::PixelSize;
/// use craft_imaging::ui::select::{LassoTool, SelectionProbe};
/// use craft_ui::canvas::CanvasTool;
/// use kurbo::Point;
///
/// struct Nothing;
/// impl SelectionProbe for Nothing {
///     fn document(&self) -> u64 { 1 }
///     fn size(&self) -> PixelSize { PixelSize::new(100, 100) }
///     fn has_selection(&self) -> bool { false }
///     fn inside_selection(&self, _: Point) -> bool { false }
/// }
///
/// let mut lasso = LassoTool::new();
/// let mut pass = lasso.pass(&Nothing);
/// let _: &mut dyn CanvasTool = &mut pass;
/// ```
pub struct SelectToolPass<'a, T> {
    tool: &'a mut T,
    probe: &'a dyn SelectionProbe,
}

impl<'a, T> SelectToolPass<'a, T> {
    /// `tool` asking `probe` this pass. Each tool also builds it as `tool.pass(probe)`.
    pub fn new(tool: &'a mut T, probe: &'a dyn SelectionProbe) -> Self {
        Self { tool, probe }
    }
}

/// The events a tool reports and the ids of its selection drags.
#[derive(Clone, Debug, Default)]
struct Report {
    events: Vec<SelectEvent>,
    next: GestureId,
}

impl Report {
    fn push(&mut self, event: SelectEvent) {
        self.events.push(event);
    }

    fn take(&mut self) -> Vec<SelectEvent> {
        std::mem::take(&mut self.events)
    }

    /// The id of the next selection drag.
    fn gesture(&mut self) -> GestureId {
        let following = self.next.next();
        std::mem::replace(&mut self.next, following)
    }
}

/// The pass's view of the document, to map screen points the host delivers to document units and back.
#[derive(Clone, Copy)]
struct Place(CanvasSpace);

impl Place {
    /// The place of this pass; `None` for a canvas without a view.
    fn of(cx: &ToolContext<'_>) -> Option<Place> {
        cx.space().map(Place)
    }

    /// The document point under screen position `pos`.
    fn doc(&self, pos: Pos2) -> Point {
        self.0.view.to_doc(pos)
    }

    /// Where the view draws document point `at`.
    fn screen(&self, at: Point) -> Pos2 {
        self.0.view.to_screen(at)
    }

    /// Screen points per document unit.
    fn zoom(&self) -> f64 {
        self.0.view.zoom
    }
}
