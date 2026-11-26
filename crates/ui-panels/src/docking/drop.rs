//! Drop resolution for a dock: what a dragged panel or group becomes when released on the spot under the pointer.
//!
//! A dock registers `DockSpot`s with `DockTargets`; [`resolve_drop`] reads the spot under the pointer and maps it to
//! the dock model's [`Action`], naming groups by an anchor panel. [`DockDropTarget`] runs the same resolution inside
//! `DropTarget::accept`, refuses what the model would refuse, so the drag session arbitrates between docks and
//! windows, and returns the mark the ghost paints. Every drag of a group, a window or a column starts through
//! [`start_group_drag`], or [`start_icon_drag`] out of a collapsed column.

use super::{DockSpot, DockTargets, FloatSpace, GroupDrag, PanelDrag, PanelKey, Placing, WindowColumn, placed_rect};
use crate::contract::{Component, Cx, DropMark, DropOperation, DropTarget, Pointer as _};
use crate::runtime::drag::{self, DragPreview};
use crate::runtime::env::{AppValues, EnvValue, EnvView, Label};
use crate::runtime::pointer;
use craft_behavior::dock::{Action, DockEdge, DockPlacement, Layout, Node, Permissions, Target};
use craft_behavior::{CancelReason, DockSide};
use egui::{Context, Id, Pos2, Rect, Vec2};
use std::hash::Hash;

/// What a dock drag moves: one panel, or the whole group, floating window or column of a panel.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DragUnit<P> {
    /// The panel alone.
    Panel(P),
    /// The group that holds the panel, named by it.
    Group(P),
    /// The floating window that holds the panel: every group in it moves, and the window moves itself while the
    /// drag runs.
    Window(P),
    /// The dock column that holds the panel.
    Column(P),
}

impl<P> DragUnit<P> {
    /// The panel that names the unit.
    pub fn lead(&self) -> &P {
        match self {
            DragUnit::Panel(panel) | DragUnit::Group(panel) | DragUnit::Window(panel) | DragUnit::Column(panel) => panel,
        }
    }
}

/// A drag payload a dock resolves drops for.
pub trait DockDrag<P> {
    /// What the drag moves in `layout`; `None` when the dragged panel is not in it.
    fn unit(&self, layout: &Layout<P>) -> Option<DragUnit<P>>;
    /// The size of what the drag left where it started, which a float takes; `None` floats it at its own or the
    /// default size.
    fn source(&self) -> Option<Vec2> {
        None
    }
    /// The size of the icons of a collapsed column the drag started on, whose float lands collapsed with its strip
    /// under the pointer; `None` for any other drag.
    fn strip(&self) -> Option<Vec2> {
        None
    }
    /// Where the press that started the drag was, when a drop that moves a floating window keeps the grabbed point
    /// under the pointer; `None` puts the window's title row under it.
    fn press(&self) -> Option<Pos2> {
        None
    }
    /// The drag is a floating window's move by its title row ([`GroupDrag::title_row`]).
    fn title_row(&self) -> bool {
        false
    }
}

impl<P: Clone + Eq + Hash> DockDrag<P> for PanelDrag {
    fn unit(&self, layout: &Layout<P>) -> Option<DragUnit<P>> {
        layout.panels().into_iter().find(|panel| PanelKey::of(*panel) == self.panel).cloned().map(DragUnit::Panel)
    }

    fn source(&self) -> Option<Vec2> {
        self.source
    }
}

impl<P: Clone> DockDrag<P> for GroupDrag<P> {
    fn unit(&self, _layout: &Layout<P>) -> Option<DragUnit<P>> {
        Some(self.unit.clone())
    }

    fn source(&self) -> Option<Vec2> {
        self.source
    }

    fn strip(&self) -> Option<Vec2> {
        self.strip
    }

    fn press(&self) -> Option<Pos2> {
        self.press
    }

    fn title_row(&self) -> bool {
        self.title_row
    }
}

/// What a drop does and what the ghost paints for it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DockDropped<P> {
    /// The change the drop makes to the layout.
    pub action: Action<P>,
    /// Where the dragged unit would land.
    pub mark: DropMark,
}

