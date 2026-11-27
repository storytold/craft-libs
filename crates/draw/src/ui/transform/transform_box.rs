//! `TransformBox`: the box of Free Transform and of a Move tool's transform controls, a canvas control (20.6). Its
//! corners, edges, reference point, inside and outside are parts in the canvas's arena; the value is the box in
//! document units.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use craft_behavior::geometry::quad::{self, BoxAllow, BoxDrag, BoxPart, BoxRules, BoxState, ScaleRule, TransformMode};
use craft_behavior::geometry::{SNAP_PX, SnapKind, SnapKinds, SnapLine, SnapSuspend};
use craft_behavior::kurbo::{Point, Rect as DocRect, Vec2 as DocVec};
use craft_behavior::{
    Axis, CancelReason, CanvasView, Cursor, CursorIcon, CursorPriority, GesturePhase, HitLevel, HitPriority, HitShape, Intent, Metrics, NavShape,
    PaintedCursor, PartKey, PartRef, PointerButton, PressPhase, Roving, Sense, resize_cursor,
};
use craft_keymap::Mods;
use craft_ui::canvas::{CanvasSpace, GestureSnap, Snap, canvas_snap, canvas_space, readout};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Name, Outcome, PaintUi, PartsCx, Pointer as _,
    Props as _, Remember as _, Role, Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{Draw, Handle, PaintShape, StrokeColor, StrokeWidth};
use craft_ui::runtime::env::{Label, UnitKind};
use craft_ui::runtime::focus::{FocusShape, KeyUse};
use craft_ui::style::{ComponentStyle, DataColor, HandleLook, StyleColor, StyleCx};
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};

use crate::ui::corner::{CornerStyle, paint_widget};

/// The part of the box a handle, the inside or the reference point is: `BOX.at(part.index())`.
pub const BOX: PartKey = PartKey::new("box");

/// The parts that stand for the outside of the box near corner `i`: `OUTSIDE.at(i)` is the outside nearest that corner
/// in [`RotateZone::Outside`] and [`RotateZone::Border`] and the band at that corner in [`RotateZone::Band`]. One key
/// scheme in every zone, so a press that began on a band keeps its part when the box it hands off to takes the whole
/// outside.
const OUTSIDE: PartKey = PartKey::new("outside");

/// The round rotate handle of [`RotateZone::Knob`], `KNOB.at(0)`: it stands for the outside, so a drag on it turns the
/// box as a drag outside does.
const KNOB: PartKey = PartKey::new("knob");

/// The pointer-only part of an edge: the band along the whole side, outside it, beside the midpoint handle `BOX.at(Edge(k))` that the keyboard
/// and the focus ring use. One part key carries one shape, so the side cannot reuse the handle's key.
const EDGE: PartKey = PartKey::new("edge");

/// The pointer-only parts of a corner along its two sides: `CORNER_REACH.at(2 * k)` is the stretch of side `k`'s band
/// next to corner `k`, `CORNER_REACH.at(2 * k + 1)` that of the side before it. On the band a press near a corner takes
/// the corner, as PhotoCraft's box does, beside the corner's round handle `BOX.at(Corner(k))`.
const CORNER_REACH: PartKey = PartKey::new("cornerReach");

/// The pointer-only move part at the centre of a box whose reference point is hidden.
const CENTRE: PartKey = PartKey::new("centre");

/// The pointer-only parts of the move reach outside the frame: `MOVE_RING.at(k)` for `k` below 4 is the strip along
/// side `k`, and `MOVE_RING.at(4 + i)` the round sector outside corner `i`.
const MOVE_RING: PartKey = PartKey::new("moveRing");

/// The segments of the arc of each round corner of the move reach.
const RING_ARC_STEPS: usize = 8;

/// The rank of a side among the handles: below corners, midpoints, the reference point and the centre, so a corner
/// wins where a corner and a side both reach, and the nearest of the others wins among themselves.
const EDGE_SUB: u8 = 0;
const HANDLE_SUB: u8 = 1;
/// The rank of the move strips among the body-level parts: above the rotate bands and the inside, below every handle.
const RING_SUB: u8 = 1;

/// The keyboard paths of the pointer-only parts: the midpoint handle, and the arrow keys.
const EDGE_BY_KEY: &str = "the edge's midpoint handle";
const CORNER_BY_KEY: &str = "the corner's handle";
const MOVE_BY_KEY: &str = "the arrow keys";

/// The side of the enlarged corner handle of an overflowing text frame, in standard handle sides.
const OVERFLOW_IN_HANDLES: f32 = 1.5;

/// The keyboard path of a rotation by the outside parts: the options bar's angle field, which the host shows beside the box.
const ROTATE_BY_KEY: &str = "the angle field in the options bar";

/// The rotate handle of [`RotateZone::Handle`]: the zone's one rotating part, a knob on a stem off the middle of side 0.
const ROTATE_HANDLE: PartKey = PartKey::new("rotateHandle");

/// The length of the rotate handle's stem, from the side to the knob's centre, in standard handle sides.
const STEM_IN_HANDLES: f32 = 3.0;

/// The ends of a line in [`LineEnds::Free`]: `LINE_END.at(0)` its start, `LINE_END.at(1)` its end.
const LINE_END: PartKey = PartKey::new("lineEnd");

/// The keyboard path of a free line end: the host commits the ends, through its own fields for them.
const LINE_END_BY_KEY: &str = "the line's end fields in the options bar";

/// The part the keyboard picked, as the box remembers it between passes.
#[derive(Clone, Copy, Default)]
struct KeyPicked(Option<BoxPart>);

/// The distance an arrow key moves a part, in document units; Shift moves it by `NUDGE_FAR`.
const NUDGE: f64 = 1.0;
const NUDGE_FAR: f64 = 10.0;

/// The pointer distances, in standard handle sides (`size.handle.md`; at an 8 handle: a handle reaches 8 from its
/// point, inside the box as outside, each side's band reaches 11 outside it and none inside, and the band's first
/// quarter of the side from each corner, at most 11 along it, is that corner's; the reference point and the centre
/// reach 12, the rotate pointer shows within 36 of a corner, the rotate band runs from 5 to 18 outside it). The handle
/// reach, the side bands and the corners' share of them are PhotoCraft's: inside the box only a handle's own reach is the box's, so a
/// press there off the handles reaches what lies under the box (a text editor under a paragraph frame).
const HANDLE_REACH_IN_HANDLES: f32 = 1.0;
const SIDE_BAND_IN_HANDLES: f32 = 1.375;
const HIT_IN_HANDLES: f32 = 1.5;
const ROTATE_REACH_IN_HANDLES: f32 = 4.5;
const BAND_INNER_IN_HANDLES: f32 = 0.625;
const BAND_OUTER_IN_HANDLES: f32 = 2.25;

/// Where a press outside the box rotates it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum RotateZone {
    /// Anywhere outside (PhotoCraft's Free Transform); the rotate pointer shows within the rotate reach of a corner.
    #[default]
    Outside,
    /// Only the band outside a corner (PhotoCraft's Move tool controls); the rest of the outside is the tool's.
    Band,
    /// A round handle on a stem above the box's top side, or below its bottom side when the canvas has no room above.
    /// It rotates like the outside under the grab pointer (grabbing while it turns), and no outside region is declared.
    Knob,
    /// Only within `points` screen points of the box's sides (PhotoCraft's type box with ⌘ held). The whole outside is
    /// still the box's: a press further out starts nothing and applies nothing, and reaches neither the content under
    /// the box nor the tool, and the pointer there is the arrow.
    #[non_exhaustive]
    Border {
        /// The reach outside the sides, in screen points.
        points: f32,
    },
    /// Only the rotate handle on its stem (Office's rotate handle); Alt is not the pivot key when snapping suspends on
    /// Alt.
    Handle,
}

impl RotateZone {
    /// The zone within `points` screen points of the box's sides.
    pub const fn border(points: f32) -> Self {
        RotateZone::Border { points }
    }

    /// Whether a press on the outside at `pointer`, on the screen, rotates the box whose corners are `quad` on the
    /// screen.
    fn rotates_at(self, quad: &[Point; 4], pointer: Pos2) -> bool {
        match self {
            RotateZone::Border { points } => border_distance(quad, pointer) <= points,
            RotateZone::Outside | RotateZone::Band | RotateZone::Knob => true,
            // The handle is the zone's only outside part, so a press that reaches the outside is on it.
            RotateZone::Handle => true,
        }
    }
}

/// What of the box takes the pointer and the keys ([`TransformOverlay::reach`](super::TransformOverlay::reach)).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum TransformReach {
    /// The whole box: its handles, sides, inside and outside, drawn, with its keys and its menu (Free Transform).
    #[default]
    Box,
    /// Only the move reach: a plain press within `points` screen points outside the frame moves the box. No handle,
    /// side, inside or outside takes a press, the box takes no key and has no menu, and it is drawn only while its
    /// drag runs. PhotoCraft's type box while typing with no modifier held: a press on the text is the editor's, one
    /// further out the tool's.
    MoveOnly {
        /// The reach outside the frame, in screen points.
        points: f32,
    },
    /// The box paints its frame and handles but declares no part, takes no key and has no menu: every press reaches
    /// what lies under it, the tool.
    Shown,
}

/// What a line's two end controls are ([`LineLook::ends`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum LineEnds {
    /// Each end stands for a handle of the box aligned with the segment.
    #[default]
    Box,
    /// Each end moves as a free point, reported for the host to commit.
    Free,
}

/// Which end of a line is dragged ([`LineEndDrag::end`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LineEnd {
    /// The line's start.
    From,
    /// The line's end.
    To,
}

/// A drag of one end of a line in [`LineEnds::Free`], for the host to commit.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct LineEndDrag {
    /// Where the drag stands in this pass.
    pub phase: GesturePhase,
    /// The end dragged.
    pub end: LineEnd,
    /// Where the end is, in document units.
    pub at: Point,
    /// The modifier keys held.
    pub mods: Mods,
}

/// The box drawn as a live line's controls ([`TransformBox::line`]), in document units: a square at each end of the
/// segment and the round widget at its middle, with no frame. `outline`, for a line with an arrowhead, is the box
/// aligned with the segment that holds the arrowheads, drawn with a square at the middle of each side.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct LineLook {
    /// The line's start.
    pub from: Point,
    /// The line's end.
    pub to: Point,
    /// The box aligned with the segment, corner by corner around it.
    pub outline: Option<[Point; 4]>,
    /// What the two ends are: box handles, or free points the host commits.
    pub ends: LineEnds,
}

impl LineEnd {
    /// The repeat of [`LINE_END`] the end is declared under.
    const fn index(self) -> u64 {
        match self {
            LineEnd::From => 0,
            LineEnd::To => 1,
        }
    }
}

