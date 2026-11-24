//! Drags across OS windows (feature `native-windows`, where viewports are native).
//!
//! The window where a drag started owns it: the window system sends it the pointer's moves
//! and the release while the button is held, inside the window or not. Each frame the owner
//! writes the pointer, with its own desktop origin, under the area's `"drag-pointer"`. The
//! context's data is shared by every viewport. The window under the desktop point (floating
//! windows first, front first, then the main window) shows the drop preview in its own
//! frame, with the targets it drew. The release is resolved at the end of the main window's
//! frame, after every native window has drawn.

use std::{fmt::Debug, hash::Hash};

use egui::{Context, Key, Pos2, Rect, Ui, Vec2, vec2};

use super::floating::WindowHit;
use super::native::{BAR_HEIGHT, WindowDrag};
use super::preview::DragInput;
use super::{Renderer, clamp_float, dock::group_action};
use crate::docking::desktop::{Over, window_at};
use crate::docking::targets::DockDrop;
use crate::docking::{Action, Drag, DropTarget, Floating, GroupDrag, GroupGesture, Target, cancel_drag};

/// Key of [`DragPointer`] in the context's data, under the area's id.
pub(super) const POINTER: &str = "drag-pointer";
/// Key of an Escape that reached one of the application's windows other than the drag's
/// owner. Keys go to the window with keyboard focus, which need not be the one dragging.
const ESCAPE: &str = "drag-escape";
/// Key of whether a native window's bar drag highlighted a drop target on the last frame, under
/// the area's id: [`Cross::highlighted`] as [`Renderer::end_cross`] left it.
pub(super) const HIGHLIGHTED: &str = "drag-highlighted";

/// The pointer of a drag, as the window that owns the drag last saw it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::docking) struct DragPointer {
    /// [`Floating::id`] of the owning native window; `None` for the main window.
    pub owner: Option<u64>,
    /// Desktop position of the owner's (0, 0). `None` where the window system reports no
    /// positions (Wayland).
    pub origin: Option<Pos2>,
    /// In the owner's coordinates. Kept while egui reports no position.
    pub local: Option<Pos2>,
    pub down: bool,
    pub released: bool,
    pub escape: bool,
    /// Ctrl or Cmd held: the drop floats.
    pub command: bool,
    /// The window a bar drag moves. It follows the pointer and is never its own target.
    pub window: Option<u64>,
    /// That drag began on the window's only tab or group: it docks whatever
    /// [`crate::docking::DockStyle::title_bar_docks`] says.
    pub docks: bool,
}

impl DragPointer {
    fn desktop(&self) -> Option<Pos2> {
        Some(self.origin? + self.local?.to_vec2())
    }
}

/// A native window's drop targets from its frame, in its own coordinates.
pub(in crate::docking) struct Surface<P> {
    pub window: u64,
    /// Desktop position of the window's (0, 0), where known.
    pub origin: Option<Pos2>,
    /// The window's content, in its coordinates.
    pub bounds: Rect,
    pub targets: Vec<DropTarget<P>>,
    pub dock_targets: Vec<DockDrop<P>>,
    pub windows: Vec<WindowHit<P>>,
}

/// Where a drag's pointer is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Dest {
    Main(Pos2),
    Window(u64, Pos2),
    /// Over no window of the application: the desktop point, where known.
    Desktop(Option<Pos2>),
}

/// What one frame of a dock area knows of its OS windows.
pub(in crate::docking) struct Cross<P> {
    /// Viewports are native: drags cross windows.
    pub on: bool,
    /// The application's windows lost focus this pass: one had it on the last pass, none now.
    pub focus_lost: bool,
    /// The main window's content on the desktop, where known.
    pub main: Option<Rect>,
    /// Shown native windows' desktop rects, front first.
    pub windows: Vec<(u64, Rect)>,
    /// How many of `windows` lie in front of the main window.
    pub front: usize,
    /// Targets of the native windows drawn this frame.
    pub surfaces: Vec<Surface<P>>,
    /// The drag's pointer, as its owner wrote it this frame.
    pub pointer: Option<DragPointer>,
    /// A bar drag released this frame.
    pub release: Option<(WindowDrag, DragPointer)>,
    /// Resolving a native window's bar drag over the main window: frame edge zones count.
    pub bar: bool,
    /// Where the native window drawing now was told to move its content this frame.
    pub moved: Option<Pos2>,
    /// Resolving a window move that docks whatever `title_bar_docks` says.
    pub docks: bool,
    /// A native window's bar drag highlighted a drop target this frame, in whichever window
    /// drew its preview.
    pub highlighted: bool,
    /// A native window's dragged widget is one of the area's controls: egui keeps a drag per
    /// viewport.
    pub held: bool,
    /// The panel whose body is under the pointer in the native window drawing now.
    pub pointed: Option<P>,
    /// The viewport of each native window drawn this frame.
    pub viewports: Vec<(u64, egui::ViewportId)>,
}

