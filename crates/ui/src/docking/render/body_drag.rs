//! App drags in panel bodies across OS windows (feature `native-windows`).
//!
//! egui gives each window only its own input, and the window where the button went down gets
//! the pointer until the release. An app's drag that began in a panel body (a media item onto
//! a timeline in another window) would never show its pointer or release to the window it is
//! over. So while one that carries something lasts (an egui drag-and-drop payload, or
//! [`crate::docking::route_app_drag`]), the owner's input hook ([`super::native`]'s plugin)
//! sends each pointer move over another of the app's windows to that window, in its
//! coordinates, as `PointerMoved`, `PointerGone` when the pointer leaves it, and the release as
//! a primary `PointerButton` there. The owner sees the pointer gone while it is over another
//! window, so its own panels under that window see no hover and no drop.
//!
//! A window drawn after the owner in the same pass gets the events in that pass: the main
//! window owning the drag reaches every native window at once. The main window, and a native
//! window drawn before the owner, get them on the next pass, which the hook asks for. The
//! owner's own release then waits a pass too, so its payload outlives the target's read.

use std::{collections::HashMap, fmt::Debug, hash::Hash};

use egui::{Context, Event, Id, Key, PointerButton, Pos2, RawInput, Rect, ViewportId};

use super::Renderer;
use super::native::{WindowDrag, body_input, content_of};
use crate::docking::desktop::{Over, window_at};
use crate::docking::{Drag, GroupDrag};

/// The application's windows on the desktop, as the main window last mapped them.
#[derive(Clone, Debug, Default)]
pub(super) struct Desktop {
    /// The main window's content, where known.
    pub main: Option<Rect>,
    /// Shown native windows' content, front first, and how many lie in front of the main window.
    pub windows: Vec<(u64, Rect)>,
    pub front: usize,
    /// The viewport of each native window.
    pub viewports: Vec<(u64, ViewportId)>,
}

impl Desktop {
    /// The window under the desktop point `point`, and the point in its coordinates.
    fn at(&self, point: Pos2) -> Option<(ViewportId, Pos2)> {
        match window_at(point, &self.windows, self.front, Some(self.main?), None) {
            Over::Main(local) => Some((ViewportId::ROOT, local)),
            Over::Window(window, local) => self.viewports.iter().find(|(w, _)| *w == window).map(|(_, viewport)| (*viewport, local)),
            Over::Desktop => None,
        }
    }
}

/// A dock area's app drag in a panel body, in the input plugin.
#[derive(Debug)]
pub(super) struct BodyRoute {
    area: Id,
    /// The window the drag began in, and the widget egui drags there.
    owner: ViewportId,
    /// [`crate::docking::Floating::id`] of the owner; `None` for the main window.
    window: Option<u64>,
    id: Id,
    desktop: Desktop,
    /// The window other than the owner that the pointer is over.
    target: Option<ViewportId>,
    /// Nothing more is routed until the drag ends: released, or ended by Escape, a focus loss,
    /// input blocking or [`crate::docking::cancel_drag`].
    ended: bool,
}

/// Events waiting for each viewport's next input, with the main window's pass that queued them
/// and the dock area whose route queued each.
pub(super) type Queued = HashMap<ViewportId, (u64, Vec<(Id, Event)>)>;

/// Each owner's held release and the pointer events after it, with the main window's pass that
/// held them: delivered in a later pass only, never taken back.
pub(super) type Held = HashMap<ViewportId, (u64, Vec<Event>)>;

fn queue(queued: &mut Queued, pass: u64, area: Id, viewport: ViewportId, event: Event) {
    let (at, events) = queued.entry(viewport).or_default();
    *at = pass;
    events.push((area, event));
}