impl LineLook {
    /// The controls of the line from `from` to `to`, with no outline, each end a box handle.
    pub fn new(from: Point, to: Point) -> Self {
        LineLook { from, to, outline: None, ends: LineEnds::Box }
    }

    /// These controls with the segment-aligned box `outline`.
    #[must_use]
    pub fn with_outline(mut self, outline: Option<[Point; 4]>) -> Self {
        self.outline = outline;
        self
    }

    /// These controls with `ends`: whether the ends are box handles or free points reported for the host to commit.
    #[must_use]
    pub fn ends(mut self, ends: LineEnds) -> Self {
        self.ends = ends;
        self
    }

    /// Each end that stands for a box handle, with the handle: the corner the end points to, away from the other end
    /// (on a level or upright line, the side). A line of no length, or one with free ends, has none.
    fn end_parts(&self) -> impl Iterator<Item = (BoxPart, Point)> {
        let boxed = self.ends == LineEnds::Box;
        [(self.from, self.to), (self.to, self.from)].into_iter().filter(move |_| boxed).filter_map(|(end, other)| Some((end_part(end, other)?, end)))
    }

    /// Each free end with its point, in [`LineEnds::Free`]; none when the ends are box handles.
    fn free_ends(&self) -> impl Iterator<Item = (LineEnd, Point)> {
        let free = self.ends == LineEnds::Free;
        [(LineEnd::From, self.from), (LineEnd::To, self.to)].into_iter().filter(move |_| free)
    }
}

craft_ui::words! {
    /// The names of the box and its parts, and the labels of its readout.
    pub struct TransformBoxWords {
        group: "transform.group" = "Free Transform";
        whole: "transform.whole" = "Whole box";
        top_left_corner: "transform.topLeftCorner" = "Top left corner";
        top_right_corner: "transform.topRightCorner" = "Top right corner";
        bottom_right_corner: "transform.bottomRightCorner" = "Bottom right corner";
        bottom_left_corner: "transform.bottomLeftCorner" = "Bottom left corner";
        top_edge: "transform.topEdge" = "Top edge";
        right_edge: "transform.rightEdge" = "Right edge";
        bottom_edge: "transform.bottomEdge" = "Bottom edge";
        left_edge: "transform.leftEdge" = "Left edge";
        pivot: "transform.pivot" = "Reference point";
        rotate_knob: "transform.rotateKnob" = "Rotate";
        readout_angle: "transform.readoutAngle" = "Angle:";
        readout_width: "transform.readoutWidth" = "W:";
        readout_height: "transform.readoutHeight" = "H:";
        readout_dx: "transform.readoutDx" = "ΔX:";
        readout_dy: "transform.readoutDy" = "ΔY:";
    }
}

impl TransformBoxWords {
    /// The name of `part` in the accessibility tree.
    fn name_of(&self, part: BoxPart) -> &str {
        match part {
            BoxPart::Corner(0) => self.top_left_corner.as_str(),
            BoxPart::Corner(1) => self.top_right_corner.as_str(),
            BoxPart::Corner(2) => self.bottom_right_corner.as_str(),
            BoxPart::Corner(_) => self.bottom_left_corner.as_str(),
            BoxPart::Edge(0) => self.top_edge.as_str(),
            BoxPart::Edge(1) => self.right_edge.as_str(),
            BoxPart::Edge(2) => self.bottom_edge.as_str(),
            BoxPart::Edge(_) => self.left_edge.as_str(),
            BoxPart::Pivot => self.pivot.as_str(),
            _ => self.whole.as_str(),
        }
    }
}

/// The style of the transform box.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TransformBoxStyle {
    #[style(metrics)]
    metrics: Metrics,
    handle: HandleLook,
    line: StyleColor,
    halo: StyleColor,
    /// The hit radius of a corner or side handle.
    handle_reach: f32,
    /// How far a side's band reaches outside the box.
    side_band: f32,
    /// The hit radius of the reference point and of the centre.
    hit: f32,
    /// How far from a corner the rotate pointer shows outside the box.
    rotate_reach: f32,
    band_inner: f32,
    band_outer: f32,
    pivot_radius: f32,
    pivot_arm: f32,
    /// The rotate handle's stem, from the side to the knob's centre.
    rotate_stem: f32,
    /// The radius of the rotate handle's knob.
    rotate_knob: f32,
    /// The round widget at a live line's middle: the corner widgets' look.
    widget: CornerStyle,
}

/// Why the box asks to apply.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ApplyBy {
    /// Enter.
    Key,
    /// A double click inside the box.
    DoubleClick,
    /// A click outside the box, at `at` in document units.
    #[non_exhaustive]
    ClickOutside {
        /// Where the click landed.
        at: Point,
    },
}

/// What the box reports beyond its edit, from this pass's input only.
#[derive(Clone, Debug, Default)]
struct Report {
    apply: Option<ApplyBy>,
    clicked: Option<(BoxPart, Point)>,
    discard: bool,
    hovered: Option<BoxPart>,
    picked: Option<BoxPart>,
    held: Option<BoxPart>,
    snapped: Vec<SnapLine>,
    line_end: Option<LineEndDrag>,
}

/// The press the box is dragging a part with, kept from the press to its release.
#[derive(Clone, Copy, Debug)]
struct Grip {
    part: BoxPart,
    /// The box the drag works from: the box at the press, its reference point moved by an Alt press.
    start: BoxState,
    /// The press in document space.
    from: Point,
    began: bool,
    /// The pressed part follows the pointer inside the slop ([`follows_in_slop`]).
    follows: bool,
    /// An Alt press away from the handles: a click there moves the reference point.
    moves_pivot: bool,
    /// The box at the press.
    pressed: BoxState,
    /// The box the drag last produced.
    last: BoxState,
}

impl Default for Grip {
    fn default() -> Self {
        let none = BoxState::from_rect(DocRect::ZERO);
        Grip { part: BoxPart::Inside, start: none, from: Point::ZERO, began: false, follows: false, moves_pivot: false, pressed: none, last: none }
    }
}

/// The press dragging a free line end, kept from the press to its release.
#[derive(Clone, Copy, Debug)]
struct EndGrip {
    end: LineEnd,
    /// From the press to the end, in document units: the end keeps it, so a press off the end's point does not jump.
    offset: DocVec,
    began: bool,
}

impl Default for EndGrip {
    fn default() -> Self {
        EndGrip { end: LineEnd::From, offset: DocVec::ZERO, began: false }
    }
}

/// Whether a press on `part` moves the box from the first pointer move inside the slop, so the box does not jump when
/// the drag is recognised: every part but the outside, where a click applies the box. The parts' declared sense
/// ([`slop_sense`]) and the grip both read it.
fn follows_in_slop(part: BoxPart) -> bool {
    part != BoxPart::Outside
}

/// [`Sense::FOLLOWS_IN_SLOP`] for a part that [`follows_in_slop`].
fn slop_sense(part: BoxPart) -> Sense {
    if follows_in_slop(part) { Sense::FOLLOWS_IN_SLOP } else { Sense::empty() }
}

/// The box of Free Transform: eight handles and a reference point over the host's content, dragged through
/// [`quad::drag`]'s rules, moved and picked from the keyboard.
///
/// - The value is the box in document units. A drag is one gesture (`Begin` on the first move, also inside the slop
///   on every part but the outside, `Commit` on the release); Escape cancels it, as does a click that ends with the
///   box as it was at the press. The host applies the box on [`TransformBoxOutput::apply`] (Enter, a click outside,
///   a double click inside) and closes it on [`discard`](TransformBoxOutput::discard).
/// - The pointer takes the nearest handle or the reference point, else a side (the whole side along its length, a
///   corner winning where both reach), the inside or the outside. Parts the [rules](Self::rules) refuse take no
///   press, except the outside in [`RotateZone::Outside`] and within the reach of [`RotateZone::Border`], where a
///   click applies. An Alt press away from the handles
///   moves the reference point there and drags it, whether it shows or not.
/// - Snapping is the canvas's ([`canvas_snap`]): a dragged handle snaps through [`Snap::point`] and a box moved from
///   inside snaps its bounds through [`Snap::rect_of`]; the lines the result reaches are
///   [`TransformBoxOutput::snapped`] for the host's one drawing site. Without a canvas snap nothing snaps but the
///   reference point, which snaps to the box's nine points only. In Distort only the corners snap, to guides. A pass
///   reports the lines of its own drag events only.
/// - The box is one Tab stop with one part group, whose arrival is the picked part. Page Down and Page Up pick the
///   next and previous part (the whole box, the corners and edges clockwise from the top left, the reference
///   point), Home and End the first and last. The arrows move the picked part as a plain drag of it would, one
///   document unit, Shift ten. A part focus reached by a press (the handle a drag ended on) is not picked: the
///   arrows then move the whole box.
/// - The box holds the canvas's session (`KeyUse::session`): with the canvas focused or nothing focused, Enter, Escape
///   and the arrows reach it before the canvas's tool, and the arrows move the whole box. With
///   [`session_arrows`](Self::session_arrows) off the arrows stay the tool's (the Move tool nudges the layer); with
///   [`session_commit`](Self::session_commit) off Enter and Escape stay the scope's around the canvas (a dialog's OK
///   and Cancel). A [`reach`](Self::reach) of [`TransformReach::Shown`] takes no press and no key at all.
/// - A [centre mark](Self::centre_mark) draws the reference point's glyph at the middle of the box and takes no press.
///   A [live line's look](Self::line) declares only the line's two ends, each standing for the box handle it points
///   to, and draws the line's controls in place of the frame. With [free ends](LineEnds::Free) each end is a point of
///   its own: its drag reports [`TransformBoxOutput::line_end`] for the host to commit and leaves the box alone.
/// - In [`RotateZone::Handle`] only the rotate handle, a knob on a stem off the middle of the top side, rotates, and
///   the rest of the outside is the tool's. While Alt is the canvas's snap suspend key, the drag never reads Alt.
/// - Nothing mirrors in a right-to-left interface: the box is the document's.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control)]
pub struct TransformBox<'a> {
    #[common]
    common: Common,
    state: Bind<'a, BoxState>,
    start: DocRect,
    skew_v: f64,
    /// The mode, scaling rule, aspect link, what the content allows and the rotate step.
    #[prop]
    rules: BoxRules,
    /// Whether the reference point shows. Hidden, it is not hit or picked and is drawn only while it is dragged; an
    /// Alt press away from the handles still moves it, and its stored position pivots Alt scaling and rotating. A
    /// hidden point leaves the middle of the box to a move part, so a small box stays movable.
    #[prop]
    reference_point: bool,
    /// Where a press outside rotates.
    #[prop]
    rotate_zone: RotateZone,
    /// Whether a press inside moves the box. Off leaves it to the tool.
    #[prop]
    inside_moves: bool,
    /// The modifiers every test of this box reads in place of the pointer's: a touch host's sticky modifiers.
    #[prop]
    mods: Option<Mods>,
    /// Enlarges corner 2 with the overflow plus (a text frame whose text does not fit).
    #[prop]
    overflow: bool,
    /// Whether the arrows the box takes from the canvas's session, while none of its parts has focus, move the whole
    /// box (the default). Off leaves them to the canvas's tool; a focused part still moves by the arrows.
    #[prop]
    session_arrows: bool,
    /// Whether a centre mark (the reference point's glyph) shows at the middle of the box, under the handles: the
    /// Ellipse tool's. It takes no press: a press there is the inside's, or the tool's.
    #[prop]
    centre_mark: bool,
    /// Whether the corner and side handles are circles (a rounded document editor's look) instead of squares.
    #[prop]
    round_handles: bool,
    /// The side of the corner and side handles in points, for a host with a handle size preference; `None` uses the
    /// theme's (`size.handle.md`). A handle's reach grows with a larger side and stays at least the theme's.
    #[prop(quantity = "points, from the host's handle size preference")]
    handle_side: Option<f32>,
    /// Draws the box as a live line's controls (the Line tool's live line as the whole selection). Only the two ends
    /// take a press and the keys, each as the box handle it points to ([`LineLook`]), so an end drag scales the box as
    /// that handle does; while it is dragged, the box shows as it scales. No other part is declared.
    #[prop]
    line: Option<LineLook>,
    /// What of the box takes the pointer and the keys: the whole box, only the move reach outside the frame, or
    /// nothing.
    #[prop]
    reach: TransformReach,
    /// The frame and handle edges in the host's colour, else `overlay.line`.
    #[prop]
    color: Option<DataColor>,
    /// Whether Enter and Escape from the canvas session are the box's (apply and discard). Off, they stay untaken so
    /// they reach the scope around the canvas (a dialog's OK and Cancel); arrows, Page keys and drags are unchanged,
    /// and an open drag's Escape still cancels the drag.
    #[prop]
    session_commit: bool,
    /// The caller hangs a context menu of its own on the box (`Menu::context` over its response): every part takes the
    /// right click, so it does not reach the tool under the box.
    #[prop]
    own_context_menu: bool,
    /// Whether this pass holds a drag of the reference point, so the hidden point is drawn while it moves.
    pivot_held: bool,
    /// Whether this pass holds a drag that began, so a live line's box shows as it scales.
    dragging: bool,
    report: Report,
}

