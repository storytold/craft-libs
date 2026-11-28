//! Cursor requests as plain data and the rule that picks the winner (8.5).

use emath::{Pos2, Vec2};
use std::borrow::Cow;

/// A system cursor: craft-behavior's copy of egui's set, converted by `runtime::egui::output`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[expect(missing_docs, reason = "the names are the system cursor names")]
pub enum CursorIcon {
    #[default]
    Default,
    None,
    ContextMenu,
    Help,
    PointingHand,
    Progress,
    Wait,
    Cell,
    Crosshair,
    Text,
    VerticalText,
    Alias,
    Copy,
    Move,
    NoDrop,
    NotAllowed,
    Grab,
    Grabbing,
    AllScroll,
    ResizeHorizontal,
    ResizeNeSw,
    ResizeNwSe,
    ResizeVertical,
    ResizeColumn,
    ResizeEast,
    ResizeSouthEast,
    ResizeSouth,
    ResizeSouthWest,
    ResizeWest,
    ResizeNorthWest,
    ResizeNorth,
    ResizeNorthEast,
    ZoomIn,
    ZoomOut,
}

/// An icon's egui-free key (5.2), so plain data such as [`PaintedCursor::Tool`] names icons without egui.
#[derive(Clone, Debug, PartialEq, Eq, Hash, craft_ui_macros::PlainData)]
pub struct IconId(Cow<'static, str>);

impl IconId {
    /// The icon named `name`.
    pub const fn new(name: &'static str) -> Self {
        IconId(Cow::Borrowed(name))
    }

    /// The name.
    pub fn name(&self) -> &str {
        &self.0
    }
}

impl From<Cow<'static, str>> for IconId {
    fn from(name: Cow<'static, str>) -> Self {
        IconId(name)
    }
}

/// A mark beside a painted cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CursorBadge {
    /// A plus.
    Add,
    /// A minus.
    Subtract,
    /// An intersection mark.
    Intersect,
    /// A copy mark.
    Copy,
    /// A not-allowed mark.
    NotAllowed,
    /// A pair of scissors: the pen's point that cuts a path.
    Scissors,
    /// A circle: the pen's point that closes a path.
    Close,
    /// An arrow: a move mark.
    Move,
    /// An arrowhead mark.
    Arrow,
    /// The colour-sampler ring with four ticks.
    Target,
    /// A continue mark.
    Continue,
    /// A count mark: add, move or delete.
    Count(CountMark),
    /// The corner-radius mark.
    Corner,
}

/// The kind of a [`CursorBadge::Count`] mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CountMark {
    /// A point is added.
    Add,
    /// A point is moved.
    Move,
    /// A point is deleted.
    Delete,
}

/// A cursor the overlay host paints. Geometry is in screen points around the pointer (a tool scales its brush
/// radius by the zoom it set); colours come from semantic tokens the overlay host picks.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PaintedCursor {
    /// A crosshair.
    Crosshair,
    /// A circle of `radius` points: a round brush tip at this zoom.
    #[non_exhaustive]
    Circle {
        /// The radius.
        radius: f32,
        /// A crosshair of [`CENTRE_ARM`](PaintedCursor::CENTRE_ARM) arms on the pointer.
        centre: bool,
        /// A mark centred on the pointer.
        badge: Option<CursorBadge>,
    },
    /// App shapes, as closed rings around the pointer: a shaped brush tip.
    #[non_exhaustive]
    Outline {
        /// The rings.
        rings: Vec<Vec<Pos2>>,
        /// A crosshair of [`CENTRE_ARM`](PaintedCursor::CENTRE_ARM) arms on the pointer.
        centre: bool,
        /// A mark centred on the pointer.
        badge: Option<CursorBadge>,
    },
    /// An open hand.
    Hand,
    /// A magnifier.
    #[non_exhaustive]
    Zoom {
        /// Zooming out.
        out: bool,
    },
    /// A tool's icon.
    #[non_exhaustive]
    Tool {
        /// The icon.
        icon: IconId,
        /// The icon point that sits on the pointer, in the icon's own 24-unit box.
        hotspot: Vec2,
        /// A mark beside it.
        badge: Option<CursorBadge>,
    },
    /// A system cursor with a mark.
    #[non_exhaustive]
    Badged {
        /// The system cursor.
        base: CursorIcon,
        /// The mark.
        badge: CursorBadge,
    },
    /// The rotate pointer: a short arc with an arrowhead at each end, bent round the corner that lies in screen
    /// direction `toward` from the pointer.
    #[non_exhaustive]
    Turn {
        /// The screen direction from the pointer to the corner's centre; the bisector into the box.
        toward: Vec2,
    },
    /// The free-distort pointer: an arrowhead with its tip on the pointer, pointing along `dir`.
    #[non_exhaustive]
    Distort {
        /// The screen direction the arrowhead points in.
        dir: Vec2,
    },
    /// The skew pointer: two opposed arrows beside each other along `dir`.
    #[non_exhaustive]
    Skew {
        /// The screen direction of the edge the arrows run along.
        dir: Vec2,
    },
    /// The type tool's pointer: an I-beam, over a path when `path`.
    #[non_exhaustive]
    TypeOn {
        /// The text goes on a path.
        path: bool,
    },
    /// A locator mark.
    Locator,
    /// An arrow with its tip on the hotspot: solid, or hollow when `hollow`.
    #[non_exhaustive]
    Arrow {
        /// The arrow is drawn hollow.
        hollow: bool,
        /// The art turns about its hotspot to point along this direction; `None` is the base art.
        toward: Option<Vec2>,
    },
    /// An I-beam across a path with a filled triangle toward each grabbable side.
    #[non_exhaustive]
    PathText {
        /// The start of the path can be grabbed.
        start: bool,
        /// The end of the path can be grabbed.
        end: bool,
    },
    /// A pointer with the tool's icon below and right of it: a crosshair (Perspective Crop), or an arrow with its tip
    /// on the hotspot (Paint Bucket, Ruler, the lassos).
    #[non_exhaustive]
    PointerTool {
        /// The pointer the icon rides beside.
        pointer: PointerGlyph,
        /// The icon.
        icon: IconId,
        /// A mark at the icon's lower right.
        badge: Option<CursorBadge>,
    },
    /// No cursor at all.
    Hidden,
    /// A plain I-beam, visible on light and dark.
    IBeam,
}