/// The panels of the group, tab node or window frame that holds `anchor`.
pub(crate) fn members<P: Clone + Eq + Hash>(layout: &Layout<P>, anchor: &P) -> Vec<P> {
    let Some(place) = layout.place(anchor) else { return vec![anchor.clone()] };
    if let Some(group) = layout.group(&place) {
        return group.panels.clone();
    }
    match layout.node_of(&place) {
        Some(Node::Tabs { panels, .. }) => panels.clone(),
        _ => vec![anchor.clone()],
    }
}

/// The panels the unit moves.
pub(crate) fn moving_panels<P: Clone + Eq + Hash>(layout: &Layout<P>, unit: &DragUnit<P>) -> Vec<P> {
    match unit {
        DragUnit::Panel(panel) => vec![panel.clone()],
        DragUnit::Group(anchor) => members(layout, anchor),
        DragUnit::Window(anchor) => match layout.floating.iter().find(|window| window.contains(anchor)) {
            Some(window) => window.panels().cloned().collect(),
            None => members(layout, anchor),
        },
        DragUnit::Column(anchor) => match layout.place(anchor).and_then(|place| layout.column(&place)) {
            Some(column) => column.groups.iter().flat_map(|group| group.panels.iter().cloned()).collect(),
            None => members(layout, anchor),
        },
    }
}

/// A panel of the group of `named` that is not moving, so a drop on the unit's own group names a destination the
/// move does not take away.
fn outside<P: Clone + Eq + Hash>(layout: &Layout<P>, named: &P, moved: &[P]) -> Option<P> {
    if !moved.contains(named) {
        return Some(named.clone());
    }
    members(layout, named).into_iter().find(|panel| !moved.contains(panel))
}

pub(crate) fn beside<P: Clone>(unit: &DragUnit<P>, anchor: P, placement: DockPlacement<P>) -> Action<P> {
    match unit {
        DragUnit::Panel(panel) => Action::move_beside(panel.clone(), anchor, placement),
        DragUnit::Group(lead) => Action::move_group(lead.clone(), Target::at(anchor, placement)),
        DragUnit::Window(lead) => Action::move_window(lead.clone(), Target::at(anchor, placement)),
        DragUnit::Column(lead) => Action::move_column(lead.clone(), Target::at(anchor, placement)),
    }
}

pub(crate) fn to_target<P: Clone>(unit: &DragUnit<P>, target: Target<P>) -> Action<P> {
    match unit {
        DragUnit::Panel(panel) => Action::move_panel(panel.clone(), target),
        DragUnit::Group(lead) => Action::move_group(lead.clone(), target),
        DragUnit::Window(lead) => Action::move_window(lead.clone(), target),
        DragUnit::Column(lead) => Action::move_column(lead.clone(), target),
    }
}

/// The layout after `action`, when the model takes it and it changes the arrangement: a dry run on a copy, so no mark
/// shows for a drop that `Layout::apply_with_permissions` would refuse or that moves nothing.
pub(crate) fn applied<P: Clone + Eq + Hash>(layout: &Layout<P>, action: &Action<P>, permissions: &dyn Fn(&P) -> Permissions) -> Option<Layout<P>> {
    let mut copy = layout.clone();
    (copy.apply_with_permissions(action.clone(), permissions).is_ok() && !copy.same_arrangement(layout)).then_some(copy)
}

/// Whether the model takes `action` and changes the arrangement by it ([`applied`]).
pub(crate) fn takes<P: Clone + Eq + Hash>(layout: &Layout<P>, action: &Action<P>, permissions: &dyn Fn(&P) -> Permissions) -> bool {
    applied(layout, action, permissions).is_some()
}

/// The action a drop of `unit` on `spot` makes and the layout after it, or `None` when the unit cannot move there: the
/// permissions refuse, the destination is the unit itself, the spot has no placement, or the model refuses the action.
fn action_for<P: Clone + Eq + Hash>(
    layout: &Layout<P>,
    spot: &DockSpot<P>,
    unit: &DragUnit<P>,
    permissions: &dyn Fn(&P) -> Permissions,
) -> Option<(Action<P>, Layout<P>)> {
    let action = spot_action(layout, spot, unit, permissions)?;
    let after = applied(layout, &action, permissions)?;
    Some((action, after))
}