impl<P> Default for Cross<P> {
    fn default() -> Self {
        Self {
            on: false,
            focus_lost: false,
            main: None,
            windows: Vec::new(),
            front: 0,
            surfaces: Vec::new(),
            pointer: None,
            release: None,
            bar: false,
            moved: None,
            docks: false,
            highlighted: false,
            held: false,
            pointed: None,
            viewports: Vec::new(),
        }
    }
}

impl<P: Clone + Eq + Hash + Debug + Send + Sync + 'static> Renderer<'_, P> {
    /// Write the pointer of the drag this window owns. `origin` is the window's desktop
    /// origin for this frame's input; `window` the bar drag it is.
    pub(super) fn record_pointer(&mut self, ctx: &Context, owner: Option<u64>, origin: Option<Pos2>, window: Option<&WindowDrag>) -> DragPointer {
        let key = self.id.with(POINTER);
        let kept = ctx.data(|d| d.get_temp::<DragPointer>(key)).filter(|p| p.owner == owner).and_then(|p| p.local);
        let elsewhere = ctx.data_mut(|d| d.remove_temp::<bool>(self.id.with(ESCAPE))).unwrap_or(false);
        let pointer = ctx.input(|i| DragPointer {
            owner,
            origin,
            local: i.pointer.latest_pos().or(i.pointer.interact_pos()).filter(|p| p.is_finite()).or(kept),
            down: i.pointer.primary_down(),
            released: i.pointer.primary_released(),
            escape: elsewhere || i.key_pressed(Key::Escape),
            command: i.modifiers.command || i.modifiers.ctrl || i.modifiers.mac_cmd,
            window: window.map(|w| w.window),
            docks: window.is_some_and(|w| w.docks),
        });
        self.write_pointer(ctx, pointer);
        pointer
    }

    /// Write `pointer` as the drag's for this frame. An Escape from another window is no
    /// cancel of a drag that the window system carries out.
    pub(super) fn write_pointer(&mut self, ctx: &Context, pointer: DragPointer) {
        let id = self.id;
        ctx.data_mut(|d| {
            d.remove::<bool>(id.with(ESCAPE));
            d.insert_temp(id.with(POINTER), pointer);
        });
        self.cross.pointer = Some(pointer);
    }