impl PaintedCursor {
    /// The arm of the centre mark of a circle or an outline, in points.
    pub const CENTRE_ARM: f32 = 3.0;

    /// A circle of `radius` points, with no centre mark and no badge.
    pub const fn circle(radius: f32) -> Self {
        PaintedCursor::Circle { radius, centre: false, badge: None }
    }

    /// App shapes as closed rings, with no centre mark and no badge.
    pub fn outline(rings: Vec<Vec<Pos2>>) -> Self {
        PaintedCursor::Outline { rings, centre: false, badge: None }
    }

    /// A circle or an outline with the centre mark when `centre` and `badge` centred on the pointer; any other cursor
    /// unchanged.
    #[must_use]
    pub fn marked(self, centre: bool, badge: Option<CursorBadge>) -> Self {
        match self {
            PaintedCursor::Circle { radius, .. } => PaintedCursor::Circle { radius, centre, badge },
            PaintedCursor::Outline { rings, .. } => PaintedCursor::Outline { rings, centre, badge },
            other => other,
        }
    }

    /// A magnifier, zooming out when `out`.
    pub const fn zoom(out: bool) -> Self {
        PaintedCursor::Zoom { out }
    }

    /// A tool's icon with its hotspot and an optional badge.
    pub const fn tool(icon: IconId, hotspot: Vec2, badge: Option<CursorBadge>) -> Self {
        PaintedCursor::Tool { icon, hotspot, badge }
    }

    /// The rotate pointer bent toward screen direction `toward`.
    pub const fn turn(toward: Vec2) -> Self {
        PaintedCursor::Turn { toward }
    }

    /// The free-distort pointer along screen direction `dir`.
    pub const fn distort(dir: Vec2) -> Self {
        PaintedCursor::Distort { dir }
    }

    /// The skew pointer along screen direction `dir`.
    pub const fn skew(dir: Vec2) -> Self {
        PaintedCursor::Skew { dir }
    }

    /// The type tool's pointer, over a path when `path`.
    pub const fn type_on(path: bool) -> Self {
        PaintedCursor::TypeOn { path }
    }

    /// An arrow with its tip on the hotspot, hollow when `hollow`.
    pub const fn arrow(hollow: bool) -> Self {
        PaintedCursor::Arrow { hollow, toward: None }
    }

    /// A solid arrow turned about its hotspot to point along `dir`.
    pub const fn arrow_toward(dir: Vec2) -> Self {
        PaintedCursor::Arrow { hollow: false, toward: Some(dir) }
    }

    /// An I-beam across the path with a triangle toward each grabbable side.
    pub const fn path_text(start: bool, end: bool) -> Self {
        PaintedCursor::PathText { start, end }
    }

    /// `pointer` with the tool's icon below and right of it.
    pub const fn pointer_tool(pointer: PointerGlyph, icon: IconId) -> Self {
        PaintedCursor::PointerTool { pointer, icon, badge: None }
    }

    /// `pointer` with the tool's icon below and right of it, and `badge` at the icon's lower right.
    pub const fn pointer_tool_badged(pointer: PointerGlyph, icon: IconId, badge: Option<CursorBadge>) -> Self {
        PaintedCursor::PointerTool { pointer, icon, badge }
    }

    /// A system cursor with a mark.
    pub const fn badged(base: CursorIcon, badge: CursorBadge) -> Self {
        PaintedCursor::Badged { base, badge }
    }
}

/// The pointer of a [`PaintedCursor::PointerTool`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PointerGlyph {
    /// A crosshair centred on the hotspot.
    Crosshair,
    /// The solid arrow with its tip on the hotspot.
    Arrow,
}

/// What a cursor request names.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Cursor {
    /// A system cursor.
    System(CursorIcon),
    /// A cursor the overlay host paints.
    Painted(PaintedCursor),
}

impl From<CursorIcon> for Cursor {
    fn from(icon: CursorIcon) -> Self {
        Cursor::System(icon)
    }
}

impl From<PaintedCursor> for Cursor {
    fn from(cursor: PaintedCursor) -> Self {
        Cursor::Painted(cursor)
    }
}

/// Who asks, in ascending order of precedence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum CursorPriority {
    /// The default.
    Default,
    /// The active tool.
    Tool,
    /// A hovered component.
    Hover,
    /// A hovered part's declared cursor.
    Part,
    /// A drop target during a drag.
    DropTarget,
    /// A drag in progress.
    Drag,
    /// Hides the pointer over everything, links included, while a presentation runs.
    Hidden,
}

/// The winning request: the highest priority, the later request on a tie.
pub fn pick(requests: &[(Cursor, CursorPriority)]) -> Option<&Cursor> {
    pick_index(requests).map(|at| &requests[at].0)
}

/// The index of the winning request ([`pick`]).
pub fn pick_index(requests: &[(Cursor, CursorPriority)]) -> Option<usize> {
    requests.iter().enumerate().max_by_key(|(order, (_, priority))| (*priority, *order)).map(|(at, _)| at)
}