fn spot_action<P: Clone + Eq + Hash>(
    layout: &Layout<P>,
    spot: &DockSpot<P>,
    unit: &DragUnit<P>,
    permissions: &dyn Fn(&P) -> Permissions,
) -> Option<Action<P>> {
    if !permissions(unit.lead()).movable {
        return None;
    }
    let moved = moving_panels(layout, unit);
    let place = |anchor: P, placement: DockPlacement<P>| placement.allowed_by(permissions(&anchor)).then(|| beside(unit, anchor, placement));
    match spot {
        DockSpot::Tab { anchor, index } => {
            let destination = outside(layout, anchor, &moved)?;
            let before = members(layout, anchor).into_iter().skip(*index).find(|panel| !moved.contains(panel));
            place(destination, DockPlacement::tab(before))
        }
        DockSpot::Group { anchor, side } => {
            if *side == DockSide::Center && members(layout, anchor).iter().any(|panel| moved.contains(panel)) {
                return None;
            }
            let destination = outside(layout, anchor, &moved)?;
            let placement = DockPlacement::for_zone(layout, &destination, *side).ok()?;
            place(destination, placement)
        }
        DockSpot::Edge(side) => {
            if *side == DockSide::Center {
                return None;
            }
            let protected = |panel: &P| permissions(panel).is_protected();
            let skip = |panel: &P| moved.contains(panel);
            let anchor = layout.edge_anchor(layout.edge_narrows(*side, &protected), &protected, &skip)?;
            Some(beside(unit, anchor, DockPlacement::Edge(*side)))
        }
        DockSpot::NewDock(side) => {
            let edge = match side {
                DockSide::Start => DockEdge::Start,
                DockSide::End => DockEdge::End,
                _ => return None,
            };
            Some(to_target(unit, Target::new_dock(edge)))
        }
        DockSpot::Seam { before, after } => match outside(layout, before, &moved) {
            Some(anchor) => place(anchor, DockPlacement::group(true, None)),
            None => place(outside(layout, after, &moved)?, DockPlacement::group(false, None)),
        },
        DockSpot::Column { anchor, side } => {
            let destination = outside(layout, anchor, &moved)?;
            let placement =
                DockPlacement::for_zone(layout, &destination, *side).ok().filter(|placement| matches!(placement, DockPlacement::Column { .. }))?;
            place(destination, placement)
        }
        DockSpot::Blocked => None,
    }
}

/// The action that floats `unit` in a window under `at`, inside the area of `space`, at [`FloatSpace::size_of`] with
/// `source` the size the unit had where the drag started, and the outline the ghost paints for it. The pointer sits in
/// the middle of the window's title row.
///
/// A unit dragged out of a collapsed column (`strip`, the size of the icons the drag started on) floats collapsed:
/// its strip, under the title row, lands with the pointer in the middle of the title row above it, the window keeps
/// to the edge of the area nearer the strip, and its rect shows the strip there ([`WindowColumn::around_strip`]);
/// the outline is the strip's. `rtl` is the layout direction.
pub fn float_drop<P: Clone + PartialEq>(
    unit: &DragUnit<P>,
    space: &FloatSpace<P>,
    at: Pos2,
    source: Option<Vec2>,
    strip: Option<Vec2>,
    rtl: bool,
) -> DockDropped<P> {
    let size = space.size_of(unit, source).min(space.area.size());
    let under = |size: Vec2| under_pointer(space, at, size);
    let (rect, edge, mark) = match strip.filter(|_| !matches!(unit, DragUnit::Window(_))) {
        Some(strip) => {
            let shown = under((strip + Vec2::new(0.0, space.chrome)).min(space.area.size()));
            let (rect, edge) = WindowColumn::around_strip(shown, size, space.area, rtl);
            (rect, Some(edge), shown)
        }
        None => {
            let rect = under(size);
            (rect, None, rect)
        }
    };
    let action = match unit {
        DragUnit::Panel(panel) if edge.is_none() => Action::float(panel.clone(), rect),
        _ => to_target(unit, Target::float_with_edge(rect, edge)),
    };
    DockDropped { action, mark: DropMark::region(mark) }
}

/// A floating window of `size` under the pointer at `at`: its title row's middle at the pointer, inside the float area.
fn under_pointer<P>(space: &FloatSpace<P>, at: Pos2, size: Vec2) -> Rect {
    let min = (at - Vec2::new(size.x * 0.5, space.chrome * 0.5)).min(space.area.max - size).max(space.area.min);
    Rect::from_min_size(min, size)
}