    /// In a window that does not own the drag (`window`, `None` for the main window): an Escape
    /// pressed here cancels the drag, and is no shortcut.
    pub(super) fn escape_elsewhere(&self, ctx: &Context, window: Option<u64>) {
        let owner = ctx.data(|d| d.get_temp::<DragPointer>(self.id.with(POINTER))).filter(|p| p.down).map(|p| p.owner);
        if owner.is_some_and(|owner| owner != window) && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            ctx.data_mut(|d| d.insert_temp(self.id.with(ESCAPE), true));
        }
    }

    /// Where the drag's pointer is. Without desktop positions only the owner's own window is
    /// known, and a point outside it is on the desktop. `current` is a window drawing now.
    fn destination(&self, pointer: &DragPointer, current: Option<&Surface<P>>) -> Option<Dest> {
        if let (Some(point), Some(main)) = (pointer.desktop(), self.cross.main) {
            return Some(match window_at(point, &self.cross.windows, self.cross.front, Some(main), pointer.window) {
                Over::Main(local) => Dest::Main(local),
                Over::Window(id, local) => Dest::Window(id, local),
                Over::Desktop => Dest::Desktop(Some(point)),
            });
        }
        let local = pointer.local?;
        let Some(owner) = pointer.owner else { return Some(Dest::Main(local)) };
        let bounds = current.into_iter().chain(&self.cross.surfaces).find(|s| s.window == owner).map(|s| s.bounds);
        Some(if bounds.is_some_and(|b| b.contains(local)) { Dest::Window(owner, local) } else { Dest::Desktop(None) })
    }

    /// Run `f` with a native window's targets and bounds in place of the main window's.
    fn within<R>(&mut self, surface: &mut Surface<P>, f: impl FnOnce(&mut Self) -> R) -> R {
        std::mem::swap(&mut self.targets, &mut surface.targets);
        std::mem::swap(&mut self.dock_targets, &mut surface.dock_targets);
        std::mem::swap(&mut self.frame.windows, &mut surface.windows);
        let bounds = std::mem::replace(&mut self.float_bounds, surface.bounds);
        let blockers = std::mem::take(&mut self.blockers);
        // A native window has no docks.
        let groups = std::mem::take(&mut self.dock_groups);
        let result = f(self);
        self.dock_groups = groups;
        std::mem::swap(&mut self.targets, &mut surface.targets);
        std::mem::swap(&mut self.dock_targets, &mut surface.dock_targets);
        std::mem::swap(&mut self.frame.windows, &mut surface.windows);
        (self.float_bounds, self.blockers) = (bounds, blockers);
        result
    }

    /// [`Self::within`] the native window `window` drawn this frame.
    fn within_window<R>(&mut self, window: u64, f: impl FnOnce(&mut Self) -> R) -> Option<R> {
        let index = self.cross.surfaces.iter().position(|s| s.window == window)?;
        let mut surface = self.cross.surfaces.swap_remove(index);
        let result = self.within(&mut surface, f);
        self.cross.surfaces.push(surface);
        Some(result)
    }

    /// The offset from the owner's coordinates to those of a window at `to`.
    fn shift(&self, pointer: &DragPointer, to: Option<Pos2>) -> Vec2 {
        match (pointer.origin, to) {
            (Some(from), Some(to)) => from - to,
            _ => Vec2::ZERO,
        }
    }

    fn origin_of(&self, window: u64) -> Option<Pos2> {
        self.cross.surfaces.iter().find(|s| s.window == window).and_then(|s| s.origin)
    }

    /// A tab drag at the end of the main window's frame. Over the main window it returns the
    /// drag in the main window's coordinates and the input to finish it with; elsewhere it
    /// docks in another window or floats a new one on release.
    pub(super) fn cross_tab(&mut self, ctx: &Context, drag: &Drag<P>, input: DragInput) -> Option<(Drag<P>, DragInput)> {
        let Some(pointer) = self.cross.pointer.filter(|p| self.cross.on && p.window.is_none() && p.local.is_some()) else {
            return Some((drag.clone(), input));
        };
        let input = DragInput { escape: input.escape || pointer.escape, ..self.input_of(&pointer, pointer.local) };
        if input.escape || !input.focused || (!input.down && !input.released) {
            return Some((drag.clone(), input));
        }
        let float_only = pointer.command || drag.undock;
        let floats_out = float_only && pointer.released && pointer.desktop().is_some();
        match self.destination(&pointer, None)? {
            Dest::Main(local) if !floats_out => {
                let shift = self.shift(&pointer, self.cross.main.map(|m| m.min));
                let drag = Drag { origin: drag.origin + shift, source: drag.source.translate(shift), ..drag.clone() };
                return Some((drag, DragInput { pointer: Some(local), ..input }));
            }
            _ if !pointer.released => {}
            Dest::Window(window, local) if !float_only => {
                let shift = self.shift(&pointer, self.origin_of(window));
                let drag = Drag { origin: drag.origin + shift, source: drag.source.translate(shift), ..drag.clone() };
                let action = self.within_window(window, |r| {
                    let proposal = r.tab_proposal(&drag.panel, local).filter(|p| !p.noop && local.distance(drag.origin) >= 2.0)?;
                    Some(Action::Move { panel: drag.panel.clone(), anchor: proposal.anchor, placement: proposal.placement })
                });
                self.push_checked(action.flatten(), &drag.panel);
            }
            _ if !self.permissions(&drag.panel).floatable => {}
            _ => {
                let size = self.float_size(&drag.panel, drag.source.size());
                let grab = drag.grab.min(vec2(size.x, self.style.tab_height)) + vec2(0.0, BAR_HEIGHT);
                let rect = self.desktop_float(&pointer, size, grab);
                let action = Action::Float { panel: drag.panel.clone(), rect: [rect.left(), rect.top(), rect.width(), rect.height()] };
                self.push_checked(Some(action), &drag.panel);
            }
        }
        if pointer.released {
            cancel_drag::<P>(ctx, self.id);
        }
        None
    }

    /// A group or column drag at the end of the main window's frame, like [`Self::cross_tab`].
    pub(super) fn cross_group(&mut self, ui: &Ui, drag: &GroupDrag<P>, input: DragInput) -> Option<(GroupDrag<P>, DragInput)> {
        let Some(pointer) = self.cross.pointer.filter(|p| self.cross.on && p.window.is_none() && p.local.is_some()) else {
            return Some((drag.clone(), input));
        };
        if drag.gesture == GroupGesture::Window {
            return Some((drag.clone(), input));
        }
        let input = self.input_of(&pointer, pointer.local);
        if input.escape || !input.focused || (!input.down && !input.released) {
            return Some((drag.clone(), input));
        }
        let floats_out = pointer.command && pointer.released && pointer.desktop().is_some();
        let ctx = ui.ctx();
        match self.destination(&pointer, None)? {
            Dest::Main(local) if !floats_out => {
                let shift = self.shift(&pointer, self.cross.main.map(|m| m.min));
                let drag = GroupDrag { origin: drag.origin + shift, source: drag.source.translate(shift), ..drag.clone() };
                return Some((drag, DragInput { pointer: Some(local), ..input }));
            }
            _ if !pointer.released => {}
            Dest::Window(window, local) if !pointer.command => {
                let shift = self.shift(&pointer, self.origin_of(window));
                let drag = GroupDrag { origin: drag.origin + shift, source: drag.source.translate(shift), ..drag.clone() };
                let action = self.within_window(window, |r| {
                    let proposal = r.group_proposal(&drag, local).filter(|p| !p.noop && local.distance(drag.origin) >= 2.0)?;
                    Some(group_action(&drag, Target::At { anchor: proposal.anchor, placement: proposal.placement }))
                });
                self.push_checked(action.flatten(), &drag.panel);
            }
            _ => {
                let size = self.group_float_size(&drag.members, drag.source.size());
                let grab = (drag.origin - drag.source.min).clamp(Vec2::ZERO, size) + vec2(0.0, BAR_HEIGHT);
                let rect = self.desktop_float(&pointer, size, grab);
                let action = group_action(drag, Target::Float { rect: Some([rect.left(), rect.top(), rect.width(), rect.height()]) });
                self.push_checked(Some(action), &drag.panel);
            }
        }
        if pointer.released {
            cancel_drag::<P>(ctx, self.id);
        }
        None
    }

    /// A bar drag of a native window, at the end of the main window's frame: the preview
    /// over the main window, and the release.
    pub(in crate::docking) fn finish_window_drag(&mut self, ui: &Ui) {
        if let Some((drag, pointer)) = self.cross.release.take() {
            self.drop_native_window(drag.window, &pointer);
            return;
        }
        let Some(pointer) = self.cross.pointer.filter(|p| p.window.is_some() && !p.released) else { return };
        let Some(drag) = pointer.window.and_then(|w| self.window_group_drag(w)) else { return };
        if let Some(Dest::Main(local)) = self.destination(&pointer, None) {
            (self.cross.bar, self.cross.docks) = (true, pointer.docks);
            self.cross.highlighted |= self.paint_group_drag(ui, &drag, local, pointer.command);
            (self.cross.bar, self.cross.docks) = (false, false);
        }
    }

    /// Release of a bar drag: over a target in another window the whole window docks or
    /// joins there. Over the desktop, or with Ctrl/Cmd held, it stays where it was moved.
    fn drop_native_window(&mut self, window: u64, pointer: &DragPointer) {
        let Some(drag) = self.window_group_drag(window).filter(|_| !pointer.command && (pointer.docks || self.style.title_bar_docks)) else { return };
        match self.destination(pointer, None) {
            Some(Dest::Main(local)) => {
                (self.cross.bar, self.cross.docks) = (true, pointer.docks);
                self.drop_window(&drag, local, false);
                (self.cross.bar, self.cross.docks) = (false, false);
            }
            Some(Dest::Window(target, local)) => {
                let action = self.within_window(target, |r| {
                    let proposal = r.group_proposal(&drag, local).filter(|p| !p.noop)?;
                    Some(group_action(&drag, Target::At { anchor: proposal.anchor, placement: proposal.placement }))
                });
                if let Some(action) = action.flatten() {
                    self.output.actions.push(action);
                }
            }
            _ => {}
        }
    }

    /// A native window's bar drag as a whole-window group drag.
    fn window_group_drag(&self, window: u64) -> Option<GroupDrag<P>> {
        let window: &Floating<P> = self.layout.floating.iter().find(|w| w.id == window)?;
        let members: Vec<P> = window.panels().cloned().collect();
        let panel = members.first()?.clone();
        Some(GroupDrag { panel, original: window.rect, gesture: GroupGesture::Window, members, origin: Pos2::ZERO, source: Rect::NOTHING })
    }

    /// At the end of a native window's frame: the drop preview, and its undock hint, when the
    /// drag's pointer is over this window.
    pub(super) fn paint_cross(&mut self, ui: &Ui, surface: &mut Surface<P>) {
        let ctx = ui.ctx();
        let Some(pointer) = ctx.data(|d| d.get_temp::<DragPointer>(self.id.with(POINTER))).filter(|p| !p.released && p.down) else { return };
        if let Some(Dest::Window(window, local)) = self.destination(&pointer, Some(&*surface))
            && window == surface.window
        {
            let shift = self.shift(&pointer, surface.origin);
            self.within(surface, |r| r.paint_drop(ui, &pointer, local, shift));
        }
    }

    /// The preview of the drag at `local`, in the window drawing now. `shift` takes the
    /// owner's coordinates to this window's.
    fn paint_drop(&mut self, ui: &Ui, pointer: &DragPointer, local: Pos2, shift: Vec2) {
        let ctx = ui.ctx();
        if let Some(window) = pointer.window {
            if let Some(drag) = self.window_group_drag(window) {
                self.cross.docks = pointer.docks;
                self.cross.highlighted |= self.paint_group_drag(ui, &drag, local, pointer.command);
                self.cross.docks = false;
            }
            return;
        }
        if let Some(drag) = ctx.data(|d| d.get_temp::<Drag<P>>(self.id.with("drag"))) {
            if pointer.command || drag.undock {
                return;
            }
            let drag = Drag { origin: drag.origin + shift, source: drag.source.translate(shift), ..drag };
            let proposal = self.tab_proposal(&drag.panel, local);
            let painter = self.preview_painter(ctx);
            self.paint_tab_proposal(ctx, &painter, &drag, proposal.as_ref(), local);
        } else if let Some(drag) = ctx.data(|d| d.get_temp::<GroupDrag<P>>(self.id.with("group-drag"))).filter(|d| d.gesture != GroupGesture::Window)
        {
            let drag = GroupDrag { origin: drag.origin + shift, source: drag.source.translate(shift), ..drag };
            self.paint_group_drag(ui, &drag, local, pointer.command);
        }
    }

    /// End of the main window's frame: forget the pointer of a finished drag, and keep whether
    /// a bar drag highlighted a drop target. The dragged window sets its opacity from that on
    /// the next frame: it draws before the main window paints its preview.
    pub(in crate::docking) fn end_cross(&mut self, ctx: &Context) {
        let id = self.id;
        ctx.data_mut(|d| d.insert_temp(id.with(HIGHLIGHTED), self.cross.highlighted));
        let active = ctx.data(|d| {
            d.get_temp::<Drag<P>>(id.with("drag")).is_some()
                || d.get_temp::<GroupDrag<P>>(id.with("group-drag")).is_some()
                || d.get_temp::<WindowDrag>(id.with("window-drag")).is_some()
        });
        if !active {
            ctx.data_mut(|d| d.remove::<DragPointer>(id.with(POINTER)));
        }
    }

    /// The drag's input as its owner saw it, at `pointer` in the coordinates of the window
    /// finishing it. Leaving a window does not end the drag.
    fn input_of(&self, pointer: &DragPointer, at: Option<Pos2>) -> DragInput {
        DragInput {
            pointer: at,
            down: pointer.down,
            released: pointer.released,
            focused: !self.cross.focus_lost,
            gone: false,
            escape: pointer.escape,
            command: pointer.command,
            touch_ended: false,
        }
    }

    /// The in-window rect of a window floated out at the desktop pointer with its top-left
    /// `grab` from it. The new window opens at the main window's origin plus this rect. Where
    /// positions are unknown the window system places it.
    fn desktop_float(&self, pointer: &DragPointer, size: Vec2, grab: Vec2) -> Rect {
        match (pointer.desktop(), self.cross.main) {
            (Some(point), Some(main)) => Rect::from_min_size(point - grab - main.min.to_vec2(), size),
            _ => clamp_float(Rect::from_min_size(self.float_bounds.min, size), self.float_bounds),
        }
    }

    /// Push `action` when the model accepts it, focusing `panel`.
    fn push_checked(&mut self, action: Option<Action<P>>, panel: &P) {
        let Some(action) = action else { return };
        if self.layout.clone().apply_with_permissions(action.clone(), |p| self.permissions(p)).is_ok() {
            self.output.actions.push(action);
            self.output.focused = Some(panel.clone());
        }
    }
}
