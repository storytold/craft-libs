//! The divider that `SplitArea`, `DockColumn` and `SplitViews` share: its hit part, its pointer drag, its keys and
//! its accessibility node. A container describes each divider as a [`Divider`] in reading order and applies the
//! coordinate the drag or the keys ask for; this module owns everything that is the same for all of them, including
//! the mirroring for right-to-left. A component that draws its own dividers (a column bar, a margin handle) calls
//! [`declare`] in its parts, [`pointer`] and [`keys`] in its update, and [`describe`] in its access, and declares `KeyUse::SPLITTER` for the arrow keys.

use crate::contract::{Access, Component, Cx, Focus as _, Name, Numeric, PartsCx, Pointer as _, Role};
use craft_behavior::dock::DockEdge;
use craft_behavior::nav::{StepResult, ValueAxes, ValueSteps, step_value};
use craft_behavior::{Axis, CursorIcon, Flow, HitLevel, HitPriority, NavMove, PartKey, PressPhase, Sense};
use egui::{Pos2, Rect};

/// A divider as its container lays it out, in reading order.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Divider {
    /// The axis the panes it separates follow: `X` for panes side by side, so the divider is vertical.
    pub axis: Axis,
    /// The strip across the boundary that takes the pointer.
    pub bar: Rect,
    /// The boundary's coordinate along `axis`.
    pub at: f32,
    /// The least and most coordinate the boundary can take.
    pub lo: f32,
    /// See `lo`.
    pub hi: f32,
}

impl Divider {
    /// A divider at `at` along `axis` across `span`, with a bar `reach` thick, that moves between `lo` and `hi`.
    pub fn new(axis: Axis, at: f32, span: Rect, (lo, hi): (f32, f32), reach: f32) -> Self {
        let half = reach / 2.0;
        let bar = match axis {
            Axis::X => Rect::from_min_max(Pos2::new(at - half, span.top()), Pos2::new(at + half, span.bottom())),
            _ => Rect::from_min_max(Pos2::new(span.left(), at - half), Pos2::new(span.right(), at + half)),
        };
        Divider { axis, bar, at, lo, hi }
    }

    /// The divider on the inner edge of the column in `rect` on the dock edge `edge`, in reading order: the edge that
    /// faces away from the window edge, which moves between the coordinates of the column's `least` and `most` widths.
    /// Its bar stays inside `rect`.
    pub(crate) fn column_edge(rect: Rect, edge: DockEdge, (least, most): (f32, f32), reach: f32) -> Self {
        let (at, lo, hi) = match edge {
            DockEdge::End => (rect.left(), rect.right() - most, rect.right() - least),
            _ => (rect.right(), rect.left() + least, rect.left() + most),
        };
        let mut divider = Divider::new(Axis::X, at, rect, (lo, hi), reach);
        divider.bar = divider.bar.intersect(rect);
        divider
    }

    /// The width of the column in `rect` on `edge` when its inner edge is at `coord`.
    pub(crate) fn column_width(rect: Rect, edge: DockEdge, coord: f32) -> f32 {
        match edge {
            DockEdge::End => rect.right() - coord,
            _ => coord - rect.left(),
        }
    }

    fn cursor(&self) -> CursorIcon {
        if self.axis == Axis::X { CursorIcon::ResizeHorizontal } else { CursorIcon::ResizeVertical }
    }
}

/// What the pointer did to a divider this pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drag {
    /// The drag started.
    Start,
    /// The pointer is at this coordinate along the divider's axis, in reading order.
    To(f32),
    /// The pointer was released.
    Release,
}

/// `x` as a coordinate from the start edge of `rect`: unchanged for left-to-right, mirrored for right-to-left.
fn logical_x(rect: Rect, rtl: bool, x: f32) -> f32 {
    if rtl { rect.right() - (x - rect.left()) } else { x }
}