/// The action a release of `drag` at `at` makes in the dock `dock`, by the spots of the previous pass; `None` where
/// no spot takes the drag. Call it from the dock's `DropTarget::accept` only (see `DockTargets::resolve`).
pub fn resolve_drop<P, D>(
    ctx: &Context,
    dock: Id,
    layout: &Layout<P>,
    at: Pos2,
    drag: &D,
    permissions: &dyn Fn(&P) -> Permissions,
) -> Option<Action<P>>
where
    P: Clone + Eq + Hash + Send + 'static,
    D: DockDrag<P>,
{
    let unit = drag.unit(layout)?;
    let (spot, _) = DockTargets::<P>::resolve(ctx, dock, at)?;
    action_for(layout, &spot, &unit, permissions).map(|(action, _)| action)
}

/// The drop target of one dock over one layout, for a tab drag ([`PanelDrag`]) and a group drag ([`GroupDrag`]).
///
/// It accepts a move only. A release over a spot docks the unit there. With `placing`, the mark over a spot is the
/// region the unit takes once dropped, laid out in the room the dock registered ([`placed_rect`]); a tab insertion
/// marks the group it joins as shown now, with its tab line; a reorder inside the unit's own group, and a drop whose panels get no rect of their own, mark
/// the spot ([`DockSpot::mark`]). Ctrl or Cmd held at the release floats it
/// instead (Photoshop: the key prevents docking), and with `float_on_empty` a release that no spot takes floats it;
/// both float it into `space` under the pointer. A floating window dragged by its title bar moves itself and never
/// floats into another window. Fields are assigned by the dock after [`Self::new`].
#[non_exhaustive]
pub struct DockDropTarget<'a, P> {
    ctx: &'a Context,
    /// The dock whose spots resolve the pointer.
    pub dock: Id,
    layout: &'a Layout<P>,
    permissions: &'a dyn Fn(&P) -> Permissions,
    /// The layout direction, which mirrors side marks.
    pub rtl: bool,
    /// Where a floating drop lands. Without it the target only docks.
    pub space: Option<&'a FloatSpace<P>>,
    /// A release that no spot takes floats the unit, except over the unit's own group (`DockConfig::float_on_empty_drop`).
    pub float_on_empty: bool,
    /// A floating window dragged by its title bar docks on a spot (`DockConfig::title_bar_docks`). Off: it only moves.
    pub title_bar_docks: bool,
    /// How the dock lays out its panels, for the region a drop gives them. Without it every mark is the spot's.
    pub placing: Option<Placing<'a, P>>,
}

impl<'a, P> DockDropTarget<'a, P> {
    /// The target of `dock` over `layout`: it docks on spots and floats nothing until `space` is set.
    pub fn new(ctx: &'a Context, dock: Id, layout: &'a Layout<P>, permissions: &'a dyn Fn(&P) -> Permissions) -> Self {
        DockDropTarget { ctx, dock, layout, permissions, rtl: false, space: None, float_on_empty: false, title_bar_docks: true, placing: None }
    }
}

impl<P: Clone + Eq + Hash + Send + 'static> DockDropTarget<'_, P> {
    fn take<D: DockDrag<P>>(&self, drag: &D, at: Pos2, op: DropOperation) -> Option<DockDropped<P>> {
        if !matches!(op, DropOperation::Move) {
            return None;
        }
        let unit = drag.unit(self.layout)?;
        let window = matches!(unit, DragUnit::Window(_));
        if pointer::mods(self.ctx).cmd {
            return if window { None } else { self.float(&unit, at, drag.source(), drag.strip(), drag.press()) };
        }
        if window && drag.title_row() && !self.title_bar_docks {
            return None;
        }
        if let Some((spot, applies)) = DockTargets::<P>::resolve(self.ctx, self.dock, at) {
            if let Some((action, after)) = action_for(self.layout, &spot, &unit, self.permissions) {
                let mark = match self.region(&spot, &unit, &after) {
                    Some(area) => match spot.tab_line(applies, self.rtl) {
                        Some([from, to]) => DropMark::region_at(area, from, to),
                        None => DropMark::region(area),
                    },
                    None => spot.mark(applies, self.rtl)?,
                };
                return Some(DockDropped { action, mark });
            }
            if on_unit(self.layout, &spot, &unit) {
                return None;
            }
        }
        if window || !self.float_on_empty {
            return None;
        }
        self.float(&unit, at, drag.source(), drag.strip(), drag.press())
    }

