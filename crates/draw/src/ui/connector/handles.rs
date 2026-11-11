//! [`ConnectorHandles`]: the ends and bends of the selected connector, a canvas control. Its parts are
//! in the canvas's arena, so a press on an end or a bend stays with the handles and one elsewhere
//! reaches the Connector tool.

use craft_behavior::geometry::draft::turn_to_45;
use craft_behavior::{Axis, CancelReason, CursorIcon, HitLevel, HitPriority, HitShape, Metrics, NavShape, PartKey, PressPhase, Roving, Sense};
use craft_keymap::Mods;
use craft_ui::canvas::{CanvasSpace, NUDGE, NUDGE_FAR, canvas_snap, canvas_space};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Name, Outcome, PaintUi, PartsCx, Pointer as _,
    Props as _, Remember as _, Role, WordSetCx as _,
};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::KeyUse;
use craft_ui::style::{ComponentStyle, StyleCx};
use egui::{Pos2, Rect, Response, Vec2};
use kurbo::Point;

use super::overlay::{ConnectorLook, paint_handles, paint_route, paint_sites};
use super::place::{Placed, end_at, geometry, glued, stepped_end, visible_bends};
use super::words::ConnectorWords;
use super::{ConnectionTarget, ConnectorEnd, ConnectorEndKind, ConnectorEvent, ConnectorItem};
use crate::geometry::connector::{RouteEnd, reshaped};

/// An end of the connector: `END.at(0)` is the start, `END.at(1)` the finish.
pub(super) const END: PartKey = PartKey::new("end");

/// A bend of the connector: `BEND.at(portion)`, the portion [`reshaped`] takes.
const BEND: PartKey = PartKey::new("bend");

/// The style of the connector handles.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ConnectorHandlesStyle {
    #[style(metrics)]
    metrics: Metrics,
    look: ConnectorLook,
    /// How far from a site or a side of a target an end still glues to it.
    reach: f32,
}

/// What a drag started from: the connector as it was when the press went down.
#[derive(Clone, Default)]
struct Start(Option<ConnectorItem>);

/// The operation of the last pass of a drag, which the release pass reports again.
#[derive(Clone, Default)]
struct Last(Option<ConnectorEvent>);

/// What [`ConnectorHandles`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ConnectorHandlesOutput {
    /// The edit of the connector: the gesture's phases, with the connector as the drag would leave it.
    pub edit: EditResponse<ConnectorItem>,
    /// The operation this pass, for an app that keeps connectors in its own type.
    pub event: Option<ConnectorEvent>,
}

impl Outcome for ConnectorHandlesOutput {
    type Value = ConnectorItem;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// The ends and bends of the selected connector.
///
/// - Drag an end onto a target to glue it there (to the nearest site, else the side the pointer is
///   towards; ⌘ or Ctrl takes the point itself on a target with no sites), onto empty canvas to free
///   it; the connector follows the pointer. Dropping it on the object the other end is on leaves it as
///   it was. ⇧ turns a free end to 45° from the other end.
/// - Drag a bend to reshape the run or the curve: the path comes from the one the drag began with.
/// - The connector's end objects must be among the targets, or its route is unknown and it has no
///   handles.
/// - Keys: the handles are one Tab stop. Page Down and Up pick the next part; arrows move a picked
///   bend by the nudge step (⇧ by the far step) or move a picked end to the next (previous) target
///   in the targets' order, skipping the other end's object.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control)]
pub struct ConnectorHandles<'a> {
    #[common]
    common: Common,
    item: Bind<'a, ConnectorItem>,
    targets: &'a [ConnectionTarget],
    event: Option<ConnectorEvent>,
    /// The target under a dragged end, for its sites.
    hover: Option<(u64, Option<u32>)>,
}

impl<'a> ConnectorHandles<'a> {
    /// Handles named `label` over `item`, which may glue to `targets` (topmost first).
    pub fn new(label: impl Into<Label>, item: Bind<'a, ConnectorItem>, targets: &'a [ConnectionTarget]) -> Self {
        ConnectorHandles { common: Common::new(), item, targets, event: None, hover: None }.label(label)
    }

    /// The end under the pointer for a drag of the end that began at `fixed`: a free end snaps with the
    /// canvas, and ⇧ turns it from `fixed` to 45° steps.
    fn place(cx: &Cx<'_, Self>, space: &CanvasSpace, targets: &[ConnectionTarget], pos: Pos2, mods: Mods, fixed: Point) -> Placed {
        let placed = end_at(targets, space.view.to_doc(pos), &space.view, cx.style.reach, mods);
        let ConnectorEnd::Free(raw) = placed.end else { return placed };
        let mut at = match canvas_snap(&cx.env) {
            Some(snap) => Point::new(snap.apply(Axis::Y, raw.x, mods), snap.apply(Axis::X, raw.y, mods)),
            None => raw,
        };
        if mods.shift {
            at = turn_to_45(fixed, at);
        }
        Placed { end: ConnectorEnd::Free(at), route: RouteEnd::Point(at), ..placed }
    }

