//! Move and resize of a layer or OS window: the move band, the grips, the keyboard mode, limits and OS window facts.

use super::BodyHooks;
use super::placement::{Placement, PlacementSlot, overlay_of};
use super::regions::Regions;
use crate::contract::overlay::{PlacementChange, PlacementKind};
use crate::contract::{Container, ContainerProps, Cx, Mount, PartsCx};
use crate::runtime::commands::{self, CommandSet as _, WindowCommand};
use crate::runtime::egui::pass;
use crate::runtime::egui::{input, viewports};
use crate::runtime::memory::sub;
use crate::runtime::windows::{self, ResizeEdge, WindowAction, WindowHit};
use crate::runtime::{keys, pointer};
use crate::style::ComponentStyle as _;
use craft_behavior::{CursorIcon, GesturePhase, HitLevel, HitPriority, PartKey, PressPhase};
use craft_host::target::{Caps, WindowChrome, WindowState};
use craft_keymap::{Chord, Key, Mods};
use egui::{Context, Id, Pos2, Rect, Vec2, ViewportId};

pub(crate) const MOVE_BAND: PartKey = PartKey::new("move_band");

const GRIP: PartKey = PartKey::new("resize_grip");

/// Which move and resize parts of the body step stand for the OS window of `viewport` (16.3, 21.2): the one rule
/// every move band and grip of an OS window reads. Both need a frame the app draws (the chrome is not `Native`). The
/// band stands on a maximized window too, so its double click restores the window and a drag is the platform's (it
/// restores and moves); the grips stand only on a window that is neither maximized nor full screen. The parts stand
/// whether or not a hit-test sink is installed: the sink reads them as the window's exclusive parts.
#[derive(Clone, Copy)]
struct WindowParts {
    band: bool,
    grips: bool,
}

impl WindowParts {
    /// The parts of a layer or a full-window mount, which move the container and never the OS window.
    const LAYER: WindowParts = WindowParts { band: true, grips: true };

    fn of(ctx: &Context, viewport: ViewportId) -> WindowParts {
        let window = windows::shell_of(ctx, viewport).window;
        let framed_by_app = window.chrome != WindowChrome::Native;
        let sized_by_os = matches!(window.state, WindowState::Maximized | WindowState::Fullscreen);
        WindowParts { band: framed_by_app && window.state != WindowState::Fullscreen, grips: framed_by_app && !sized_by_os }
    }
}

/// Whether the platform answers the hit test of `viewport`'s undecorated window itself, as it does for a window whose
/// hit-test sink is installed (`WindowHost::install_hit_sink`): it takes the move, edge and caption hits from the
/// window's exclusive parts, so a press on the move band or a grip never reaches the app as a move.
fn answered_by_platform(ctx: &Context, viewport: ViewportId) -> bool {
    let shell = windows::shell_of(ctx, viewport);
    shell.caps.contains(Caps::OS_RESIZES_UNDECORATED) && shell.window.chrome == WindowChrome::Integrated && windows::hit_sink_installed(ctx, viewport)
}

/// The viewport of the OS window a container's move and resize parts act on: a viewport mount's own window, the
/// window a `Window`, `Pane` or `Inline` container is shown in. A layer (also a viewport mount's fallback layer)
/// and a full-window mount are not an OS window of their own.
pub(super) fn os_window(props: &ContainerProps, ctx: &Context, id: Id) -> Option<ViewportId> {
    match &props.mount {
        Mount::Viewport { .. } if viewports::available(ctx) => Some(viewports::viewport_id(id)),
        Mount::Window { .. } | Mount::Pane(_) | Mount::Inline => Some(pass::current_viewport(ctx)),
        Mount::Viewport { .. } | Mount::Layer(_) | Mount::FullWindow => None,
    }
}

/// The floor of a resizable layer's size: the larger of the style minimum and the overlay's least size.
fn window_min(props: &ContainerProps, style_min: Vec2) -> Vec2 {
    overlay_of(props).map_or(style_min, |overlay| style_min.max(overlay.size_limits.min))
}