impl BodyRoute {
    /// The owner's input before egui reads it: each pointer event over another of the app's
    /// windows goes to that window, and the owner sees the pointer gone. Returns whether
    /// anything was queued. Where the owner reports no desktop position nothing is routed.
    pub(super) fn route(&mut self, input: &mut RawInput, queued: &mut Queued, held: &mut Held, pass: u64) -> bool {
        if input.viewport_id != self.owner || self.ended {
            return false;
        }
        let Some((origin, _)) = input.viewports.get(&input.viewport_id).and_then(content_of) else { return false };
        let (mut routed, mut holding) = (false, false);
        let mut events = Vec::with_capacity(input.events.len());
        for event in std::mem::take(&mut input.events) {
            // Pointer events after a held release wait behind it, in order.
            if holding && matches!(event, Event::PointerMoved(_) | Event::PointerButton { .. } | Event::PointerGone) {
                held.entry(self.owner).or_default().1.push(event);
                continue;
            }
            let (pos, release) = match &event {
                Event::PointerMoved(pos) => (*pos, None),
                Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers } => (*pos, Some(*modifiers)),
                _ => {
                    events.push(event);
                    continue;
                }
            };
            if self.ended {
                events.push(event);
                continue;
            }
            let over = pos.is_finite().then(|| self.desktop.at(origin + pos.to_vec2())).flatten().filter(|(v, _)| *v != self.owner);
            let target = over.map(|(viewport, _)| viewport);
            if let Some(left) = self.target.filter(|left| Some(*left) != target) {
                queue(queued, pass, self.area, left, Event::PointerGone);
                routed = true;
            }
            self.target = target;
            if let Some((viewport, local)) = over {
                queue(queued, pass, self.area, viewport, Event::PointerMoved(local));
                if let Some(modifiers) = release {
                    let release = Event::PointerButton { pos: local, button: PointerButton::Primary, pressed: false, modifiers };
                    queue(queued, pass, self.area, viewport, release);
                    // The owner's own release waits for its input in a later pass of the main
                    // window (not a discarded pass's rerun), so its drag, and an egui
                    // drag-and-drop payload, last until a window drawn before it has read the
                    // release. A stop never takes it back, or the button would stay down.
                    let (at, held_events) = held.entry(self.owner).or_default();
                    *at = pass;
                    held_events.extend([event, Event::PointerGone]);
                    holding = true;
                    routed = true;
                    (self.ended, self.target) = (true, None);
                    events.push(Event::PointerGone);
                    continue;
                }
                routed = true;
                events.push(Event::PointerGone);
            } else {
                events.push(event);
            }
            if release.is_some() {
                (self.ended, self.target) = (true, None);
            }
        }
        input.events = events;
        routed
    }
}

/// End `area`'s routing: the window the pointer was over sees it go. `remove`: the drag is
/// over, so another may start. Otherwise the routing was stopped (Escape, a focus loss, input
/// blocking, [`crate::docking::cancel_drag`]), and events it queued that no window has read
/// yet are taken back, so a release queued before the stop reaches no window.
fn end_route(ctx: &Context, area: Id, remove: bool) {
    let body = body_input(ctx);
    let queued = {
        let mut input = body.lock();
        let input = &mut *input;
        if let Some(route) = input.routes.get_mut(&area) {
            let mut gone: Vec<ViewportId> = route.target.take().into_iter().collect();
            route.ended = true;
            if remove {
                input.routes.remove(&area);
            } else {
                for (viewport, (_, events)) in &mut input.queued {
                    let before = events.len();
                    events.retain(|(from, _)| *from != area);
                    if events.len() != before && !gone.contains(viewport) {
                        gone.push(*viewport);
                    }
                }
            }
            for viewport in &gone {
                queue(&mut input.queued, input.pass, area, *viewport, Event::PointerGone);
            }
            Some(!gone.is_empty())
        } else {
            None
        }
    };
    if queued == Some(true) {
        ctx.request_repaint_of(ViewportId::ROOT);
    }
}

/// [`crate::docking::cancel_drag`]: an app drag in a panel body goes on, but no longer reaches
/// other windows.
pub(in crate::docking) fn cancel_body_route(ctx: &Context, area: Id) {
    end_route(ctx, area, false);
}

impl<P: Clone + Eq + Hash + Debug + Send + Sync + 'static> Renderer<'_, P> {
    /// In the main window, once the desktop is mapped and before native windows draw: give the
    /// routing the map, and end it where it cannot go on. `native`: viewports are native.
    pub(super) fn map_body_route(&mut self, ctx: &Context, native: bool) {
        let desktop = self.desktop();
        let owner = {
            let body = body_input(ctx);
            let mut input = body.lock();
            input.routes.get_mut(&self.id).map(|route| {
                route.desktop = desktop;
                route.window
            })
        };
        // Windows are no longer native, or the owner closed.
        let natives = &self.cross.viewports;
        if owner.is_some_and(|window| !native || window.is_some_and(|w| !natives.iter().any(|(id, _)| *id == w))) {
            end_route(ctx, self.id, true);
        }
        if native {
            self.route_body_drag(ctx, None);
        }
    }