impl<'a> TransformBox<'a> {
    /// A box named `label` over `state`, a box in document units. `start` is the rect before the transform and
    /// `skew_v` the vertical skew the host keeps in degrees: the readout reads the width and height against them.
    pub fn new(label: impl Into<Label>, state: Bind<'a, BoxState>, start: DocRect, skew_v: f64) -> Self {
        TransformBox {
            common: Common::new(),
            state,
            start,
            skew_v,
            rules: BoxRules::default(),
            reference_point: true,
            rotate_zone: RotateZone::Outside,
            inside_moves: true,
            mods: None,
            overflow: false,
            session_arrows: true,
            centre_mark: false,
            round_handles: false,
            handle_side: None,
            line: None,
            reach: TransformReach::Box,
            color: None,
            session_commit: true,
            own_context_menu: false,
            pivot_held: false,
            dragging: false,
            report: Report::default(),
        }
        .label(label)
    }

    /// A resize-only box for a text frame: free scaling with the aspect unlinked, sides of at least 4 units, no
    /// rotating, skewing or distorting, no warp, no reference point and no inside move. The rotate band is declared
    /// without rotation, so a press outside the frame reaches the tool and the text editor's background commit.
    pub fn text_frame(label: impl Into<Label>, state: Bind<'a, BoxState>, start: DocRect) -> Self {
        let rules = BoxRules::default()
            .with_scale(ScaleRule::Free)
            .with_aspect_linked(false)
            .with_allow(BoxAllow::new(false, false, false).with_warp(false))
            .with_min_side(4.0);
        Self::new(label, state, start, 0.0).rules(rules).reference_point(false).inside_moves(false).rotate_zone(RotateZone::Band)
    }

    /// The width of the move strips outside the frame when the box offers only them; `None` for the whole box.
    fn move_only(&self) -> Option<f32> {
        match self.reach {
            TransformReach::MoveOnly { points } => Some(points),
            TransformReach::Box | TransformReach::Shown => None,
        }
    }

    /// Whether the box takes any press or key: a shown box only paints, so what lies under it keeps every press.
    fn takes_input(&self) -> bool {
        self.reach != TransformReach::Shown
    }

    /// The colour of a stroke that is `theme` unless the host gave the box its own.
    fn ink(&self, theme: StyleColor) -> StrokeColor {
        self.color.map_or(StrokeColor::from(theme), StrokeColor::from)
    }

    /// The side of a corner or side handle: the host's, else the theme's.
    fn side_of_handles(&self, style: &TransformBoxStyle) -> f32 {
        self.handle_side.unwrap_or(style.handle.side)
    }

    /// The hit radius of a corner or side handle: the theme's, or more for a host's larger handle.
    fn reach_of_handles(&self, style: &TransformBoxStyle) -> f32 {
        self.handle_side.map_or(style.handle_reach, |side| style.handle_reach.max(side * HANDLE_REACH_IN_HANDLES))
    }

    /// Whether the middle of the box is a move part: the reference point is hidden, so it does not take that place.
    fn declares_centre(&self) -> bool {
        self.takes_input() && self.move_only().is_none() && !self.reference_point && self.inside_moves && self.rules.allows(BoxPart::Inside)
    }

    /// The width of the strips outside the frame, when they are move parts.
    fn ring_reach(&self) -> Option<f32> {
        self.move_only().filter(|&points| points > 0.0 && self.rules.allows(BoxPart::Inside))
    }

    /// Whether the box declares, draws and names a rotate knob: the knob zone on a box drawn as a box, where the rules
    /// let it turn.
    fn has_knob(&self) -> bool {
        matches!(self.rotate_zone, RotateZone::Knob) && self.line.is_none() && self.move_only().is_none() && self.rules.allows(BoxPart::Outside)
    }

    /// The parts the keyboard walks, in order. A box whose inside does not move has no inside part, so a press or drag
    /// over its body reaches what lies beneath (a type editor under a paragraph frame). A box that offers only its
    /// move reach or only a display has none.
    fn keyboard_parts(&self) -> Vec<BoxPart> {
        if self.move_only().is_some() || !self.takes_input() {
            return Vec::new();
        }
        if let Some(line) = &self.line {
            return line.end_parts().map(|(part, _)| part).filter(|&part| self.rules.allows(part)).collect();
        }
        let corners = (0..4u8).map(BoxPart::Corner);
        let edges = (0..4u8).map(BoxPart::Edge);
        let pivot = self.reference_point.then_some(BoxPart::Pivot);
        let inside = self.inside_moves.then_some(BoxPart::Inside);
        inside.into_iter().chain(corners).chain(edges).chain(pivot).filter(|&part| self.rules.allows(part)).collect()
    }

    /// Where `part` is drawn and taken, in document units: a live line's end, else [`handle_point`].
    fn part_point(&self, state: &BoxState, part: BoxPart) -> Option<Point> {
        match &self.line {
            Some(line) => line.end_parts().find(|(end_part, _)| *end_part == part).map(|(_, end)| end),
            None => handle_point(state, part),
        }
    }

    /// The box after an arrow key moves `part` by `by`: as a plain drag of the part would.
    fn nudged(&self, state: &BoxState, part: BoxPart, by: DocVec) -> BoxState {
        let at = match part {
            BoxPart::Corner(i) => state.quad[usize::from(i % 4)],
            BoxPart::Edge(i) => state.quad[usize::from(i % 4)].midpoint(state.quad[usize::from(i + 1) % 4]),
            BoxPart::Pivot => state.pivot,
            _ => return state.moved(by),
        };
        quad::drag(state, state, part, at, at + by, Mods::NONE, &self.rules)
    }

    /// The box the pointer at screen position `pos` gives a drag, snapped and rounded, with the lines it snapped to.
    ///
    /// - A corner or an edge snaps the grabbed handle through the canvas's `snap`, not the pointer: a press a few
    ///   pixels off the handle still lands the handle on the line. A distorting corner reports only the lines it
    ///   reaches once the box's folding rule has placed it: a corner stopped short of a line claims no alignment.
    /// - A drag from inside snaps the bounds of the moved box (its edges and centre).
    /// - The reference point snaps to the nine points of the box at the press (corners, edge midpoints, centre) and
    ///   to nothing else, with or without the canvas's snap, the Control key held or not.
    /// - In [`TransformMode::Distort`] only the corners snap, and to guides only. The outside (rotating) never snaps.
    /// - The canvas's suspend key (Control by default) suspends the snapping unless the drag reads it
    ///   ([`reads_suspend_key`](Self::reads_suspend_key)). The snap reads the keys as held; the drag acts on them as
    ///   [`drag_mods`] leaves them, so with Alt as the suspend key Alt only suspends.
    fn proposal(&self, space: &CanvasSpace, snap: Option<&Snap>, grip: &Grip, pos: Pos2, mods: Mods) -> (BoxState, Vec<SnapLine>) {
        let to = space.view.to_doc(pos);
        let suspend = suspend_key(snap);
        let acting = drag_mods(mods, suspend);
        let kind = self.rules.drag_kind(grip.part, acting);
        let distort_mode = self.rules.effective_mode() == TransformMode::Distort;
        let kinds = if distort_mode { SnapKinds::only(SnapKind::Guide) } else { SnapKinds::ALL };
        let gesture = GestureSnap::default().reads_suspend_key(self.reads_suspend_key(grip.part, acting, suspend)).kinds(kinds);
        let grabbed = handle_point(&grip.start, grip.part);
        let (to, mut lines) = match (grip.part, snap, grabbed) {
            (BoxPart::Pivot, _, Some(grabbed)) if !distort_mode => {
                let off = grip.from - grabbed;
                (box_point_pull(&grip.start, to - off, space.view.doc_len(SNAP_PX as f32)) + off, Vec::new())
            }
            (BoxPart::Corner(_), Some(snap), Some(grabbed)) | (BoxPart::Edge(_), Some(snap), Some(grabbed))
                if !distort_mode || matches!(grip.part, BoxPart::Corner(_)) =>
            {
                let off = grip.from - grabbed;
                let (at, lines) = snap.point(to - off, mods, gesture);
                (at + off, lines)
            }
            (BoxPart::Inside, Some(snap), _) if !distort_mode => {
                let moved = bounds_of(&grip.start.quad) + (to - grip.from);
                let (by, lines) = snap.rect_of(moved, mods, kinds);
                (to + by, lines)
            }
            _ => (to, Vec::new()),
        };
        let dragged = quad::drag(&grip.start, &grip.last, grip.part, grip.from, to, acting, &self.rules);
        // The canvas's pixel grid rounds an upright box and its corners to whole document units; keys are not rounded.
        let dragged = if snap.is_some_and(Snap::rounds_to_pixels) { on_pixel_grid(dragged) } else { dragged };
        if kind == Some(BoxDrag::Distort)
            && let Some(at) = handle_point(&dragged, grip.part)
        {
            lines.retain(|line| {
                let across = if matches!(line.along, Axis::Y) { at.x } else { at.y };
                (across - line.position).abs() < 1e-6
            });
        }
        (dragged, lines)
    }