/// The instance id of the `window` command scope a movable or resizable layer container holds (16.3), whatever
/// scope its own commands use: the container's id when it publishes the window scope itself, else a sub-id.
pub(super) fn window_instance(props: &ContainerProps, id: Id) -> Option<Id> {
    let overlay = overlay_of(props)?;
    (overlay.movable || overlay.resize.grips).then(|| if props.command_scope == Some(WindowCommand::SCOPE) { id } else { sub(id, "window") })
}

/// The keyboard mode of a floating container (16.3): arrows move or resize it until Enter or Escape.
#[derive(Clone, Copy, PartialEq)]
enum WindowModeKind {
    Move,
    Size,
}

/// A container's window gesture, the keyboard mode or a pointer drag, with the placement Escape restores and the
/// rect the layer had when the gesture began. Kept in session memory.
#[derive(Clone, Default, PartialEq)]
pub(super) struct WindowMode {
    kind: Option<WindowModeKind>,
    restore: Placement,
    began: Option<Rect>,
}

/// The outer rect of a viewport mount's OS window as the container last learnt it: the window's placement, which the
/// container keeps as a layer keeps its own.
#[derive(Clone, Default)]
struct WindowPlaced(Option<Rect>);

/// The edge or corner a resize from `before` to `after` moved: on each axis whose length changed, the side that moved,
/// or the far side (bottom, end of the screen's x) when both did.
fn resized_edge(before: Rect, after: Rect) -> Option<ResizeEdge> {
    let vertical = (before.height() != after.height()).then(|| before.top() != after.top() && before.bottom() == after.bottom());
    let horizontal = (before.width() != after.width()).then(|| before.left() != after.left() && before.right() == after.right());
    Some(match (vertical, horizontal) {
        (None, None) => return None,
        (Some(true), None) => ResizeEdge::Top,
        (Some(false), None) => ResizeEdge::Bottom,
        (None, Some(true)) => ResizeEdge::Left,
        (None, Some(false)) => ResizeEdge::Right,
        (Some(true), Some(true)) => ResizeEdge::TopLeft,
        (Some(true), Some(false)) => ResizeEdge::TopRight,
        (Some(false), Some(true)) => ResizeEdge::BottomLeft,
        (Some(false), Some(false)) => ResizeEdge::BottomRight,
    })
}

/// What every change of a layer's rect keeps to: the least and most size, and the rect it stays inside.
struct Limits {
    min: Vec2,
    max: Vec2,
    bounds: Option<Rect>,
}

impl Limits {
    fn new(overlay: &crate::contract::Overlay, min: Vec2) -> Limits {
        Limits { min, max: overlay.size_limits.max.unwrap_or(Vec2::INFINITY).max(min), bounds: overlay.bounds }
    }

    /// `rect` moved by `delta`, kept inside the bounds.
    fn moved(&self, rect: Rect, delta: Vec2) -> Rect {
        let rect = rect.translate(delta);
        let Some(bounds) = self.bounds else { return rect };
        let keep = |low: f32, high: f32, bounds_low: f32, bounds_high: f32| low.min(bounds_high - (high - low)).max(bounds_low) - low;
        rect.translate(Vec2::new(keep(rect.min.x, rect.max.x, bounds.min.x, bounds.max.x), keep(rect.min.y, rect.max.y, bounds.min.y, bounds.max.y)))
    }

    /// `rect` with the edges `dx` and `dy` (-1: low edge, 1: high edge, 0: none) moved by `delta`, within the size
    /// limits and the bounds. The opposite edge stays.
    fn resized(&self, rect: Rect, dx: i8, dy: i8, delta: Vec2) -> Rect {
        let axis = |low: f32, high: f32, edge: i8, delta: f32, min: f32, max: f32, bounds: Option<(f32, f32)>| {
            let (bounds_low, bounds_high) = bounds.unwrap_or((f32::NEG_INFINITY, f32::INFINITY));
            match edge {
                -1 => ((low + delta).max((high - max).max(bounds_low)).min(high - min), high),
                1 => (low, (high + delta).min((low + max).min(bounds_high)).max(low + min)),
                _ => (low, high),
            }
        };
        let (left, right) = axis(rect.min.x, rect.max.x, dx, delta.x, self.min.x, self.max.x, self.bounds.map(|bounds| (bounds.min.x, bounds.max.x)));
        let (top, bottom) = axis(rect.min.y, rect.max.y, dy, delta.y, self.min.y, self.max.y, self.bounds.map(|bounds| (bounds.min.y, bounds.max.y)));
        Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom))
    }
}