    fn desktop(&self) -> Desktop {
        Desktop { main: self.cross.main, windows: self.cross.windows.clone(), front: self.cross.front, viewports: self.cross.viewports.clone() }
    }

    /// After the window drawing now (`window`; `None` for the main window) drew its panels'
    /// bodies: start routing an app drag that began in one of them, and end it when the drag
    /// is over. An Escape in any of the app's windows, a focus loss and input blocking end it.
    pub(super) fn route_body_drag(&mut self, ctx: &Context, window: Option<u64>) {
        let (area, viewport, dragged) = (self.id, ctx.viewport_id(), ctx.dragged_id());
        let route = {
            let body = body_input(ctx);
            let input = body.lock();
            input.routes.get(&area).map(|r| (r.owner, r.id))
        };
        let stops = self.input_blocked || self.cross.focus_lost || ctx.input(|i| i.key_pressed(Key::Escape));
        match route {
            Some((owner, id)) if owner == viewport && dragged != Some(id) => end_route(ctx, area, true),
            Some(_) if stops => end_route(ctx, area, false),
            Some(_) => {}
            None => {
                let Some(id) = dragged.filter(|id| !stops && self.starts_in_body(ctx, *id)) else { return };
                let route = BodyRoute { area, owner: viewport, window, id, desktop: self.desktop(), target: None, ended: false };
                let _ = body_input(ctx).lock().routes.insert(area, route);
            }
        }
    }

    /// Whether egui's drag of `id` in the window drawing now is the app's and carries something:
    /// pressed with the primary button in a panel body, none of the area's own controls or
    /// drags, and with an egui drag-and-drop payload or [`crate::docking::route_app_drag`]
    /// called this frame or the last. Other drags (sliders, scrollbars, text selection, canvas
    /// pans) keep the owner's pointer.
    fn starts_in_body(&self, ctx: &Context, id: Id) -> bool {
        let payload = ctx.plugin_opt::<egui::DragAndDrop>().is_some() && egui::DragAndDrop::has_any_payload(ctx);
        let main = ctx.cumulative_pass_nr_for(ViewportId::ROOT);
        let asked = ctx.data(|d| d.get_temp::<u64>(app_drag_key(self.id))).is_some_and(|at| at + 1 >= main);
        if !payload && !asked {
            return false;
        }
        let (area, viewport) = (self.id, ctx.viewport_id());
        let (down, press) = ctx.input(|i| (i.pointer.primary_down(), i.pointer.press_origin()));
        let (pressed, owned, dock) = ctx.data(|d| {
            let pressed = d.get_temp::<Pos2>(body_press_key(area, viewport));
            let owned = d.get_temp::<Vec<Id>>(area.with("owned-interactions")).is_some_and(|ids| ids.contains(&id))
                || d.get_temp::<Id>(area.with("pointer-resize")) == Some(id);
            let dock = d.get_temp::<Drag<P>>(area.with("drag")).is_some()
                || d.get_temp::<GroupDrag<P>>(area.with("group-drag")).is_some()
                || d.get_temp::<WindowDrag>(area.with("window-drag")).is_some();
            (pressed, owned, dock)
        });
        down && press.is_some() && pressed == press && !owned && !dock && !self.owned_interactions.contains(&id)
    }
}

/// The main window's pass in which the app last called [`crate::docking::route_app_drag`].
fn app_drag_key(area: Id) -> Id {
    area.with("route-app-drag")
}

/// [`crate::docking::route_app_drag`].
pub(in crate::docking) fn mark_app_drag(ctx: &Context, area: Id) {
    let main = ctx.cumulative_pass_nr_for(ViewportId::ROOT);
    ctx.data_mut(|d| d.insert_temp(app_drag_key(area), main));
}

/// The events held for `viewport`'s input in a later pass.
#[cfg(test)]
pub(in crate::docking) fn held_events(ctx: &Context, viewport: ViewportId) -> Vec<Event> {
    let body = body_input(ctx);
    let events = body.lock().held.get(&viewport).map(|(_, events)| events.clone());
    events.unwrap_or_default()
}

/// Where the last press in one of the area's panel bodies in `viewport` began.
pub(super) fn body_press_key(area: Id, viewport: ViewportId) -> Id {
    area.with(("body-press", viewport))
}