    /// Whether a drag of `part` with `mods` (as the drag acts on them, [`drag_mods`]) reads the snap's `suspend` key
    /// itself, so the key does not suspend the snapping. Control: where ⌘ (Ctrl off macOS) changes what the drag
    /// does (a corner distorts, an edge skews) or on a corner that distorts. Alt: never, since the drag does not
    /// see Alt while Alt is the suspend key.
    fn reads_suspend_key(&self, part: BoxPart, mods: Mods, suspend: SnapSuspend) -> bool {
        match suspend {
            SnapSuspend::Alt => false,
            _ => {
                let kind = self.rules.drag_kind(part, mods);
                mods.cmd && (kind != self.rules.drag_kind(part, Mods { cmd: false, ..mods }) || kind == Some(BoxDrag::Distort))
            }
        }
    }

    /// The pointer for `part` at `pointer`, `dragging` during its drag.
    fn cursor(&self, look: &CursorLook<'_>, part: BoxPart, pointer: Pos2, mods: Mods, dragging: bool) -> Cursor {
        let Some(kind) = self.rules.drag_kind(part, mods) else { return CursorIcon::Default.into() };
        let (screen, view, style) = (look.screen, &look.space.view, look.style);
        let (corner, away) = nearest_corner(screen, pointer);
        let turn = || Cursor::from(PaintedCursor::turn(quad::bisector(screen, corner)));
        let along = |from: usize| vec2_of(screen.quad[(from + 1) % 4] - screen.quad[from % 4]);
        match (kind, part) {
            // The knob is a handle the pointer takes hold of and swings, not a stretch of the outside.
            (BoxDrag::Rotate, BoxPart::Outside) if self.rotate_zone == RotateZone::Knob => {
                if dragging { CursorIcon::Grabbing } else { CursorIcon::Grab }.into()
            }
            // The rotate handle turns the cursor with the box: it points in across the side the handle stands on.
            (BoxDrag::Rotate, BoxPart::Outside) if self.rotate_zone == RotateZone::Handle => {
                Cursor::from(PaintedCursor::turn(-outward_normals(&screen.quad.map(pos))[0]))
            }
            (BoxDrag::Rotate, BoxPart::Outside) if !dragging && !self.rotate_zone.rotates_at(&screen.quad, pointer) => CursorIcon::Default.into(),
            (BoxDrag::Rotate, BoxPart::Outside) => {
                if dragging || matches!(self.rotate_zone, RotateZone::Band | RotateZone::Border { .. }) || away <= style.rotate_reach {
                    turn()
                } else {
                    CursorIcon::Alias.into()
                }
            }
            (BoxDrag::Rotate, _) if dragging || away <= style.rotate_reach => turn(),
            (BoxDrag::Move, _) => CursorIcon::Move.into(),
            (BoxDrag::Pivot, _) => CursorIcon::Crosshair.into(),
            (BoxDrag::Distort | BoxDrag::Perspective, BoxPart::Corner(i)) => PaintedCursor::distort(-quad::bisector(screen, usize::from(i))).into(),
            (BoxDrag::Skew | BoxDrag::SlideCorner, BoxPart::Edge(i) | BoxPart::Corner(i)) => PaintedCursor::skew(along(usize::from(i))).into(),
            (_, BoxPart::Corner(i)) => resize_cursor(view, view.unmap_vec(-quad::bisector(screen, usize::from(i)))).into(),
            (_, BoxPart::Edge(i)) => {
                let i = usize::from(i);
                let mid = |k: usize| screen.quad[k % 4].midpoint(screen.quad[(k + 1) % 4]);
                resize_cursor(view, view.unmap_vec(vec2_of(mid(i) - mid(i + 2)))).into()
            }
            _ => CursorIcon::Default.into(),
        }
    }

    /// The rows of the readout of a drag of `part`: the box `now` against the box `at_press`.
    fn readout_rows(&self, cx: &Cx<'_, Self>, space: &CanvasSpace, part: BoxPart, now: &BoxState, at_press: &BoxState) -> Vec<(Label, String)> {
        let words = cx.words::<TransformBoxWords>();
        let units = cx.env.units();
        let row = |label: &Label, value: String| (label.clone(), value);
        let rotating = part == BoxPart::Outside
            || (matches!(part, BoxPart::Corner(_) | BoxPart::Edge(_)) && self.rules.effective_mode() == TransformMode::Rotate);
        if rotating {
            return vec![row(&words.readout_angle, units.text(UnitKind::Degrees, fixed(now.angle(), 1), 1..=1))];
        }
        match part {
            BoxPart::Inside => {
                let moved = now.quad[0] - at_press.quad[0];
                let rulers = space.rulers();
                let (across_x, _) = rulers.across(Axis::Y);
                let (across_y, _) = rulers.across(Axis::X);
                vec![
                    row(&words.readout_dx, across_x.format_value(&units, fixed(moved.x, 4), 0.0)),
                    row(&words.readout_dy, across_y.format_value(&units, fixed(moved.y, 4), 0.0)),
                ]
            }
            BoxPart::Corner(_) | BoxPart::Edge(_) if now.is_parallelogram() => {
                let parts = now.parts(self.start, self.skew_v);
                let percent = |v: f64| units.text(UnitKind::Percent, fixed(v * 100.0, 1), 1..=1);
                // Size in the rulers' unit, as the move rows are; a percent ruler keeps the share of the start box.
                let rulers = space.rulers();
                let (across_w, _) = rulers.across(Axis::Y);
                let (across_h, _) = rulers.across(Axis::X);
                let width = if across_w.kind == UnitKind::Percent {
                    percent(parts.w)
                } else {
                    across_w.format_value(&units, fixed(parts.w * self.start.width(), 4), 0.0)
                };
                let height = if across_h.kind == UnitKind::Percent {
                    percent(parts.h)
                } else {
                    across_h.format_value(&units, fixed(parts.h * self.start.height(), 4), 0.0)
                };
                vec![row(&words.readout_width, width), row(&words.readout_height, height)]
            }
            _ => Vec::new(),
        }
    }

    /// The parts that take a press, with the box part each stands for. Mirrors what [`parts`](Component::parts)
    /// declares: the keyboard parts, a segment per allowed edge, the centre where declared, and the outside, the knob in
    /// [`RotateZone::Knob`] or the rotate handle; or only the move strips; or none for a shown box.
    fn pointer_parts(&self) -> Vec<(PartRef, BoxPart)> {
        if !self.takes_input() {
            return Vec::new();
        }
        if self.move_only().is_some() {
            let parts = if self.ring_reach().is_some() { 0..8 } else { 0..0 };
            return parts.map(|part| (MOVE_RING.at(part), BoxPart::Inside)).collect();
        }
        let mut parts: Vec<_> = self.keyboard_parts().into_iter().map(|part| (BOX.at(part.index()), part)).collect();
        if self.line.is_some() {
            return parts;
        }
        parts.extend((0..4u8).filter(|&k| self.rules.allows(BoxPart::Edge(k))).map(|k| (EDGE.at(u64::from(k)), BoxPart::Edge(k))));
        parts.extend(
            (0..4u8)
                .filter(|&k| self.rules.allows(BoxPart::Corner(k)))
                .flat_map(|k| [0, 1].map(|end| (CORNER_REACH.at(2 * u64::from(k) + end), BoxPart::Corner(k)))),
        );
        if self.declares_centre() {
            parts.push((CENTRE.at(0), BoxPart::Inside));
        }
        if matches!(self.rotate_zone, RotateZone::Knob) {
            if self.has_knob() {
                parts.push((KNOB.at(0), BoxPart::Outside));
            }
        } else {
            parts.extend((0..4).map(|corner| (OUTSIDE.at(corner), BoxPart::Outside)));
            parts.push((ROTATE_HANDLE.at(0), BoxPart::Outside));
        }
        parts
    }