/// The edge or corner the grip with direction (`dx`, `dy`) of `GRIPS` sits on.
fn edge_of(dx: i8, dy: i8) -> ResizeEdge {
    match (dx, dy) {
        (-1, 0) => ResizeEdge::Left,
        (1, 0) => ResizeEdge::Right,
        (0, -1) => ResizeEdge::Top,
        (-1, -1) => ResizeEdge::TopLeft,
        (1, -1) => ResizeEdge::TopRight,
        (-1, 1) => ResizeEdge::BottomLeft,
        (1, 1) => ResizeEdge::BottomRight,
        _ => ResizeEdge::Bottom,
    }
}

/// The arrow chords, with and without Shift, a window mode holds, with the direction each moves.
const MODE_ARROWS: [(Key, [i8; 2]); 4] = [(Key::Left, [-1, 0]), (Key::Right, [1, 0]), (Key::Up, [0, -1]), (Key::Down, [0, 1])];

/// The eight edge and corner grips of a resizable layer, in the order of `GRIPS`: which edges each moves.
const GRIPS: [(i8, i8, CursorIcon); 8] = [
    (-1, 0, CursorIcon::ResizeHorizontal),
    (1, 0, CursorIcon::ResizeHorizontal),
    (0, -1, CursorIcon::ResizeVertical),
    (0, 1, CursorIcon::ResizeVertical),
    (-1, -1, CursorIcon::ResizeNwSe),
    (1, 1, CursorIcon::ResizeNwSe),
    (1, -1, CursorIcon::ResizeNeSw),
    (-1, 1, CursorIcon::ResizeNeSw),
];