    /// `item` with the end `kind` set to `end`.
    fn with_end(item: &ConnectorItem, kind: ConnectorEndKind, end: ConnectorEnd) -> ConnectorItem {
        match kind {
            ConnectorEndKind::Start => ConnectorItem { from: end, ..item.clone() },
            ConnectorEndKind::End => ConnectorItem { to: end, ..item.clone() },
        }
    }
}

fn kind_of(index: u64) -> ConnectorEndKind {
    if index == 0 { ConnectorEndKind::Start } else { ConnectorEndKind::End }
}

impl Component for ConnectorHandles<'_> {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::CANVAS.with_moves().with_chrome_key(false);
    type Style = ConnectorHandlesStyle;
    type Output = ConnectorHandlesOutput;

    fn style(cx: &StyleCx<'_>, _: ConnectorHandlesStyleKey) -> ConnectorHandlesStyle {
        ConnectorHandlesStyle { metrics: Metrics::new(), look: ConnectorLook::of(cx), reach: cx.tokens.size.hit.target }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn bind(&mut self) -> Option<BindMut<'_, ConnectorItem>> {
        Some(self.item.reborrow())
    }

    /// The ends and the bends: drag parts at handle priority, the ends above the bends, one Tab stop.
    fn parts(&mut self, _rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let (Some(space), Some(item)) = (canvas_space(&cx.env), cx.value()) else { return };
        let Some((_, _, points)) = geometry(self.targets, item) else { return };
        let reach = cx.style.reach;
        let ends = [points.first(), points.last()];
        for (at, end) in ends.into_iter().enumerate() {
            let Some(&end) = end else { continue };
            cx.part(END.at(at as u64), HitShape::circle(space.view.to_screen(end), reach))
                .sense(Sense::DRAG | Sense::TAKE_FOCUS)
                .priority(HitPriority::new(HitLevel::Handle, 1))
                .cursor(CursorIcon::Move)
                .focusable();
        }
        for (portion, bend) in visible_bends(&points, item.routing, &space.view) {
            cx.part(BEND.at(portion as u64), HitShape::circle(space.view.to_screen(bend), reach))
                .sense(Sense::DRAG | Sense::TAKE_FOCUS)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .cursor(CursorIcon::Move)
                .focusable();
        }
        cx.roving(Roving::new(NavShape::Flat));
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let (Some(space), Some(item)) = (canvas_space(&cx.env), cx.value().cloned()) else { return };
        let Some((from, to, points)) = geometry(self.targets, &item) else { return };
        self.hover = None;
        for at in 0..2_u64 {
            let Some(result) = cx.part_if_declared(END.at(at)) else { continue };
            let press = result.press;
            let kind = kind_of(at);
            let (fixed, other) = match kind {
                ConnectorEndKind::Start => (points.last(), &item.to),
                ConnectorEndKind::End => (points.first(), &item.from),
            };
            let Some(&fixed) = fixed else { continue };
            match press.phase {
                PressPhase::DragStart { .. } => {
                    cx.begin();
                    cx.memory::<Start>().0 = Some(item.clone());
                }
                PressPhase::Drag { pos, .. } => {
                    let base = cx.memory::<Start>().0.clone().unwrap_or_else(|| item.clone());
                    let placed = Self::place(cx, &space, self.targets, pos, press.mods, fixed);
                    let onto_other = glued(&placed.end).is_some() && glued(&placed.end) == glued(other);
                    let end = if onto_other {
                        match kind {
                            ConnectorEndKind::Start => base.from,
                            ConnectorEndKind::End => base.to,
                        }
                    } else {
                        placed.end
                    };
                    self.hover = placed.target.filter(|&t| Some(t) != glued(other)).map(|t| (t, placed.active));
                    self.event = Some(ConnectorEvent::Reattach { end: kind, to: end });
                    cx.memory::<Last>().0 = self.event.clone();
                    cx.propose(Self::with_end(&base, kind, end));
                }
                PressPhase::Release { .. } => {
                    self.event = cx.memory::<Last>().0.take();
                    cx.commit();
                }
                PressPhase::Cancel { .. } => cx.cancel(CancelReason::Escape),
                _ => {}
            }
        }
        for (portion, _) in visible_bends(&points, item.routing, &space.view) {
            let Some(result) = cx.part_if_declared(BEND.at(portion as u64)) else { continue };
            match result.press.phase {
                PressPhase::DragStart { .. } => {
                    cx.begin();
                    cx.memory::<Start>().0 = Some(item.clone());
                }
                PressPhase::Drag { pos, .. } => {
                    let base = cx.memory::<Start>().0.clone().unwrap_or_else(|| item.clone());
                    let doc = space.view.to_doc(pos);
                    match reshaped(from, to, item.routing, base.path.as_ref(), portion, doc) {
                        Some(path) => {
                            self.event = Some(ConnectorEvent::Reshape(path.clone()));
                            cx.memory::<Last>().0 = self.event.clone();
                            cx.propose(ConnectorItem { path: Some(path), ..base });
                        }
                        // A reshape the geometry refuses makes nothing.
                        None => cx.cancel(CancelReason::Refused),
                    }
                }
                PressPhase::Release { .. } => {
                    self.event = cx.memory::<Last>().0.take();
                    cx.commit();
                }
                _ => {}
            }
        }
        self.run_keys(cx, &space, &item, from, to, &points);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (Some(space), Some(item)) = (canvas_space(&cx.env), cx.value()) else { return };
        let Some((_, _, points)) = geometry(self.targets, item) else { return };
        let draw = ui.draw();
        let look = &cx.style.look;
        if let Some((id, active)) = self.hover
            && let Some(target) = self.targets.iter().find(|t| t.id == id)
        {
            paint_sites(&draw, look, &space.view, target, active);
        }
        paint_route(&draw, look, &points, item.routing, &space.view);
        paint_handles(&draw, look, &points, item.routing, &space.view);
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let Some(item) = cx.value() else { return };
        let words = cx.words::<ConnectorWords>();
        let name = |end: &ConnectorEnd, attached: &craft_i18n::WordTemplate, free: &str| match end {
            ConnectorEnd::Glued { target, .. } => {
                let label = self.targets.iter().find(|t| t.id == *target).and_then(|t| t.name.clone()).unwrap_or_else(|| target.to_string());
                attached.fill(&[("target", label.as_str())])
            }
            ConnectorEnd::Free(_) => free.to_owned(),
        };
        let space = canvas_space(&cx.env);
        let bends = space
            .zip(geometry(self.targets, item))
            .map(|(space, (_, _, points))| visible_bends(&points, item.routing, &space.view))
            .unwrap_or_default();
        access.node(END.at(0), Role::BUTTON).name = Name::Text(name(&item.from, &words.start_attached, words.start_free.as_str()));
        access.node(END.at(1), Role::BUTTON).name = Name::Text(name(&item.to, &words.end_attached, words.end_free.as_str()));
        let count = bends.len().to_string();
        for (n, (portion, _)) in bends.into_iter().enumerate() {
            let index = (n + 1).to_string();
            access.node(BEND.at(portion as u64), Role::BUTTON).name =
                Name::Text(words.bend.fill(&[("index", index.as_str()), ("count", count.as_str())]));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> ConnectorHandlesOutput {
        let event = self.event;
        ConnectorHandlesOutput { edit: cx.edit_response(), event }
    }
}

impl ConnectorHandles<'_> {
    /// The keys: the pick follows the focus; arrows move the picked bend or the picked end.
    fn run_keys(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, item: &ConnectorItem, from: RouteEnd, to: RouteEnd, points: &[Point]) {
        let Some(part) = cx.active_part().filter(|part| part.key == END || part.key == BEND) else { return };
        let Some(index) = part.index else { return };
        let Some((direction, mods)) = cx.intents().take_nudge() else { return };
        if part.key == END {
            let kind = kind_of(index);
            let forward = direction.x + direction.y > 0.0;
            if let Some(to) = stepped_end(self.targets, item, kind, forward) {
                self.event = Some(ConnectorEvent::Reattach { end: kind, to });
                cx.set(Self::with_end(item, kind, to));
            }
            return;
        }
        let portion = index as usize;
        let Some((_, at)) = visible_bends(points, item.routing, &space.view).into_iter().find(|&(p, _)| p == portion) else { return };
        let step = if mods.shift { NUDGE_FAR } else { NUDGE };
        let by = space.view.unmap_vec(direction);
        let by = if by.hypot() > 0.0 { by / by.hypot() } else { by };
        if let Some(path) = reshaped(from, to, item.routing, item.path.as_ref(), portion, at + by * step) {
            self.event = Some(ConnectorEvent::Reshape(path.clone()));
            cx.set(ConnectorItem { path: Some(path), ..item.clone() });
        }
    }
}