    /// The pointer on the parts: presses, drags, releases and the cursor.
    fn run_pointer(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, shown: &BoxState) {
        let screen = on_screen(&space.view, shown);
        let style = cx.style;
        let mut held: Option<Grip> = *cx.press_slot::<Grip>();
        let snap = canvas_snap(&cx.env);
        let suspend = suspend_key(snap.as_ref());
        for (reference, part) in self.pointer_parts() {
            let Some(result) = cx.part_if_declared(reference) else { continue };
            let (press, hover) = (result.press, cx.hover_pos(reference));
            let mods = self.mods.unwrap_or(press.mods);
            let acting = drag_mods(mods, suspend);
            if let Some(pos) = hover
                && held.is_none()
            {
                self.report.hovered = Some(part);
                let cursor = self.cursor(&CursorLook { space, screen: &screen, style }, part, pos, acting, false);
                cx.cursor(cursor, CursorPriority::Part);
            }
            match press.phase {
                // Past a border zone's reach the outside keeps the press and does nothing with it.
                PressPhase::Down { start, .. } if part == BoxPart::Outside && !self.rotate_zone.rotates_at(&screen.quad, start.origin) => {}
                PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                    let from = space.view.to_doc(start.origin);
                    // The move reach only moves: an Alt press there is a move too. The knob and the rotate handle are
                    // handles, so an Alt press on one turns the box rather than moving the reference point onto it.
                    let knob = reference.key == KNOB;
                    let away_from_handles = part == BoxPart::Inside || (part == BoxPart::Outside && self.rotate_zone != RotateZone::Handle && !knob);
                    let moves_pivot = acting.alt && self.move_only().is_none() && away_from_handles;
                    let follows = follows_in_slop(part);
                    let (part, begin) = if moves_pivot { (BoxPart::Pivot, BoxState::new(shown.quad, from)) } else { (part, *shown) };
                    held = Some(Grip { part, start: begin, from, began: false, follows, moves_pivot, pressed: *shown, last: *shown });
                }
                // Inside the slop a part that follows there moves the box from the first move; the outside starts at
                // the drag.
                PressPhase::Held { start, pos, .. } => {
                    let Some(mut grip) = held else { continue };
                    if !self.rules.allows(grip.part) || !grip.follows || (!grip.began && pos == start.origin) {
                        continue;
                    }
                    if !grip.began {
                        cx.begin();
                        grip.began = true;
                    }
                    self.follow(cx, space, snap.as_ref(), &mut grip, pos, mods);
                    held = Some(grip);
                }
                PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => {
                    let Some(mut grip) = held else { continue };
                    if !self.rules.allows(grip.part) {
                        continue;
                    }
                    if matches!(press.phase, PressPhase::DragStart { .. }) && !grip.began {
                        cx.begin();
                        grip.began = true;
                    }
                    if !grip.began {
                        continue;
                    }
                    self.follow(cx, space, snap.as_ref(), &mut grip, pos, mods);
                    held = Some(grip);
                }
                PressPhase::Release { start, click, .. } => {
                    // A double click inside applies, whether or not the first press of the release was seen.
                    if click && start.clicks >= 2 && start.button == PointerButton::Primary && part == BoxPart::Inside && self.move_only().is_none() {
                        self.report.apply = Some(ApplyBy::DoubleClick);
                    }
                    let Some(grip) = held.take() else { continue };
                    if click
                        && start.button == PointerButton::Primary
                        && grip.part != BoxPart::Outside
                        && !grip.moves_pivot
                        && self.move_only().is_none()
                    {
                        self.report.clicked = Some((grip.part, grip.from));
                    }
                    if grip.began && click && grip.last == grip.pressed {
                        // A click that ended where it began moved nothing: no step.
                        cx.cancel(CancelReason::Unchanged);
                    } else if grip.began {
                        cx.commit();
                    } else if grip.part == BoxPart::Outside && reference.key == OUTSIDE && click && start.clicks <= 1 {
                        self.report.apply = Some(ApplyBy::ClickOutside { at: space.view.to_doc(start.origin) });
                    } else if grip.moves_pivot && click {
                        cx.set(grip.start);
                    }
                }
                PressPhase::Cancel { .. } => held = None,
                _ => {}
            }
        }
        *cx.press_slot::<Grip>() = held;
    }

    /// One step of a begun grip with the pointer at `pos`: the proposal and its snap lines, the readout and the drag
    /// cursor.
    fn follow(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, snap: Option<&Snap>, grip: &mut Grip, pos: Pos2, mods: Mods) {
        let style = cx.style;
        let (next, lines) = self.proposal(space, snap, grip, pos, mods);
        self.report.snapped = lines;
        grip.last = next;
        cx.propose(next);
        let rows = self.readout_rows(cx, space, grip.part, &next, &grip.start);
        readout(&*cx, pos, &rows);
        let acting = drag_mods(mods, suspend_key(snap));
        let cursor = self.cursor(&CursorLook { space, screen: &on_screen(&space.view, &next), style }, grip.part, pos, acting, true);
        cx.cursor(cursor, CursorPriority::Drag);
    }

    /// The pointer on a line's free ends: a drag of an end reports [`LineEndDrag`] (`Begin` with its first position
    /// once the press passes the slop, `Update`, `Commit` on the release, `Cancel` when the press is cancelled) with
    /// the end snapped through the canvas's snap, and leaves the box's value alone: the host moves the end. A click
    /// moves nothing.
    fn run_line_ends(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, line: &LineLook) {
        let snap = canvas_snap(&cx.env);
        let mut held: Option<EndGrip> = *cx.press_slot::<EndGrip>();
        for (end, at) in line.free_ends() {
            let Some(result) = cx.part_if_declared(LINE_END.at(end.index())) else { continue };
            let (press, hovered) = (result.press, result.hovered);
            let mods = self.mods.unwrap_or(press.mods);
            if hovered && held.is_none() {
                cx.cursor(Cursor::System(CursorIcon::Move), CursorPriority::Part);
            }
            let ours = held.filter(|grip| grip.end == end);
            let (phase, pos, grip) = match (press.phase, ours) {
                (PressPhase::Down { start, .. }, _) if start.button == PointerButton::Primary => {
                    held = Some(EndGrip { end, offset: at - space.view.to_doc(start.origin), began: false });
                    continue;
                }
                (PressPhase::DragStart { pos, .. }, Some(grip)) => (GesturePhase::Begin, pos, grip),
                (PressPhase::Drag { pos, .. }, Some(grip)) if grip.began => (GesturePhase::Update, pos, grip),
                (PressPhase::Release { pos, .. }, Some(grip)) => {
                    held = None;
                    if !grip.began {
                        continue;
                    }
                    (GesturePhase::Commit, pos, grip)
                }
                (PressPhase::Cancel { .. }, Some(grip)) => {
                    held = None;
                    if grip.began {
                        self.report.line_end = Some(LineEndDrag { phase: GesturePhase::Cancel, end, at, mods });
                    }
                    continue;
                }
                _ => continue,
            };
            let to = space.view.to_doc(pos) + grip.offset;
            let (to, lines) = match &snap {
                Some(snap) => snap.point(to, mods, GestureSnap::default()),
                None => (to, Vec::new()),
            };
            self.report.snapped = lines;
            self.report.line_end = Some(LineEndDrag { phase, end, at: to, mods });
            held = (phase != GesturePhase::Commit).then_some(EndGrip { began: true, ..grip });
            cx.cursor(Cursor::System(CursorIcon::Move), CursorPriority::Drag);
        }
        *cx.press_slot::<EndGrip>() = held;
    }

    /// The session keys: the arrows nudge the picked part (the whole box when no part has focus), Enter applies and
    /// Escape closes the box. Untaken, a session's arrows go back to the canvas's tool.
    fn run_keys(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, shown: &BoxState, pressing: bool) {
        let focused = cx.active_part().filter(|part| part.key == BOX).and_then(|part| part.index).and_then(BoxPart::from_index);
        // A part picked from the keyboard (Tab, Page Down, Home, End) holds the arrows; a part that focus reached by
        // a press (the handle a drag just ended on) is not picked, so the arrows move the whole box.
        let arrived = cx.focus_arrived_by_key().filter(|part| part.key == BOX).and_then(|part| part.index).and_then(BoxPart::from_index);
        let keyed = cx.memory::<KeyPicked>();
        if pressing || focused.is_none() {
            keyed.0 = None;
        } else if arrived.is_some() {
            keyed.0 = arrived;
        } else if keyed.0 != focused {
            keyed.0 = None;
        }
        let picked = keyed.0;
        self.report.picked = picked;
        let takes_arrows = self.session_arrows || (cx.is_focused() && (focused.is_none() || picked.is_some()));
        if takes_arrows && let Some((direction, mods)) = cx.intents().take_nudge() {
            let step = if mods.shift { NUDGE_FAR } else { NUDGE };
            let doc = space.view.unmap_vec(direction);
            let length = doc.hypot();
            if length > 0.0 {
                let part =
                    picked.filter(|&part| self.rules.allows(part) && (self.reference_point || part != BoxPart::Pivot)).unwrap_or(BoxPart::Inside);
                let next = self.nudged(shown, part, doc * (step / length));
                if next != *shown {
                    cx.set(next);
                }
            }
        }
        if self.session_commit && cx.intents().take(Intent::Submit) {
            self.report.apply = Some(ApplyBy::Key);
        }
        if self.session_commit && !pressing && cx.intents().take(Intent::Cancel) {
            self.report.discard = true;
        }
    }
}

impl TransformBox<'_> {
    /// The shape of a handle of `side` at `at`: a square, or a circle of that diameter with round handles.
    fn handle_shape(&self, at: Pos2, side: f32) -> PaintShape {
        if self.round_handles {
            PaintShape::Circle(at, side / 2.0)
        } else {
            PaintShape::Rect(Rect::from_center_size(at, Vec2::splat(side)), CornerRadius::ZERO)
        }
    }

    /// The square of handle `part` at `at` on the screen, filled as the part's state shows when the rules let it take a
    /// press; `enlarged` with the overflow plus.
    fn paint_handle(&self, draw: &Draw<'_>, cx: &Cx<'_, Self>, part: BoxPart, at: Pos2, enlarged: bool) {
        let look = &cx.style.handle;
        let side = self.side_of_handles(cx.style);
        let side = if enlarged { side * OVERFLOW_IN_HANDLES } else { side };
        self.paint_handle_shape(draw, cx, self.handle_shape(at, side), self.rules.allows(part).then(|| BOX.at(part.index())));
        if enlarged {
            Handle::plus(draw, look, at, side);
        }
    }

    /// A live line's controls: the segment-aligned box with a square at the middle of each side (drawn only), a
    /// handle square at each end, and the round widget at the middle of the segment.
    fn paint_line(&self, draw: &Draw<'_>, cx: &Cx<'_, Self>, view: &CanvasView, line: &LineLook) {
        let style = cx.style;
        if let Some(outline) = line.outline {
            let corners = outline.map(|corner| view.to_screen(corner));
            paint_frame(draw, style, self.ink(style.line), corners);
            for k in 0..4 {
                let middle = corners[k].lerp(corners[(k + 1) % 4], 0.5);
                self.paint_handle_shape(draw, cx, self.handle_shape(middle, self.side_of_handles(style)), None);
            }
        }
        for (part, end) in line.end_parts() {
            self.paint_handle(draw, cx, part, view.to_screen(end), false);
        }
        for (end, at) in line.free_ends() {
            self.paint_handle_shape(draw, cx, self.handle_shape(view.to_screen(at), self.side_of_handles(style)), Some(LINE_END.at(end.index())));
        }
        paint_widget(draw, view.to_screen(line.from.midpoint(line.to)), &style.widget, None);
    }

    /// A handle's `square` (a circle with round handles), filled as the state of `part` shows, or at rest without one.
    fn paint_handle_shape(&self, draw: &Draw<'_>, cx: &Cx<'_, Self>, square: PaintShape, part: Option<PartRef>) {
        let look = &cx.style.handle;
        let fill = match part {
            Some(part) => cx.resolve_part(part, &look.fill),
            None => cx.resolve(&look.fill),
        };
        draw.fill(&square, fill);
        draw.stroke(&square, StrokeWidth::Thin, self.ink(look.stroke));
    }

    /// The rotate handle of [`RotateZone::Handle`] on the box whose corners are `quad` on the screen: its stem from the
    /// middle of side 0 over its halo, and the round knob at its end, filled as the handle's state shows.
    fn paint_rotate_handle(&self, draw: &Draw<'_>, cx: &Cx<'_, Self>, quad: &[Pos2; 4]) {
        let style = cx.style;
        let (base, knob) = rotate_handle(quad, style.rotate_stem);
        let stem = PaintShape::Path(vec![base, knob]);
        draw.stroke(&stem, StrokeWidth::Thick, style.halo);
        draw.stroke(&stem, StrokeWidth::Medium, self.ink(style.line));
        let look = &style.handle;
        let circle = PaintShape::Circle(knob, style.rotate_knob);
        draw.fill(&circle, cx.resolve_part(ROTATE_HANDLE.at(0), &look.fill));
        draw.stroke(&circle, StrokeWidth::Thin, self.ink(look.stroke));
    }
}