    /// The region a drop of `unit` on `spot` marks: for a tab join, the group it joins as shown now (production's
    /// mark: the group under the pointer, not the size it grows to); else the rect the panels of `unit` take in
    /// `after`, the layout once dropped. `None` without `placing` or a registered room, for a reorder inside the
    /// unit's own group, which marks only its tab line, and where the panels get no rect of their own.
    fn region(&self, spot: &DockSpot<P>, unit: &DragUnit<P>, after: &Layout<P>) -> Option<Rect> {
        let placing = self.placing.as_ref()?;
        if let DockSpot::Tab { anchor, .. } = spot {
            if on_unit(self.layout, spot, unit) {
                return None;
            }
            let group = members(self.layout, anchor);
            if let Some(rect) = DockTargets::<P>::group_rect(self.ctx, self.dock, |panel| group.contains(panel)) {
                return Some(rect);
            }
        }
        let room = DockTargets::<P>::room(self.ctx, self.dock)?;
        placed_rect(after, &moving_panels(self.layout, unit), room, placing, self.rtl)
    }

    /// The unit floated under `at`, when it may float and the model takes it; `source` is its size where the drag
    /// started, `strip` the size of the icons it started on ([`float_drop`]).
    ///
    /// A unit that is the whole of a floating window already (a floating tool rail dragged by its grip,
    /// photocraft#16426) moves that window under `at`: it is floating, so floating it again would be refused.
    fn float(&self, unit: &DragUnit<P>, at: Pos2, source: Option<Vec2>, strip: Option<Vec2>, press: Option<Pos2>) -> Option<DockDropped<P>> {
        let space = self.space?;
        if let Some(window) = self.layout.floating.iter().find(|window| window.contains(unit.lead()))
            && window.panels().all(|panel| moving_panels(self.layout, unit).contains(panel))
        {
            // The point grabbed stays under the pointer, inside the float area; without the press, the title row.
            let rect = match press {
                Some(press) => {
                    let moved = window.rect.translate(at - press);
                    Rect::from_min_size(moved.min.min(space.area.max - moved.size()).max(space.area.min), moved.size())
                }
                None => under_pointer(space, at, space.size_of(unit, source).min(space.area.size())),
            };
            let moved = Action::move_floating(unit.lead().clone(), rect);
            // A move keeps the arrangement (`takes` asks for a change of it): the model need only take the action.
            let taken = self.layout.clone().apply_with_permissions(moved.clone(), self.permissions).is_ok();
            return ((self.permissions)(unit.lead()).movable && taken).then_some(DockDropped { action: moved, mark: DropMark::region(rect) });
        }
        let floated = float_drop(unit, space, at, source, strip, self.rtl);
        ((self.permissions)(unit.lead()).floatable && takes(self.layout, &floated.action, self.permissions)).then_some(floated)
    }
}

/// Whether `spot` lies on a group that `unit` moves panels out of: a release there that docks nowhere leaves the unit
/// where it is.
fn on_unit<P: Clone + Eq + Hash>(layout: &Layout<P>, spot: &DockSpot<P>, unit: &DragUnit<P>) -> bool {
    let moved = moving_panels(layout, unit);
    let touches = |anchor: &P| members(layout, anchor).iter().any(|panel| moved.contains(panel));
    match spot {
        DockSpot::Tab { anchor, .. } | DockSpot::Group { anchor, .. } | DockSpot::Column { anchor, .. } => touches(anchor),
        DockSpot::Seam { before, after } => touches(before) || touches(after),
        _ => false,
    }
}

impl<P: Clone + Eq + Hash + Send + 'static> DropTarget<PanelDrag> for DockDropTarget<'_, P> {
    type Drop = DockDropped<P>;

    fn accept(&self, payload: &PanelDrag, at: Pos2, op: DropOperation) -> Option<DockDropped<P>> {
        self.take(payload, at, op)
    }

    fn mark(&self, drop: &DockDropped<P>) -> DropMark {
        drop.mark
    }
}