impl BodyHooks<'_> {
    /// The `window.Move` and `window.Size` commands and the keys of the mode they start (16.3): arrows move or
    /// resize by a step (Shift: four steps), Enter, the command again or a primary press keeps the placement, Escape
    /// restores it, and focus leaving the container ends the mode. The mode holds its chords through the key stage, so no
    /// component under focus sees them. Each step is reported as a `PlacementKind::Keyboard` change.
    pub(super) fn window_keys<C: Container>(&mut self, props: &ContainerProps, cx: &mut Cx<'_, C>) {
        let (id, ctx) = (cx.core.id, cx.core.ctx.clone());
        let (Some(window), Some(overlay)) = (window_instance(props, id), overlay_of(props)) else { return };
        let limits = Limits::new(overlay, window_min(props, cx.style.metrics().min_size));
        let rect = cx.core.rect;
        let mut slot = PlacementSlot::new(&mut *cx.core.memory, id, self.save.as_ref(), overlay);
        let current = slot.read();
        let step = |rect: Rect, phase: GesturePhase| PlacementChange { rect, phase, pointer: None, kind: PlacementKind::Keyboard };
        let started = commands::take_for_instance(&ctx, Some(window), false, WindowCommand::from_ref).map(|(command, _)| command);
        let mut mode = self.mode.clone();
        if let Some(command) = started {
            let kind = match command {
                WindowCommand::Move => WindowModeKind::Move,
                _ => WindowModeKind::Size,
            };
            if mode.kind.is_some() {
                self.reports.push(step(current.rect_over(rect), GesturePhase::Commit));
            }
            if mode.kind == Some(kind) {
                mode = WindowMode::default();
            } else {
                mode = WindowMode { kind: Some(kind), restore: current.clone(), began: Some(current.rect_over(rect)) };
                self.reports.push(step(current.rect_over(rect), GesturePhase::Begin));
            }
        }
        // A primary press ends the mode and keeps the placement reached, like Enter.
        let pressed = || pointer::with_frame(&ctx, |frame, _| frame.is_some_and(|frame| frame.press.is_some()));
        if mode.kind.is_some() && (!commands::focus_within(&ctx, id) || pressed()) {
            self.reports.push(step(current.rect_over(rect), GesturePhase::Commit));
            mode = WindowMode::default();
        }
        if let Some(kind) = mode.kind {
            let unit = self.nudge;
            let mut placement = current;
            for (key, direction) in MODE_ARROWS {
                for (mods, scale) in [(Mods::NONE, 1.0), (Mods::SHIFT, 4.0)] {
                    let chord = Chord::new(mods, key);
                    keys::capture(&ctx, id, chord);
                    while keys::take_claimed(&ctx, id, chord) {
                        let delta = Vec2::new(f32::from(direction[0]), f32::from(direction[1])) * unit * scale;
                        let before = placement.rect_over(rect);
                        let next = match kind {
                            WindowModeKind::Move => limits.moved(before, delta),
                            WindowModeKind::Size => limits.resized(before, 1, 1, delta),
                        };
                        placement = Placement::at(next);
                        self.reports.push(step(next, GesturePhase::Update));
                    }
                }
            }
            for key in [Key::Enter, Key::Escape] {
                let chord = Chord::new(Mods::NONE, key);
                keys::capture(&ctx, id, chord);
                if keys::take_claimed(&ctx, id, chord) {
                    if key == Key::Escape {
                        placement = mode.restore.clone();
                        self.reports.push(step(mode.began.unwrap_or(rect), GesturePhase::Cancel));
                    } else {
                        self.reports.push(step(placement.rect_over(rect), GesturePhase::Commit));
                    }
                    mode = WindowMode::default();
                }
            }
            slot.write(placement);
        }
        self.mode = mode;
    }

    /// A drag of the move band or a grip of a layer (not an OS window): the layer follows the pointer within the
    /// limits, and each step is reported. Escape, which cancels the press, restores the placement from before.
    pub(super) fn window_drag<C: Container>(&mut self, props: &ContainerProps, cx: &mut Cx<'_, C>) {
        let Some(overlay) = overlay_of(props) else { return };
        // A band that moves the window below starts the OS move of the window the layer is shown in.
        if overlay.moves_window {
            let viewport = pass::current_viewport(&cx.core.ctx);
            self.window_os(viewport, cx);
            return;
        }
        let id = cx.core.id;
        let limits = Limits::new(overlay, window_min(props, cx.style.metrics().min_size));
        let rect = cx.core.rect;
        // A part the container did not declare this pass (no band, no grips) has no result.
        let phase = |key: (PartKey, Option<u64>)| cx.core.results.get(&key).map(|result| result.press.phase);
        let mut pressed = vec![(None, phase((MOVE_BAND, None)))];
        pressed.extend((0..GRIPS.len()).map(|index| (Some(index), phase((GRIP, Some(index as u64))))));
        let Some((grip, phase)) = pressed.into_iter().find_map(|(grip, phase)| match phase? {
            phase @ (PressPhase::DragStart { .. } | PressPhase::Drag { .. } | PressPhase::Release { .. } | PressPhase::Cancel { .. }) => {
                Some((grip, phase))
            }
            _ => None,
        }) else {
            return;
        };
        let mut slot = PlacementSlot::new(&mut *cx.core.memory, id, self.save.as_ref(), overlay);
        let current = slot.read();
        let (origin, pos, gesture) = match phase {
            PressPhase::DragStart { start, pos, .. } => {
                self.mode.began = Some(current.rect_over(rect));
                self.mode.restore = current.clone();
                (start.origin, Some(pos), GesturePhase::Begin)
            }
            PressPhase::Drag { start, pos, .. } => (start.origin, Some(pos), GesturePhase::Update),
            PressPhase::Release { start, pos, .. } => (start.origin, Some(pos), GesturePhase::Commit),
            PressPhase::Cancel { start, .. } => (start.origin, None, GesturePhase::Cancel),
            _ => return,
        };
        let Some(began) = self.mode.began else { return };
        let (dx, dy, _) = grip.map_or((0, 0, CursorIcon::Grab), |index| GRIPS[index]);
        let kind = if grip.is_some() { PlacementKind::Size(edge_of(dx, dy)) } else { PlacementKind::Move };
        let next = match pos {
            Some(pos) if grip.is_some() => limits.resized(began, dx, dy, pos - origin),
            Some(pos) => limits.moved(began, pos - origin),
            None => began,
        };
        let change = |rect: Rect, phase: GesturePhase| PlacementChange { rect, phase, pointer: pos, kind };
        if gesture == GesturePhase::Begin {
            self.reports.push(change(began, GesturePhase::Begin));
        }
        self.reports.push(change(next, if gesture == GesturePhase::Begin { GesturePhase::Update } else { gesture }));
        // A move leaves a content-sized layer its size; a resize fixes both.
        slot.write(match (gesture, grip) {
            (GesturePhase::Cancel, _) => self.mode.restore.clone(),
            (_, Some(_)) => Placement::at(next),
            (_, None) => Placement { pos: Some(next.min.into()), size: current.size },
        });
        if matches!(gesture, GesturePhase::Commit | GesturePhase::Cancel) {
            self.mode.began = None;
        }
    }

    /// A press on the move band or a grip of a container in an OS window starts the OS move or resize. Where the
    /// platform answers the hit test (`answered_by_platform`), the parts stand for its sink only.
    pub(super) fn window_os<C: Container>(&self, viewport: ViewportId, cx: &Cx<'_, C>) {
        if answered_by_platform(&cx.core.ctx, viewport) {
            return;
        }
        // The parts stand only by `WindowParts`, and the results of a part not registered this pass are gone.
        let started =
            |key: (PartKey, Option<u64>)| cx.core.results.get(&key).is_some_and(|result| matches!(result.press.phase, PressPhase::DragStart { .. }));
        if started((MOVE_BAND, None)) {
            windows::act(&cx.core.ctx, viewport, WindowAction::StartMove);
        }
        for (index, (dx, dy, _)) in GRIPS.into_iter().enumerate() {
            if started((GRIP, Some(index as u64))) {
                windows::act(&cx.core.ctx, viewport, WindowAction::StartResize(edge_of(dx, dy)));
            }
        }
    }

    /// A viewport mount in an OS window reports the window's moves and resizes from its facts, through the
    /// `placement_changed` a layer reports its own through: one `Commit` per change, as the platform tells no gesture
    /// phase. The first facts are the window's placement, not a change.
    pub(super) fn window_placed<C: Container>(&mut self, props: &ContainerProps, cx: &mut Cx<'_, C>) {
        if !matches!(props.mount, Mount::Viewport { .. }) {
            return;
        }
        let Some(viewport) = os_window(props, &cx.core.ctx, cx.core.id) else { return };
        let Some(facts) = windows::WindowHost::of(&cx.core.ctx).facts(viewport) else { return };
        let now = facts.outer.unwrap_or(facts.inner);
        let placed = &mut cx.core.memory.shown::<WindowPlaced>(sub(sub(cx.core.id, "body"), "window_placed")).0;
        let Some(before) = placed.replace(now).filter(|before| *before != now) else { return };
        let kind = resized_edge(before, now).map_or(PlacementKind::Move, PlacementKind::Size);
        self.reports.push(PlacementChange { rect: now, phase: GesturePhase::Commit, pointer: None, kind });
    }

    /// The move band and the grips (16.3). A layer's move and resize the layer; those of an OS window (`os_window`)
    /// stand by `WindowParts` and join the window's exclusive list, which the hit-test sink reads. A layer's parts never
    /// join it, or the sink would move the OS window from a dialog inside it, except the band of a layer that moves the
    /// window it is shown in (`Overlay::moves_window`): that band is a drag area of the window.
    pub(super) fn move_parts<C: Container>(props: &ContainerProps, regions: &Regions, rect: Rect, pcx: &mut PartsCx<'_, C>) {
        let ctx = pcx.ui.ctx().clone();
        let handle = pcx.tokens.tokens().size.handle.sm;
        let reason = "the Move and Size commands of the window command set do the same";
        let os = os_window(props, &ctx, pcx.id);
        let stands = os.map_or(WindowParts::LAYER, |viewport| WindowParts::of(&ctx, viewport));
        let exclusive = |hit: WindowHit| os.is_some().then_some(hit);
        let moves_window = matches!(&props.mount, Mount::Layer(overlay) | Mount::Viewport { fallback: overlay, .. } if overlay.moves_window);
        // A layer moves when its overlay says so; other mounts follow the header's band.
        let movable = match &props.mount {
            // A band that moves the window below follows the rule of the window's own band (a title bar's); where the
            // platform answers the hit test, the sink reads it from the exclusive list.
            _ if moves_window => WindowParts::of(&ctx, pass::current_viewport(&ctx)).band,
            Mount::Layer(overlay) | Mount::Viewport { fallback: overlay, .. } => overlay.movable && stands.band,
            _ => props.header.is_some_and(|header| header.band == crate::contract::DragBand::Move) && stands.band,
        };
        let band = move_band(props, regions, rect);
        if props.header.is_some() && movable && band.is_positive() {
            // The band lies under the line's own parts and children, like the reorder band, so they keep their presses.
            let part = pcx
                .part(MOVE_BAND, band)
                .sense(craft_behavior::Sense::DRAG)
                .cursor(CursorIcon::Grab)
                .drag_cursor(CursorIcon::Grabbing)
                .pointer_only(reason);
            if let Some(hit) = exclusive(WindowHit::Move).or(moves_window.then_some(WindowHit::Move)) {
                part.exclusive(hit);
            }
        }
        // The grips of a layer straddle its edge; those of a window lie inside it, where the window has pixels. They
        // stack above the body (`over`), so a root in the body never covers their inner half (8.3).
        let (grips, inset) = match &props.mount {
            Mount::Layer(overlay) | Mount::Viewport { fallback: overlay, .. } => (overlay.resize.grips, 0.0),
            Mount::Window { resize } => (resize.grips, handle / 2.0),
            _ => (false, 0.0),
        };
        if !grips || !stands.grips {
            return;
        }
        for (index, (dx, dy, cursor)) in GRIPS.into_iter().enumerate() {
            let along = |edge: i8, low: f32, high: f32| match edge {
                -1 => (low - handle / 2.0 + inset, low + handle / 2.0 + inset),
                1 => (high - handle / 2.0 - inset, high + handle / 2.0 - inset),
                _ => (low + handle / 2.0 + inset, high - handle / 2.0 - inset),
            };
            let (x0, x1) = along(dx, rect.min.x, rect.max.x);
            let (y0, y1) = along(dy, rect.min.y, rect.max.y);
            let level = if dx != 0 && dy != 0 { HitLevel::Handle } else { HitLevel::Edge };
            let part = pcx
                .part(GRIP.at(index as u64), Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1)))
                .sense(craft_behavior::Sense::DRAG)
                .priority(HitPriority::new(level, 0))
                .cursor(cursor)
                .pointer_only(reason)
                .over();
            if let Some(hit) = exclusive(WindowHit::Resize(edge_of(dx, dy))) {
                part.exclusive(hit);
            }
        }
    }

    /// Tells the platform whether the OS may move the container's OS window from where the pointer is (21.2
    /// `WindowDrag`; macOS `NSWindow.movable`): only where a press would go to the move band under the press-target
    /// rule (8.3), so a press on a control over the band never drags the window. Bands of one window combine: any band
    /// that would take the press allows the move. A pass with no band writes nothing; `AppShell` writes `false` for a
    /// pass with no title bar.
    pub(super) fn window_movable<C: Container>(&self, cx: &Cx<'_, C>) {
        let Some(band) = cx.core.results.get(&(MOVE_BAND, None)) else { return };
        let ctx = &cx.core.ctx;
        let takes_press = band.hovered
            && pointer::pass_pointer(ctx).hover.is_some_and(|at| {
                let (click, drag) = input::hit_targets(ctx);
                pointer::press_winner(ctx, at, pointer::PressTargets { click, drag })
                    .is_some_and(|(winner, root)| root == cx.core.id && winner.part.is_some_and(|part| part.key == MOVE_BAND))
            });
        let this_pass = viewports::pass_number(ctx);
        let allowed = viewports::movable_record(ctx).is_some_and(|(pass, movable)| pass == this_pass && movable);
        viewports::write_movable(ctx, allowed || takes_press);
    }
}

/// The move band of a container whose header drags: an inline header's band spans the header's whole line (header,
/// body and end region), beneath the line's children, so every free point of the line drags; a header on a line of
/// its own is the band.
fn move_band(props: &ContainerProps, regions: &Regions, rect: Rect) -> Rect {
    if !props.header.is_some_and(|header| header.inline) {
        return regions.header;
    }
    let line = regions.header.union(regions.body);
    let line = if props.end.is_some() { line.union(regions.end) } else { line };
    line.intersect(rect)
}