/// The rotate handle of the box whose corners are `quad` on the screen: the middle of side 0 (corners 0 to 1) and the
/// knob's centre, `stem` out from it across the side, so the handle turns with the box.
fn rotate_handle(quad: &[Pos2; 4], stem: f32) -> (Pos2, Pos2) {
    let base = quad[0].lerp(quad[1], 0.5);
    (base, base + outward_normals(quad)[0] * stem)
}

/// The outline of a box whose corners are `corners` on the screen: `line` over the halo.
fn paint_frame(draw: &Draw<'_>, style: &TransformBoxStyle, line: StrokeColor, corners: [Pos2; 4]) {
    let outline = PaintShape::Polygon(corners.to_vec());
    draw.stroke(&outline, StrokeWidth::Thick, style.halo);
    draw.stroke(&outline, StrokeWidth::Medium, line);
}

/// The move reach outside the box whose corners are `quad` on the screen: every point within `points` of its border,
/// as a strip along each side and a round sector outside each corner (PhotoCraft's type move reach measures the
/// distance to the nearest side, pc-prod-e76 type_transform.rs:156-196).
fn declare_ring(cx: &mut PartsCx<'_, TransformBox<'_>>, quad: &[Pos2; 4], points: f32, context: Sense) {
    let outward = outward_normals(quad);
    let mut declare = |part: usize, shape: Vec<Pos2>| {
        cx.part(MOVE_RING.at(part as u64), HitShape::Polygon(shape))
            .sense(Sense::DRAG | Sense::CLICK | slop_sense(BoxPart::Inside) | context)
            .priority(HitPriority::new(HitLevel::Body, RING_SUB))
            .pointer_only(MOVE_BY_KEY);
    };
    for side in 0..4usize {
        let (a, b, out) = (quad[side], quad[(side + 1) % 4], outward[side] * points);
        declare(side, vec![a, b, b + out, a + out]);
    }
    for corner in 0..4usize {
        let (from, to) = (outward[(corner + 3) % 4], outward[corner]);
        let (start, mut turn) = (from.angle(), to.angle() - from.angle());
        if turn > std::f32::consts::PI {
            turn -= std::f32::consts::TAU;
        } else if turn < -std::f32::consts::PI {
            turn += std::f32::consts::TAU;
        }
        let centre = quad[corner];
        let arc = (0..=RING_ARC_STEPS).map(|step| centre + Vec2::angled(start + turn * step as f32 / RING_ARC_STEPS as f32) * points);
        declare(4 + corner, std::iter::once(centre).chain(arc).collect());
    }
}

/// The unit normal of each side of `quad` that points out of the box, whichever way its corners turn; a side with no
/// length points away from the box's centre.
fn outward_normals(quad: &[Pos2; 4]) -> [Vec2; 4] {
    let area: f32 = (0..4).map(|k| quad[k].to_vec2().x * quad[(k + 1) % 4].y - quad[(k + 1) % 4].x * quad[k].y).sum();
    let turn = if area < 0.0 { -1.0 } else { 1.0 };
    let centre = quad.iter().fold(Vec2::ZERO, |sum, corner| sum + corner.to_vec2()) / 4.0;
    std::array::from_fn(|side| {
        let (a, b) = (quad[side], quad[(side + 1) % 4]);
        let along = b - a;
        if along.length() > 1e-3 { Vec2::new(along.y, -along.x).normalized() * turn } else { (a.to_vec2() - centre).normalized() }
    })
}

/// The rotate knob of the box whose corners are `quad` on the screen, in the canvas body `within`: the knob and the foot
/// of its stem. The knob stands `rotate_reach` out from the middle of the top side, along the side's outward normal so a
/// turned box's knob turns with it; where that point is outside `within`, it stands out from the bottom side instead.
fn knob_at(quad: &[Pos2; 4], within: Rect, style: &TransformBoxStyle) -> (Pos2, Pos2) {
    let outward = outward_normals(quad);
    let off = |side: usize| {
        let foot = quad[side].lerp(quad[(side + 1) % 4], 0.5);
        (foot + outward[side] * style.rotate_reach, foot)
    };
    let top = off(0);
    if within.contains(top.0) { top } else { off(2) }
}

/// The reference point's glyph centred at `centre`: a circle crossed by two arms, over its halo. The one glyph of the
/// reference point and of the centre mark; `line` is the colour over the halo.
fn paint_centre_glyph(draw: &Draw<'_>, centre: Pos2, style: &TransformBoxStyle, line: StrokeColor) {
    let (radius, arm) = (style.pivot_radius, style.pivot_arm);
    for (width, color) in [(StrokeWidth::Thick, StrokeColor::from(style.halo)), (StrokeWidth::Medium, line)] {
        draw.stroke(&PaintShape::Circle(centre, radius), width, color);
        draw.stroke(&PaintShape::Path(vec![centre - Vec2::new(arm, 0.0), centre + Vec2::new(arm, 0.0)]), width, color);
        draw.stroke(&PaintShape::Path(vec![centre - Vec2::new(0.0, arm), centre + Vec2::new(0.0, arm)]), width, color);
    }
}

/// The box handle a line's end at `end` stands for, the other end at `other`: the corner it points to, away from the
/// other end, or on a level or upright line the side. `None` when the ends meet.
fn end_part(end: Point, other: Point) -> Option<BoxPart> {
    let sign = |d: f64| i8::from(d > 0.0) - i8::from(d < 0.0);
    match (sign(end.x - other.x), sign(end.y - other.y)) {
        (-1, -1) => Some(BoxPart::Corner(0)),
        (1, -1) => Some(BoxPart::Corner(1)),
        (1, 1) => Some(BoxPart::Corner(2)),
        (-1, 1) => Some(BoxPart::Corner(3)),
        (0, -1) => Some(BoxPart::Edge(0)),
        (1, 0) => Some(BoxPart::Edge(1)),
        (0, 1) => Some(BoxPart::Edge(2)),
        (-1, 0) => Some(BoxPart::Edge(3)),
        _ => None,
    }
}

/// What the cursor of a part reads: the box on the screen, the canvas space and the style.
struct CursorLook<'a> {
    space: &'a CanvasSpace,
    screen: &'a BoxState,
    style: &'a TransformBoxStyle,
}