impl<P: Clone + Eq + Hash + Send + 'static> DropTarget<GroupDrag<P>> for DockDropTarget<'_, P> {
    type Drop = DockDropped<P>;

    fn accept(&self, payload: &GroupDrag<P>, at: Pos2, op: DropOperation) -> Option<DockDropped<P>> {
        self.take(payload, at, op)
    }

    fn mark(&self, drop: &DockDropped<P>) -> DropMark {
        drop.mark
    }
}

/// The arrangement a dock drag started on: the dock area and the generation of the workspace it showed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, crate::contract::MemoProps)]
#[doc(hidden)]
pub struct Arrangement {
    pub dock: Id,
    pub generation: u64,
}

/// The env value a dock area puts into its body: the arrangement it shows. `None` outside a dock area.
#[derive(Clone, Copy, PartialEq, crate::contract::MemoProps)]
struct InDock(Option<Arrangement>);

impl EnvValue for InDock {
    fn root() -> Self {
        InDock(None)
    }
}

/// Publishes the arrangement a dock area shows to its body, where every tab and group drag records it.
pub(crate) fn publish(values: &mut AppValues, shown: Option<Arrangement>) {
    values.set(InDock(shown));
}

/// The arrangement of the dock area around the component that reads `env`, which a tab or group drag started there
/// records.
pub(crate) fn started(env: &EnvView<'_>) -> Option<Arrangement> {
    env.get::<InDock>().0
}

/// Starts the drag of `unit`, whose shown panel is `active` and whose size where it starts is `source`. A group or a
/// column shows `name` inside the region its drop would take ([`DragPreview::Named`]). A window moves itself, so its
/// drag shows nothing and `name` is `None`.
pub fn start_group_drag<P, C>(cx: &mut Cx<'_, C>, unit: DragUnit<P>, active: PanelKey, source: Option<Vec2>, name: Option<Label>)
where
    P: Clone + Send + Sync + 'static,
    C: Component,
{
    let preview = name.map_or(DragPreview::None, DragPreview::Named);
    let started = started(&cx.env);
    cx.start_drag(GroupDrag { unit, active, source, strip: None, press: None, started, title_row: false }, preview);
}

/// Starts the drag of the floating window that holds `anchor`, whose shown panel is `active`, at its `size`: a move by
/// its title row when `title_row`, else a panel drag the window took over ([`GroupDrag::title_row`]).
pub(crate) fn start_window_drag<P, C>(cx: &mut Cx<'_, C>, anchor: P, active: PanelKey, size: Vec2, title_row: bool)
where
    P: Clone + Send + Sync + 'static,
    C: Component,
{
    let started = started(&cx.env);
    let drag = GroupDrag { unit: DragUnit::Window(anchor), active, source: Some(size), strip: None, press: None, started, title_row };
    cx.start_drag(drag, DragPreview::None);
}

/// Starts the drag of `unit` out of a collapsed column, from its icons of size `strip`, whose shown panel is
/// `active`: the region its drop would take shows `label`, and a float of the unit lands collapsed with its strip under
/// the pointer ([`float_drop`]).
pub fn start_icon_drag<P, C>(cx: &mut Cx<'_, C>, unit: DragUnit<P>, active: PanelKey, strip: Vec2, label: Label)
where
    P: Clone + Send + Sync + 'static,
    C: Component,
{
    let started = started(&cx.env);
    cx.start_drag(GroupDrag { unit, active, source: None, strip: Some(strip), press: None, started, title_row: false }, DragPreview::Named(label));
}

/// Ends a running tab or group drag that started on `shown`'s dock under another generation: another arrangement
/// replaced the workspace, and the panel or group the drag names may be gone or elsewhere. The dock's own edits keep
/// the generation, and so the drag.
#[doc(hidden)]
pub fn end_replaced_drags<P: Send + Sync + 'static>(ctx: &Context, shown: Arrangement) {
    let replaced = |started: Option<Arrangement>| started.is_some_and(|started| started.dock == shown.dock && started != shown);
    if drag::active::<PanelDrag>(ctx).is_some_and(|drag| replaced(drag.started))
        || drag::active::<GroupDrag<P>>(ctx).is_some_and(|drag| replaced(drag.started))
    {
        drag::cancel(ctx, CancelReason::HostChanged);
    }
}