/// A vertical move read as the move of a value that grows downwards.
fn along_y(mv: NavMove) -> NavMove {
    match mv {
        NavMove::Up => NavMove::Down,
        NavMove::Down => NavMove::Up,
        NavMove::PageUp => NavMove::PageDown,
        NavMove::PageDown => NavMove::PageUp,
        NavMove::Forward => NavMove::Up,
        NavMove::Back => NavMove::Down,
        other => other,
    }
}

/// Declares the part `key.at(index)` for `divider`: an edge part above the panes that takes drags and is its own Tab
/// stop. `rect` is the container's rect, which the bar is mirrored across for right-to-left.
pub fn declare<C: Component>(cx: &mut PartsCx<'_, C>, rect: Rect, key: PartKey, index: u64, divider: &Divider) {
    let bar = Flow::new(rect, cx.flow.is_rtl()).to_physical(divider.bar);
    cx.part(key.at(index), bar)
        .sense(Sense::DRAG | Sense::TAKE_FOCUS)
        .priority(HitPriority::new(HitLevel::Edge, 0))
        .cursor(divider.cursor())
        .focusable()
        .over();
}

/// What the pointer did to the divider `key.at(index)` this pass.
pub fn pointer<C: Component>(cx: &Cx<'_, C>, key: PartKey, index: u64, divider: &Divider) -> Option<Drag> {
    drag(cx.part(key.at(index)).press.phase, cx.rect, cx.flow.is_rtl(), divider)
}

/// What the press phase of a divider's part says the pointer did to `divider`, for a container `rect` that is
/// mirrored for right-to-left. A body reads the phase from `BodyCx::part_press`.
pub fn drag(phase: PressPhase, rect: Rect, rtl: bool, divider: &Divider) -> Option<Drag> {
    match phase {
        PressPhase::DragStart { .. } => Some(Drag::Start),
        PressPhase::Drag { pos, .. } => Some(Drag::To(if divider.axis == Axis::X { logical_x(rect, rtl, pos.x) } else { pos.y })),
        PressPhase::Release { .. } => Some(Drag::Release),
        _ => None,
    }
}

/// The coordinate the keys or assistive technology ask the focused divider of `key` to take, with its index: an arrow
/// press steps by `step`, Shift or Page by `jump`, and a set value is taken as it is. Takes the intent only when one
/// of `dividers` (each with its index) is focused.
pub fn keys<C: Component>(cx: &mut Cx<'_, C>, key: PartKey, dividers: &[(u64, Divider)], step: f32, jump: f32) -> Option<(u64, f32)> {
    let focused = cx.active_part().filter(|part| part.key == key).and_then(|part| part.index)?;
    let (_, divider) = dividers.iter().find(|(index, _)| *index == focused)?;
    let value = if let Some(set) = cx.intents().take_set_value() {
        set.clamp(f64::from(divider.lo), f64::from(divider.hi))
    } else if let Some((mv, mods)) = cx.intents().take_move() {
        let (axes, mv) = match divider.axis {
            Axis::X => (ValueAxes::Horizontal, mv),
            _ => (ValueAxes::Vertical, along_y(mv)),
        };
        let steps = ValueSteps::new(f64::from(divider.at), f64::from(divider.lo), f64::from(divider.hi), f64::from(step), f64::from(jump));
        match step_value(&steps, axes, mv, mods) {
            StepResult::Set(value) => value,
            _ => return None,
        }
    } else {
        return None;
    };
    Some((focused, value as f32))
}

/// The accessibility node of the divider `key.at(index)`: a splitter named `name` whose value is its coordinate.
pub fn describe(access: &mut Access, key: PartKey, index: u64, divider: &Divider, name: &str, step: f32) {
    let node = access.node(key.at(index), Role::SPLITTER);
    node.name = Name::Text(name.to_owned());
    node.numeric = Some(Numeric::new(f64::from(divider.at), f64::from(divider.lo)..=f64::from(divider.hi)).step(f64::from(step)));
}