impl Component for TransformBox<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::CANVAS.with_moves().with_session();
    type Style = TransformBoxStyle;
    type Output = TransformBoxOutput;

    fn style(cx: &StyleCx<'_>, _: TransformBoxStyleKey) -> TransformBoxStyle {
        let overlay = &cx.tokens.color.overlay;
        let (hit, handle) = (&cx.tokens.size.hit, &cx.tokens.size.handle);
        TransformBoxStyle {
            metrics: Metrics::new(),
            handle: cx.handle(),
            line: cx.color(overlay.line),
            halo: cx.color(overlay.halo),
            handle_reach: handle.md * HANDLE_REACH_IN_HANDLES,
            side_band: handle.md * SIDE_BAND_IN_HANDLES,
            hit: handle.md * HIT_IN_HANDLES,
            rotate_reach: handle.md * ROTATE_REACH_IN_HANDLES,
            band_inner: handle.md * BAND_INNER_IN_HANDLES,
            band_outer: handle.md * BAND_OUTER_IN_HANDLES,
            pivot_radius: cx.tokens.size.handle.sm / 2.0,
            pivot_arm: hit.target,
            rotate_stem: handle.md * STEM_IN_HANDLES,
            rotate_knob: handle.sm / 2.0,
            widget: CornerStyle::from_cx(cx),
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn bind(&mut self) -> Option<BindMut<'_, BoxState>> {
        Some(self.state.reborrow())
    }

    /// A host keeps the box in f32 (a layer's transform, a text layer's box) and gives a proposal back rounded to
    /// it: each corner and the pivot within f32 precision of the proposal is the proposal, not a change of the
    /// document under the drag. The host computes every corner from the same box and transform, so a corner near
    /// the origin holds the precision of the box's largest coordinate, not its own.
    fn same_value(&self, proposed: &BoxState, host: &BoxState) -> bool {
        let points = proposed.quad.iter().chain(&host.quad).chain([&proposed.pivot, &host.pivot]);
        let magnitude = points.fold(0.0f64, |most, point| most.max(point.x.abs()).max(point.y.abs()));
        let same = |a: f64, b: f64| craft_behavior::same_in_f32_at(a, b, magnitude);
        let near = |a: Point, b: Point| same(a.x, b.x) && same(a.y, b.y);
        proposed.quad.iter().zip(&host.quad).all(|(a, b)| near(*a, *b)) && near(proposed.pivot, host.pivot)
    }

    /// The whole box, the corners, the edges and the reference point the rules allow, in keyboard order, then each
    /// side as a pointer-only band outside it (either side of its line with the rules' edge band), each corner's stretch of the two bands beside it, the centre and the move
    /// strips where declared, and the outside. A handle, a corner's stretch, the reference point and the centre rank
    /// alike, so the nearest wins where they overlap on a small box;
    /// they rank above a side, which ranks above the inside, which ranks above the outside. Every part takes a press's
    /// click with its drag, so a press on the box is the box's alone, not split with what lies under it (8.3). A live
    /// line's controls declare its two ends only; free ends as points of their own. In [`RotateZone::Handle`] the
    /// rotate handle ranks with the handles and is the only outside part.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let (Some(space), Some(state)) = (canvas_space(&cx.env), cx.value().copied()) else { return };
        if !self.takes_input() {
            return;
        }
        let style = cx.style;
        let screen = on_screen(&space.view, &state);
        let quad: [Pos2; 4] = screen.quad.map(pos);
        let context = if self.own_context_menu { Sense::CONTEXT_CLICK } else { Sense::empty() };
        if self.move_only().is_some() {
            if let Some(points) = self.ring_reach() {
                declare_ring(cx, &quad, points, context);
            }
            return;
        }
        let mut any = false;
        for part in self.keyboard_parts() {
            let (shape, sense, priority, focus) = match part {
                BoxPart::Inside => {
                    let sense = Sense::DRAG | Sense::CLICK | Sense::TAKE_FOCUS;
                    (HitShape::Polygon(quad.to_vec()), sense, HitPriority::new(HitLevel::Body, 0), FocusShape::Rect)
                }
                BoxPart::Pivot => (
                    HitShape::circle(pos(screen.pivot), style.hit),
                    Sense::DRAG | Sense::CLICK | Sense::TAKE_FOCUS,
                    HitPriority::new(HitLevel::Handle, HANDLE_SUB),
                    FocusShape::Circle,
                ),
                handle => {
                    let at = self.part_point(&state, handle).map_or(Pos2::ZERO, |at| space.view.to_screen(at));
                    (
                        HitShape::circle(at, self.reach_of_handles(style)),
                        Sense::DRAG | Sense::CLICK | Sense::TAKE_FOCUS,
                        HitPriority::new(HitLevel::Handle, HANDLE_SUB),
                        FocusShape::Circle,
                    )
                }
            };
            cx.part(BOX.at(part.index()), shape).sense(sense | slop_sense(part) | context).priority(priority).focus_shape(focus).focusable();
            any = true;
        }
        if let Some(line) = self.line {
            for (end, at) in line.free_ends() {
                cx.part(LINE_END.at(end.index()), HitShape::circle(space.view.to_screen(at), self.reach_of_handles(style)))
                    .sense(Sense::DRAG | Sense::CLICK | context)
                    .priority(HitPriority::new(HitLevel::Handle, HANDLE_SUB))
                    .pointer_only(LINE_END_BY_KEY);
            }
            if any {
                cx.roving(Roving::new(NavShape::Flat).page(Some(1)));
            }
            return;
        }
        for k in (0..4u8).filter(|&k| self.rules.allows(BoxPart::Edge(k))) {
            let side = usize::from(k);
            let band =
                if self.rules.edge_band > 0.0 { edge_band(&quad, side, self.rules.edge_band) } else { side_band(&quad, side, style.side_band) };
            cx.part(EDGE.at(u64::from(k)), band)
                .sense(Sense::DRAG | Sense::CLICK | slop_sense(BoxPart::Edge(k)) | context)
                .priority(HitPriority::new(HitLevel::Handle, EDGE_SUB))
                .pointer_only(EDGE_BY_KEY);
        }
        for k in (0..4u8).filter(|&k| self.rules.allows(BoxPart::Corner(k))) {
            let corner = usize::from(k);
            // The side that starts at the corner, then the one that ends there.
            for (end, side) in [(0u64, corner), (1, (corner + 3) % 4)] {
                let Some(reach) = corner_reach(&quad, side, corner, style.side_band) else { continue };
                cx.part(CORNER_REACH.at(2 * u64::from(k) + end), reach)
                    .sense(Sense::DRAG | Sense::CLICK | slop_sense(BoxPart::Corner(k)) | context)
                    .priority(HitPriority::new(HitLevel::Handle, HANDLE_SUB))
                    .pointer_only(CORNER_BY_KEY);
            }
        }
        if self.declares_centre() {
            let centre = quad.iter().fold(Vec2::ZERO, |sum, corner| sum + corner.to_vec2()) / 4.0;
            cx.part(CENTRE.at(0), HitShape::circle(centre.to_pos2(), style.hit))
                .sense(Sense::DRAG | Sense::CLICK | slop_sense(BoxPart::Inside) | context)
                .priority(HitPriority::new(HitLevel::Handle, HANDLE_SUB))
                .pointer_only(MOVE_BY_KEY);
        }
        if any {
            cx.roving(Roving::new(NavShape::Flat).page(Some(1)));
        }
        match self.rotate_zone {
            RotateZone::Outside | RotateZone::Border { .. } if self.rules.allows(BoxPart::Outside) => {
                for (corner, region) in outside_regions(&quad, rect).into_iter().enumerate() {
                    cx.part(OUTSIDE.at(corner as u64), HitShape::Polygon(region))
                        .sense(Sense::DRAG | Sense::CLICK | slop_sense(BoxPart::Outside) | context)
                        .priority(HitPriority::new(HitLevel::Background, 0))
                        .pointer_only(ROTATE_BY_KEY);
                }
            }
            RotateZone::Band if self.rules.allows(BoxPart::Outside) => {
                for (corner, ring) in band_rings(&screen, style).into_iter().enumerate() {
                    cx.part(OUTSIDE.at(corner as u64), HitShape::Polygon(ring))
                        .sense(Sense::DRAG | Sense::CLICK | slop_sense(BoxPart::Outside) | context)
                        .priority(HitPriority::new(HitLevel::Body, 0))
                        .pointer_only(ROTATE_BY_KEY);
                }
            }
            RotateZone::Outside | RotateZone::Border { .. } | RotateZone::Band => {}
            // The knob alone turns the box; the rest of the outside is the tool's.
            RotateZone::Knob => {
                if self.has_knob() {
                    let (knob, _) = knob_at(&quad, rect, style);
                    cx.part(KNOB.at(0), HitShape::circle(knob, style.handle_reach))
                        .sense(Sense::DRAG | Sense::CLICK | slop_sense(BoxPart::Outside) | context)
                        .priority(HitPriority::new(HitLevel::Handle, HANDLE_SUB))
                        .pointer_only(ROTATE_BY_KEY);
                }
            }
            RotateZone::Handle if self.rules.allows(BoxPart::Outside) => {
                let (_, knob) = rotate_handle(&quad, style.rotate_stem);
                cx.part(ROTATE_HANDLE.at(0), HitShape::circle(knob, style.handle_reach))
                    .sense(Sense::DRAG | Sense::CLICK | slop_sense(BoxPart::Outside) | context)
                    .priority(HitPriority::new(HitLevel::Handle, HANDLE_SUB))
                    .pointer_only(ROTATE_BY_KEY);
            }
            RotateZone::Handle => {}
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let (Some(space), Some(shown)) = (canvas_space(&cx.env), cx.value().copied()) else { return };
        if !self.takes_input() {
            return;
        }
        let held = cx.press_slot::<Grip>().is_some() || cx.press_slot::<EndGrip>().is_some();
        self.run_pointer(cx, &space, &shown);
        if let Some(line) = self.line.filter(|line| line.ends == LineEnds::Free) {
            self.run_line_ends(cx, &space, &line);
        }
        self.pivot_held = cx.press_slot::<Grip>().is_some_and(|grip| grip.part == BoxPart::Pivot);
        self.dragging = cx.press_slot::<Grip>().is_some_and(|grip| grip.began);
        self.report.held = cx.press_slot::<Grip>().map(|grip| grip.part);
        let pressing = held || cx.press_slot::<Grip>().is_some() || cx.press_slot::<EndGrip>().is_some();
        if self.move_only().is_none() {
            self.run_keys(cx, &space, &shown, pressing);
        }
    }

    /// The frame and its handles, with the centre mark under the handles; or, for a live line not being dragged, the
    /// line's controls. The reference point over both. A box that offers only its move reach is drawn only while its
    /// drag runs.
    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if self.move_only().is_some() && !self.dragging {
            return;
        }
        let (Some(space), Some(state)) = (canvas_space(&cx.env), cx.value()) else { return };
        let screen = on_screen(&space.view, state);
        let style = cx.style;
        let draw = ui.draw();
        match self.line.filter(|_| !self.dragging) {
            Some(line) => self.paint_line(&draw, cx, &space.view, &line),
            None => {
                paint_frame(&draw, style, self.ink(style.line), screen.quad.map(pos));
                if self.rotate_zone == RotateZone::Handle && self.rules.allows(BoxPart::Outside) {
                    self.paint_rotate_handle(&draw, cx, &screen.quad.map(pos));
                }
                if self.centre_mark {
                    let centre = screen.quad.iter().fold(Vec2::ZERO, |sum, corner| sum + pos(*corner).to_vec2()) / 4.0;
                    paint_centre_glyph(&draw, centre.to_pos2(), style, self.ink(style.line));
                }
                // A handle the handle set hides is not drawn; one the mode refuses still draws, dimmed by
                // `paint_handle`.
                for part in (0..4u8).flat_map(|i| [BoxPart::Corner(i), BoxPart::Edge(i)]).filter(|part| self.rules.shows(*part)) {
                    let Some(at) = handle_point(&screen, part) else { continue };
                    let enlarged = self.overflow && part == BoxPart::Corner(2);
                    self.paint_handle(&draw, cx, part, pos(at), enlarged);
                }
                if self.has_knob() {
                    let (knob, foot) = knob_at(&screen.quad.map(pos), cx.rect, style);
                    draw.stroke(&PaintShape::Path(vec![foot, knob]), StrokeWidth::Thin, style.line);
                    paint_widget(&draw, knob, &style.widget, self.color);
                }
            }
        }
        if self.reference_point || self.pivot_held {
            paint_centre_glyph(&draw, pos(screen.pivot), style, self.ink(style.line));
        }
    }

    /// One node per part: the whole box, the corners, the edges and the reference point, each described by its point
    /// in the canvas's units, and the rotate knob where the box has one.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let (Some(space), Some(state)) = (canvas_space(&cx.env), cx.value()) else { return };
        let words = cx.words::<TransformBoxWords>();
        let units = cx.env.units();
        let rulers = space.rulers();
        let ((across_x, zero_x), (across_y, zero_y)) = (rulers.across(Axis::Y), rulers.across(Axis::X));
        for part in self.keyboard_parts() {
            let node = access.node(BOX.at(part.index()), Role::BUTTON);
            node.name = Name::Text(words.name_of(part).to_owned());
            if let Some(at) = self.part_point(state, part) {
                node.description = Some(format!("{}, {}", across_x.format_value(&units, at.x, zero_x), across_y.format_value(&units, at.y, zero_y)));
            }
        }
        // The knob is pointer-only, so it is not among the keyboard parts; its node names it for assistive tools.
        if self.has_knob() {
            access.node(KNOB.at(0), Role::BUTTON).name = Name::Text(words.rotate_knob.as_str().to_owned());
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> TransformBoxOutput {
        let Report { apply, clicked, discard, hovered, picked, held, snapped, line_end } = self.report;
        TransformBoxOutput { edit: cx.edit_response(), apply, discard, picked, hovered, held, snapped, clicked, line_end }
    }
}

/// What the box reports: the edit of the box and what it asks the host to do.
#[derive(Debug)]
#[non_exhaustive]
pub struct TransformBoxOutput {
    /// The part and press point in document units of a primary click on the box or a grip, including a release within click slop.
    pub clicked: Option<(BoxPart, Point)>,
    /// A drag (`Begin` on the first move, `Update`, `Commit` on the release; Escape cancels it) or an arrow key's
    /// `Set`.
    pub edit: EditResponse<BoxState>,
    /// Apply the box, and why: Enter, a click outside it, a double click inside it.
    pub apply: Option<ApplyBy>,
    /// Close the box without applying it: Escape while no drag is on.
    pub discard: bool,
    /// The part the keys act on, while the box has the keyboard.
    pub picked: Option<BoxPart>,
    /// The part a press at the pointer would take; `None` where it is the tool's.
    pub hovered: Option<BoxPart>,
    /// The part the open press holds, from the press to its release or cancel, before and during its drag.
    pub held: Option<BoxPart>,
    /// The snap lines of the point being dragged, for the host's one drawing site.
    pub snapped: Vec<craft_behavior::geometry::SnapLine>,
    /// The drag of a line end, in [`LineEnds::Free`].
    pub line_end: Option<LineEndDrag>,
}

impl Outcome for TransformBoxOutput {
    type Value = BoxState;

    fn response(&self) -> &Response {
        &self.edit.response
    }
}

/// `state` as the view draws it, in screen points.
fn on_screen(view: &CanvasView, state: &BoxState) -> BoxState {
    let affine = view.affine();
    BoxState::new(state.quad.map(|p| affine * p), affine * state.pivot)
}

fn pos(p: Point) -> Pos2 {
    Pos2::new(p.x as f32, p.y as f32)
}

fn vec2_of(v: DocVec) -> Vec2 {
    Vec2::new(v.x as f32, v.y as f32)
}

/// The suspend key of the canvas's snap, Control without one.
fn suspend_key(snap: Option<&Snap>) -> SnapSuspend {
    snap.map_or_else(SnapSuspend::default, |snap| snap.options().suspend)
}

/// The keys a drag acts on, from the keys held: while Alt suspends the canvas's snapping, Alt is the snap's alone, so
/// it neither scales about the reference point nor moves it.
fn drag_mods(mods: Mods, suspend: SnapSuspend) -> Mods {
    if suspend == SnapSuspend::Alt { Mods { alt: false, ..mods } } else { mods }
}

/// The position of a corner, an edge's midpoint or the reference point; `None` for the inside and the outside.
fn handle_point(state: &BoxState, part: BoxPart) -> Option<Point> {
    let q = &state.quad;
    match part {
        BoxPart::Corner(i) => Some(q[usize::from(i % 4)]),
        BoxPart::Edge(i) => Some(q[usize::from(i % 4)].midpoint(q[usize::from(i + 1) % 4])),
        BoxPart::Pivot => Some(state.pivot),
        _ => None,
    }
}

/// The bounds of `quad`.
fn bounds_of(quad: &[Point; 4]) -> DocRect {
    quad[1..].iter().fold(DocRect::from_points(quad[0], quad[0]), |r, p| r.union_pt(*p))
}

/// `p` pulled onto the nearest of the nine points of `state`'s box (its corners, edge midpoints and centre) within
/// `reach` document units, else `p`.
fn box_point_pull(state: &BoxState, p: Point, reach: f64) -> Point {
    let q = &state.quad;
    let points = [
        q[0],
        q[1],
        q[2],
        q[3],
        q[0].midpoint(q[1]),
        q[1].midpoint(q[2]),
        q[2].midpoint(q[3]),
        q[3].midpoint(q[0]),
        q[0].midpoint(q[2]).midpoint(q[1].midpoint(q[3])),
    ];
    points.into_iter().map(|a| (a.distance(p), a)).filter(|(d, _)| *d <= reach).min_by(|a, b| a.0.total_cmp(&b.0)).map_or(p, |(_, a)| a)
}

/// The corner of `screen` nearest `p` and its distance.
fn nearest_corner(screen: &BoxState, p: Pos2) -> (usize, f32) {
    let p = Point::new(f64::from(p.x), f64::from(p.y));
    (0..4).map(|i| (i, screen.quad[i].distance(p) as f32)).min_by(|a, b| a.1.total_cmp(&b.1)).unwrap_or((0, f32::INFINITY))
}

/// The distance from `p` to the nearest side of the box whose corners are `quad`, all on the screen.
fn border_distance(quad: &[Point; 4], p: Pos2) -> f32 {
    let p = Point::new(f64::from(p.x), f64::from(p.y));
    (0..4)
        .map(|side| {
            let (a, b) = (quad[side], quad[(side + 1) % 4]);
            let along = b - a;
            let length = along.hypot2();
            let t = if length > 0.0 { ((p - a).dot(along) / length).clamp(0.0, 1.0) } else { 0.0 };
            (p - (a + along * t)).hypot() as f32
        })
        .fold(f32::INFINITY, f32::min)
}

/// The outside of the box `quad` within `within`, split into one region per corner: region `i` lies between the rays
/// from the box's centre through the midpoints of the two sides that meet at corner `i`. For a convex box the regions
/// tile the outside; where a folded box makes one reach inside, the inside part ranks above it.
fn outside_regions(quad: &[Pos2; 4], within: Rect) -> [Vec<Pos2>; 4] {
    let centre = quad.iter().fold(Vec2::ZERO, |sum, corner| sum + corner.to_vec2()) / 4.0;
    let centre = centre.to_pos2();
    let far = (within.size().length() + centre.distance(within.center())) * 2.0 + 1.0;
    // Side `k` runs from corner `k` to corner `k + 1`; a box with no extent there points the ray along an axis.
    let mid = |k: usize| quad[k % 4].lerp(quad[(k + 1) % 4], 0.5);
    let ray = |k: usize| {
        let along = mid(k) - centre;
        if along.length() > 1e-3 {
            along.normalized()
        } else {
            [Vec2::new(0.0, -1.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0), Vec2::new(-1.0, 0.0)][k % 4]
        }
    };
    std::array::from_fn(|i| {
        let (previous, next) = (i + 3, i);
        let between = ray(previous) + ray(next);
        // Far enough along the bisector to lie past the canvas however wide the angle between the rays is.
        let out = if between.length() > 0.1 { between * (2.0 * far / between.length_sq()) } else { (quad[i] - centre).normalized() * far };
        let region = vec![quad[i], mid(next), mid(next) + ray(next) * far, centre + out, mid(previous) + ray(previous) * far, mid(previous)];
        clipped_to(&region, within)
    })
}

/// The band outside side `side` of the box `quad` (from corner `side` to the next), `width` wide, with the side's
/// line itself: a press on the line or outside it within `width` takes the side; inside the box it does not. A side
/// with no length is its line.
fn side_band(quad: &[Pos2; 4], side: usize, width: f32) -> HitShape {
    let (a, b) = (quad[side], quad[(side + 1) % 4]);
    match outward(quad, side) {
        Some(out) => band_along(a, b, out, width),
        None => HitShape::segment(a, b),
    }
}

/// The grab band of side `side` of the box `quad` by [`BoxRules::edge_band`]: `reach` out from the line and past each
/// end, and as far in, but no more than a quarter of the way to the opposite side. A side with no length is its line.
fn edge_band(quad: &[Pos2; 4], side: usize, reach: f32) -> HitShape {
    let (a, b) = (quad[side], quad[(side + 1) % 4]);
    let Some(out) = outward(quad, side) else { return HitShape::segment(a, b) };
    let along = (b - a).normalized() * reach;
    let across = (quad[(side + 2) % 4] - b).length();
    let (inner, outer) = (out * -reach.min(across / 4.0).max(SIDE_LINE_INSIDE), out * reach);
    HitShape::Polygon(vec![a - along + inner, b + along + inner, b + along + outer, a - along + outer])
}

/// The stretch of side `side`'s band next to its end `corner`: a quarter of the side, at most `width` along it, as far
/// out as the band. `None` for a side with no length.
fn corner_reach(quad: &[Pos2; 4], side: usize, corner: usize, width: f32) -> Option<HitShape> {
    let out = outward(quad, side)?;
    let other = if corner == side { quad[(side + 1) % 4] } else { quad[side] };
    let from = quad[corner];
    let along = other - from;
    let reach = (along.length() * 0.25).min(width);
    Some(band_along(from, from + along.normalized() * reach, out, width))
}

/// The unit normal pointing out of the box `quad` from side `side`, `None` for a side with no length.
fn outward(quad: &[Pos2; 4], side: usize) -> Option<Vec2> {
    let along = quad[(side + 1) % 4] - quad[side];
    if along.length() < 1e-3 {
        return None;
    }
    // The winding tells outside from inside, as `quad::outset` reads it.
    let area: f32 = (0..4).map(|i| quad[i].x * quad[(i + 1) % 4].y - quad[(i + 1) % 4].x * quad[i].y).sum();
    let d = along.normalized();
    Some(Vec2::new(d.y, -d.x) * if area >= 0.0 { 1.0 } else { -1.0 })
}

/// The band from `a` to `b` reaching `width` out along `out`, with the line itself: half a point inside the line
/// still counts as on it.
fn band_along(a: Pos2, b: Pos2, out: Vec2, width: f32) -> HitShape {
    let (inner, outer) = (out * -SIDE_LINE_INSIDE, out * width);
    HitShape::Polygon(vec![a + inner, b + inner, b + outer, a + outer])
}

/// How far inside the box a press still lies on a side's line, in screen points.
const SIDE_LINE_INSIDE: f32 = 0.5;

/// `polygon` clipped to `rect` (Sutherland-Hodgman, one rect side at a time).
fn clipped_to(polygon: &[Pos2], rect: Rect) -> Vec<Pos2> {
    // How far a point lies inside each side of the rect.
    let sides: [fn(Pos2, Rect) -> f32; 4] = [|p, r| p.x - r.min.x, |p, r| r.max.x - p.x, |p, r| p.y - r.min.y, |p, r| r.max.y - p.y];
    let mut out = polygon.to_vec();
    for inside in sides {
        let input = std::mem::take(&mut out);
        for (k, &here) in input.iter().enumerate() {
            let before = input[(k + input.len() - 1) % input.len()];
            let (a, b) = (inside(before, rect), inside(here, rect));
            if (a >= 0.0) != (b >= 0.0) {
                out.push(before + (here - before) * (a / (a - b)));
            }
            if b >= 0.0 {
                out.push(here);
            }
        }
        if out.is_empty() {
            break;
        }
    }
    out
}

/// The rotate band at each corner of `screen`: the L-shaped ring outside the box between the band's inner and outer
/// reach, as long as the outer reach along both edges.
fn band_rings(screen: &BoxState, style: &TransformBoxStyle) -> [Vec<Pos2>; 4] {
    let corners: [Pos2; 4] = screen.quad.map(pos);
    let inner = quad::outset(&corners, style.band_inner);
    let outer = quad::outset(&corners, style.band_outer);
    std::array::from_fn(|i| {
        let toward = |k: usize| (corners[k % 4] - corners[i]).normalized() * style.band_outer;
        let (previous, next) = (toward(i + 3), toward(i + 1));
        vec![inner[i] + previous, outer[i] + previous, outer[i], outer[i] + next, inner[i] + next, inner[i]]
    })
}

/// `state` with its corners on whole units, when it is an upright rectangle of at least one unit each way;
/// otherwise as it is. The reference point stays where it is.
fn on_pixel_grid(state: BoxState) -> BoxState {
    let q = state.quad;
    let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
    let upright = near(q[0].y, q[1].y) && near(q[2].y, q[3].y) && near(q[0].x, q[3].x) && near(q[1].x, q[2].x);
    let rounded = q.map(|p| Point::new(p.x.round(), p.y.round()));
    let ok = rounded.iter().all(|p| p.x.is_finite() && p.y.is_finite())
        && (rounded[1].x - rounded[0].x).abs() >= 1.0
        && (rounded[3].y - rounded[0].y).abs() >= 1.0;
    if upright && ok { BoxState::new(rounded, state.pivot) } else { state }
}

/// `v` rounded to `places` decimals, with no negative zero (a hair below zero never reads "-0").
pub(crate) fn fixed(v: f64, places: i32) -> f64 {
    let k = 10f64.powi(places);
    let v = (v * k).round() / k;
    if v == 0.0 { 0.0 } else { v }
}
