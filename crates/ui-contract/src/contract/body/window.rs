//! Move and resize of a layer or OS window: the move band, the grips, the keyboard mode, limits and OS window facts.

use super::BodyHooks;
use super::placement::{Placement, PlacementSlot, overlay_of};
use super::regions::Regions;
use crate::contract::cx::CxCore;
use crate::contract::overlay::{PlacementChange, PlacementKind, ResizeEdges};
use crate::contract::{Container, ContainerProps, Cx, Mount, PartsCx};
use crate::runtime::commands::{self, CommandSet as _, WindowCommand};
use crate::runtime::egui::pass;
use crate::runtime::egui::viewports;
use crate::runtime::env::{EnvView, ShellEntry};
use crate::runtime::memory::sub;
use crate::runtime::windows::{self, ResizeEdge, WindowAction, WindowHit};
use crate::runtime::{keys, pointer};
use crate::style::{ComponentStyle as _, UiTokens};
use craft_behavior::{CursorIcon, GesturePhase, HitLevel, HitPriority, PartKey, PointerEvent, PressPhase};
use craft_host::target::{Caps, WindowChrome, WindowState};
use craft_keymap::{Chord, Key, Mods};
use egui::{Context, Id, Pos2, Rect, Vec2, ViewportId};

pub(crate) const MOVE_BAND: PartKey = PartKey::new("move_band");

const GRIP: PartKey = PartKey::new("resize_grip");

/// Which move and resize parts of the body step stand for the OS window of `viewport` (16.3, 21.2): the one rule
/// every move band and grip of an OS window reads. Both need a frame the app draws (the chrome is not `Native`). The
/// band stands on a maximized window too, so its double click restores the window and a drag is the platform's (it
/// restores and moves); the grips stand only on a window that is neither maximized nor full screen, nor one the OS
/// resizes from its own frame (`windows::os_resizes`: AppKit's edges, where app grips would contest its presses). The
/// parts stand whether or not a hit-test sink is installed: the sink reads them as the window's exclusive parts.
#[derive(Clone, Copy)]
struct WindowParts {
    band: bool,
    grips: bool,
}

impl WindowParts {
    /// The parts of a layer or a full-window mount, which move the container and never the OS window.
    const LAYER: WindowParts = WindowParts { band: true, grips: true };
    /// The parts of a container that stands for no OS window and is no layer: none.
    const NONE: WindowParts = WindowParts { band: false, grips: false };

    fn of(ctx: &Context, viewport: ViewportId) -> WindowParts {
        let window = windows::shell_of(ctx, viewport).window;
        let framed_by_app = window.chrome != WindowChrome::Native;
        let sized_by_os = matches!(window.state, WindowState::Maximized | WindowState::Fullscreen) || windows::os_resizes(ctx, viewport);
        WindowParts { band: framed_by_app && window.state != WindowState::Fullscreen, grips: framed_by_app && !sized_by_os }
    }
}

/// The band a layer keeps clear at its top for its top grip, so that grip never lies over the header: the grip's
/// width ([`grip_band`]) on a layer whose header is on a line of its own and whose top grip or a top corner's stands,
/// or that the user moves: a floating window keeps the band while it has no top grip (collapsed, minimized), so its
/// header stays where it was. None elsewhere, nor on a layer whose band moves the window below it. The regions start
/// below the band, or below the padding where that is deeper. An OS window keeps none: its grips, the platform's or
/// the app's, lie over its title bar as an OS title bar's resize edge does.
pub(super) fn top_band(props: &ContainerProps, ctx: &Context, id: Id, tokens: &UiTokens) -> f32 {
    let overlay = match &props.mount {
        Mount::Layer(overlay) => overlay,
        Mount::Viewport { fallback, .. } if !windows::mount_window(ctx, id) => fallback,
        _ => return 0.0,
    };
    if !props.header.is_some_and(|header| !header.inline) || overlay.moves_window {
        return 0.0;
    }
    let pinned = Pinned::of(overlay, ctx, id, false);
    let top = [(0, -1), (-1, -1), (1, -1)].into_iter().any(|(dx, dy)| grip_stands(overlay.resize.edges, pinned, dx, dy));
    if top || overlay.movable { grip_band(tokens) } else { 0.0 }
}

/// The width of a window's edge grips, and of the band a layer keeps clear above its header for its top grip:
/// `size.handle.sm`. A corner grip reaches twice as far along each of its edges.
pub fn grip_band(tokens: &UiTokens) -> f32 {
    tokens.tokens().size.handle.sm
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
/// and a full-window mount are not an OS window of their own. Nor is a `Window`, `Pane` or `Inline` container inside an
/// embedded shell (`ShellCx::region`): the shell fills a region of another window and never acts on that window, so a
/// title bar in an app preview moves, frames and reports nothing.
pub(super) fn os_window(props: &ContainerProps, ctx: &Context, id: Id, env: &EnvView<'_>) -> Option<ViewportId> {
    let embedded = env.get::<ShellEntry>().0.is_some_and(|shell| shell.region().is_some());
    match &props.mount {
        Mount::Viewport { .. } if windows::mount_window(ctx, id) => Some(viewports::viewport_id(id)),
        Mount::Window { .. } | Mount::Pane(_) | Mount::Inline if !embedded => Some(pass::current_viewport(ctx)),
        Mount::Window { .. } | Mount::Pane(_) | Mount::Inline | Mount::Viewport { .. } | Mount::Layer(_) | Mount::FullWindow => None,
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
    (overlay.movable || overlay.resize.edges != ResizeEdges::NONE)
        .then(|| if props.command_scope == Some(WindowCommand::SCOPE) { id } else { sub(id, "window") })
}

/// The keyboard mode of a floating container (16.3): arrows move or resize it until Enter or Escape.
#[derive(Clone, Copy, PartialEq)]
enum WindowModeKind {
    Move,
    Size,
}

/// A container's window gesture, the keyboard mode or a pointer drag, with the placement Escape restores and the
/// rect the layer had when the gesture began, or the move of its OS window that the app runs. Kept in session memory.
#[derive(Clone, Default, PartialEq)]
pub(super) struct WindowMode {
    kind: Option<WindowModeKind>,
    restore: Placement,
    began: Option<Rect>,
    /// The rect the layer had when a press went down on its move band or a grip, until the press ends: a drag that
    /// starts from the press runs from it.
    pressed: Option<Rect>,
    moving: Option<WindowMove>,
    sizing: Option<WindowSize>,
    /// The index in `GRIPS` of the grip a drag holds, from its `DragStart` to its release: `move_parts` keeps it
    /// declared when the drag flips the layer to a side whose placement holds its edge, so the drag keeps its result.
    held: Option<usize>,
}

impl WindowMode {
    /// The grip a drag holds, for `move_parts`.
    pub(super) fn held(&self) -> Option<usize> {
        self.held
    }

    /// The rect a drag of the move band of the layer `id` puts the layer at in this pass, before the bounds keep it:
    /// the rect the layer had when the press went down, moved by the pointer's travel since. `window_drag` reports the
    /// same rect for the step. `None` while no move by the band runs. The layer host shows the layer there
    /// (`shown_placement`) before the container's pass runs, so the layer stands under the pointer in the frame of
    /// each step, whoever owns its placement.
    pub(super) fn running_move(&self, ctx: &Context, id: Id) -> Option<Rect> {
        if self.pressed.is_none() && self.began.is_none() {
            return None;
        }
        let (began, start, pos) = match crate::primitives::sensing::part_press(ctx, id, MOVE_BAND.into()).press.phase {
            PressPhase::DragStart { start, pos, .. } => (self.pressed?, start, pos),
            PressPhase::Drag { start, pos, .. } | PressPhase::Release { start, pos, .. } => (self.began?, start, pos),
            _ => return None,
        };
        Some(began.translate(pos - start.origin))
    }
}

/// A move of a viewport mount's OS window that the app runs (`Overlay::moved_by_app`), from its Begin to its Commit
/// or Cancel. Rects are the window's outer rect in desktop points.
#[derive(Clone, Copy, PartialEq)]
struct WindowMove {
    /// The pressed point in the window's inner coordinates, which stays under the pointer.
    grab: Vec2,
    /// Where the window was when the move began; a cancel puts it back there.
    start: Rect,
    /// Where the move put the window last.
    at: Rect,
    /// The OS moves the window and the app follows the system pointer ([`Caps::OS_MOVES_TRACKED`]).
    tracked: bool,
    /// The pointer has passed the click distance from the press, and the move is reported. A tracked move is handed
    /// to the OS on the press, and stays a click, which moves and docks nothing, until then.
    begun: bool,
}

/// A resize of a viewport mount's OS window that its caller sizes ([`Resize::by_caller`](crate::contract::overlay::Resize::by_caller)),
/// which the app runs, from its Begin to its Commit or Cancel. Rects are the window's outer rect in desktop points.
#[derive(Clone, Copy, PartialEq)]
struct WindowSize {
    /// The index in `GRIPS` of the grip the drag holds.
    grip: usize,
    /// The pressed point in desktop points.
    pressed: Pos2,
    /// Where the window was when the resize began; a cancel reports it again.
    start: Rect,
    /// The rect the resize reported last.
    at: Rect,
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

/// `rect` moved, at its size, the least distance that puts it inside `bounds`: the one keep rule for a layer's
/// bounds, whether the user moves the layer or the host places it. On an axis where it is longer than the bounds, its
/// low edge sits on theirs.
pub fn keep_inside(rect: Rect, bounds: Rect) -> Rect {
    let keep = |low: f32, high: f32, bounds_low: f32, bounds_high: f32| low.min(bounds_high - (high - low)).max(bounds_low) - low;
    rect.translate(Vec2::new(keep(rect.min.x, rect.max.x, bounds.min.x, bounds.max.x), keep(rect.min.y, rect.max.y, bounds.min.y, bounds.max.y)))
}

/// `rect` kept inside `bounds`, when the layer has bounds.
pub(super) fn kept_in(rect: Rect, bounds: Option<Rect>) -> Rect {
    bounds.map_or(rect, |bounds| keep_inside(rect, bounds))
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
        kept_in(rect.translate(delta), self.bounds)
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

/// The move band and the resize grips the body step declared for a container this pass, for a host that names them
/// (an automation interface that drives a window by its grips). Each comes with the id of its accessibility node, the
/// id [`focused_node`](crate::runtime::access::focused_node) reports while it holds focus.
impl<C: Container> Cx<'_, C> {
    /// The band the container is moved by, `None` when it has none this pass (no header, not movable).
    pub fn move_band(&self) -> Option<(Rect, Id)> {
        let band = self.core.results.get(&(MOVE_BAND, None))?;
        Some((band.rect, crate::runtime::access::sub_node_id(self.core.id, MOVE_BAND.into())))
    }

    /// The edge and corner grips the container is resized by, each with the edge or corner it moves; none when it does
    /// not resize this pass (minimized, maximized, or a window the platform resizes from its own frame).
    pub fn resize_grips(&self) -> Vec<(ResizeEdge, Rect, Id)> {
        GRIPS
            .into_iter()
            .enumerate()
            .filter_map(|(index, (dx, dy, _))| {
                let part = GRIP.at(index as u64);
                let grip = self.core.results.get(&(GRIP, Some(index as u64)))?;
                Some((edge_of(dx, dy), grip.rect, crate::runtime::access::sub_node_id(self.core.id, part)))
            })
            .collect()
    }
}

/// The arrow chords, with and without Shift, a window mode holds, with the direction each moves.
const MODE_ARROWS: [(Key, [i8; 2]); 4] = [(Key::Left, [-1, 0]), (Key::Right, [1, 0]), (Key::Up, [0, -1]), (Key::Down, [0, 1])];

/// The eight edge and corner grips of a resizable layer, in the order of `GRIPS`: which edges each moves.
const GRIPS: [(i8, i8, CursorIcon); 8] = [
    (-1, 0, CursorIcon::ResizeWest),
    (1, 0, CursorIcon::ResizeEast),
    (0, -1, CursorIcon::ResizeNorth),
    (0, 1, CursorIcon::ResizeSouth),
    (-1, -1, CursorIcon::ResizeNorthWest),
    (1, 1, CursorIcon::ResizeSouthEast),
    (1, -1, CursorIcon::ResizeNorthEast),
    (-1, 1, CursorIcon::ResizeSouthWest),
];

/// The edges of a layer its placement holds in place, by screen side (`-1` left or top, `1` right or bottom). A layer
/// hung from its anchor that the user cannot move keeps only its size, so its placement holds the edge that faces the
/// anchor and the edge it aligns to the anchor's: a grip on such an edge would move the opposite one. A centred layer
/// has both edges across its side held.
#[derive(Clone, Copy)]
struct Pinned {
    x: [bool; 2],
    y: [bool; 2],
}

impl Pinned {
    const NONE: Pinned = Pinned { x: [false; 2], y: [false; 2] };

    fn of(overlay: &crate::contract::Overlay, ctx: &Context, id: Id, rtl: bool) -> Pinned {
        use craft_behavior::overlay::{Align, Side};
        let crate::contract::overlay::Place::Anchored { side, align, .. } = &overlay.place else { return Pinned::NONE };
        if overlay.movable {
            return Pinned::NONE;
        }
        let side = crate::runtime::overlay::placed_side(ctx, id).unwrap_or(*side);
        // Index 0 is the left or top edge, 1 the right or bottom one; reading order flips the horizontal ones.
        let (start, end) = if rtl { (1, 0) } else { (0, 1) };
        let edge = |at: usize| {
            let mut held = [false; 2];
            held[at] = true;
            held
        };
        let aligned = |start: usize, end: usize| match align {
            Align::Start => edge(start),
            Align::End => edge(end),
            _ => [true; 2],
        };
        match side {
            Side::Below => Pinned { x: aligned(start, end), y: edge(0) },
            Side::Above => Pinned { x: aligned(start, end), y: edge(1) },
            Side::Start => Pinned { x: edge(end), y: aligned(0, 1) },
            Side::End => Pinned { x: edge(start), y: aligned(0, 1) },
            _ => Pinned::NONE,
        }
    }

    /// Whether the grip that moves the edges `dx`, `dy` moves a held edge.
    fn holds(self, dx: i8, dy: i8) -> bool {
        let held = |edges: [bool; 2], d: i8| d != 0 && edges[usize::from(d > 0)];
        held(self.x, dx) || held(self.y, dy)
    }
}

/// The edges the grip with direction (`dx`, `dy`) of `GRIPS` moves.
fn moved_edges(dx: i8, dy: i8) -> ResizeEdges {
    let x = match dx {
        -1 => ResizeEdges::LEFT,
        1 => ResizeEdges::RIGHT,
        _ => ResizeEdges::NONE,
    };
    let y = match dy {
        -1 => ResizeEdges::TOP,
        1 => ResizeEdges::BOTTOM,
        _ => ResizeEdges::NONE,
    };
    x.union(y)
}

/// The grip rule every resize of a layer reads: the grip with direction (`dx`, `dy`) stands when every edge it moves
/// is in `edges` and the placement leaves it free. An explicit edge set never overrides the placement.
fn grip_stands(edges: ResizeEdges, pinned: Pinned, dx: i8, dy: i8) -> bool {
    edges.contains(moved_edges(dx, dy)) && !pinned.holds(dx, dy)
}

/// The edges keyboard Size mode moves, by the grip rule: on each axis the high edge (right, bottom) when its grip
/// stands, else the low one when its grip does, else none (`1`, `-1`, `0`).
fn size_edges(edges: ResizeEdges, pinned: Pinned) -> (i8, i8) {
    let axis = |high: (i8, i8), low: (i8, i8)| {
        if grip_stands(edges, pinned, high.0, high.1) {
            1
        } else if grip_stands(edges, pinned, low.0, low.1) {
            -1
        } else {
            0
        }
    };
    (axis((1, 0), (-1, 0)), axis((0, 1), (0, -1)))
}

impl BodyHooks<'_> {
    /// The `window.Move` and `window.Size` commands and the keys of the mode they start (16.3): arrows move or
    /// resize by a step (Shift: four steps), Enter, the command again or a primary press keeps the placement, Escape
    /// restores it, and focus leaving the container ends the mode. The mode holds its chords through the key stage, so no
    /// component under focus sees them. Each step is reported as a `PlacementKind::Keyboard` change.
    pub(super) fn window_keys<C: Container>(&mut self, props: &ContainerProps, cx: &mut Cx<'_, C>) {
        let (id, ctx) = (cx.core.id, cx.core.ctx.clone());
        let (Some(window), Some(overlay)) = (window_instance(props, id), overlay_of(props)) else { return };
        let overlay = overlay.in_screen(crate::runtime::overlay::parent_space(&ctx, id));
        let overlay = &*overlay;
        let limits = Limits::new(overlay, window_min(props, cx.style.metrics().min_size));
        let rect = cx.core.rect;
        let rtl = cx.core.env.direction().is_rtl();
        let mut slot = PlacementSlot::new(&mut *cx.core.memory, id, self.save.as_ref(), overlay);
        let current = slot.read();
        let step = |rect: Rect, phase: GesturePhase| PlacementChange { rect, phase, pointer: None, kind: PlacementKind::Keyboard, continued: false };
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
                mode = WindowMode { kind: Some(kind), restore: current.clone(), began: Some(current.rect_over(rect)), ..WindowMode::default() };
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
            // Size mode moves the edges whose grips stand at this pass's side of the anchor.
            let (size_x, size_y) = size_edges(overlay.resize.edges, Pinned::of(overlay, &ctx, id, rtl));
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
                            // Left and Right act on the free horizontal edge only, Up and Down on the vertical one.
                            WindowModeKind::Size if direction[0] != 0 => limits.resized(before, size_x, 0, delta),
                            WindowModeKind::Size => limits.resized(before, 0, size_y, delta),
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
    /// limits, and each step is reported. The drag runs from the rect the layer had when the press went down. Escape,
    /// which cancels the press, restores the placement from before.
    pub(super) fn window_drag<C: Container>(&mut self, props: &ContainerProps, cx: &mut Cx<'_, C>) {
        let Some(overlay) = overlay_of(props) else { return };
        // A band that moves the window below starts the OS move of the window the layer is shown in.
        if overlay.moves_window {
            let viewport = pass::current_viewport(&cx.core.ctx);
            self.window_os(viewport, props, cx);
            return;
        }
        let id = cx.core.id;
        let overlay = overlay.in_screen(crate::runtime::overlay::parent_space(&cx.core.ctx, id));
        let overlay = &*overlay;
        let limits = Limits::new(overlay, window_min(props, cx.style.metrics().min_size));
        let rect = cx.core.rect;
        // A part the container did not declare this pass (no band, no grips) has no result.
        let phase = |key: (PartKey, Option<u64>)| cx.core.results.get(&key).map(|result| result.press.phase);
        let mut pressed = vec![(None, phase((MOVE_BAND, None)))];
        pressed.extend((0..GRIPS.len()).map(|index| (Some(index), phase((GRIP, Some(index as u64))))));
        let Some((grip, phase)) = pressed.into_iter().find_map(|(grip, phase)| match phase? {
            phase @ (PressPhase::Down { .. }
            | PressPhase::DragStart { .. }
            | PressPhase::Drag { .. }
            | PressPhase::Release { .. }
            | PressPhase::Cancel { .. }) => Some((grip, phase)),
            _ => None,
        }) else {
            return;
        };
        let mut slot = PlacementSlot::new(&mut *cx.core.memory, id, self.save.as_ref(), overlay);
        let current = slot.read();
        let (start, pos, gesture) = match phase {
            PressPhase::Down { .. } => {
                self.mode.pressed = Some(current.rect_over(rect));
                return;
            }
            PressPhase::DragStart { start, pos, .. } => {
                // The layer host shows this pass at the step's rect already (`running_move`): the drag runs from where
                // the layer stood at the press.
                self.mode.began = Some(self.mode.pressed.unwrap_or_else(|| current.rect_over(rect)));
                self.mode.restore = current.clone();
                self.mode.held = grip;
                (start, Some(pos), GesturePhase::Begin)
            }
            PressPhase::Drag { start, pos, .. } => (start, Some(pos), GesturePhase::Update),
            PressPhase::Release { start, pos, .. } => (start, Some(pos), GesturePhase::Commit),
            PressPhase::Cancel { start, .. } => (start, None, GesturePhase::Cancel),
            _ => return,
        };
        let origin = start.origin;
        if matches!(gesture, GesturePhase::Commit | GesturePhase::Cancel) {
            self.mode.pressed = None;
        }
        let Some(began) = self.mode.began else { return };
        let (dx, dy, _) = grip.map_or((0, 0, CursorIcon::Grab), |index| GRIPS[index]);
        let kind = if grip.is_some() { PlacementKind::Size(edge_of(dx, dy)) } else { PlacementKind::Move };
        let next = match pos {
            Some(pos) if grip.is_some() => limits.resized(began, dx, dy, pos - origin),
            Some(pos) => limits.moved(began, pos - origin),
            None => began,
        };
        // A move band press another component handed over (`Overlay::continue_move`) continues that component's drag.
        let continued = grip.is_none() && start.handed;
        let change = |rect: Rect, phase: GesturePhase| PlacementChange { rect, phase, pointer: pos, kind, continued };
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
            self.mode.held = None;
        }
    }

    /// A press on the move band or a grip of a container in an OS window starts the OS move or resize; the band of a
    /// window whose move the app runs starts that move instead ([`Self::window_move`]), and a grip of a window its
    /// caller sizes that resize ([`Self::window_resize`]). Where the platform answers the hit test
    /// (`answered_by_platform`), the parts it takes stand for its sink only; the grips of a window its caller sizes
    /// are never its.
    pub(super) fn window_os<C: Container>(&mut self, viewport: ViewportId, props: &ContainerProps, cx: &mut Cx<'_, C>) {
        let by_caller = overlay_of(props).is_some_and(|overlay| overlay.resize.by_caller);
        if by_caller {
            let limits = overlay_of(props).map(|overlay| Limits::new(overlay, window_min(props, cx.style.metrics().min_size)));
            if let Some(limits) = limits {
                self.window_resize(viewport, &limits, cx.core);
            }
        }
        if answered_by_platform(&cx.core.ctx, viewport) {
            return;
        }
        let app_moves = windows::app_moves(&cx.core.ctx, viewport);
        if app_moves {
            self.window_move(viewport, cx.core);
        }
        // The parts stand only by `WindowParts`, and the results of a part not registered this pass are gone.
        let started =
            |key: (PartKey, Option<u64>)| cx.core.results.get(&key).is_some_and(|result| matches!(result.press.phase, PressPhase::DragStart { .. }));
        if !app_moves && started((MOVE_BAND, None)) {
            windows::act(&cx.core.ctx, viewport, WindowAction::StartMove);
        }
        for (index, (dx, dy, _)) in GRIPS.into_iter().enumerate() {
            if !by_caller && started((GRIP, Some(index as u64))) {
                windows::act(&cx.core.ctx, viewport, WindowAction::StartResize(edge_of(dx, dy)));
            }
        }
    }

    /// The resize of an OS window its caller sizes: a drag of a grip moves the edges the grip holds by the pointer's
    /// travel since the press, within `limits`, and each step is reported through `placement_changed` as a layer's
    /// resize is, from Begin to Commit or Cancel. The app sends the window no size: the caller places the size it
    /// makes of the steps. The press holds the pointer, so its moves arrive while it is outside the window. A pointer
    /// move of this pass is relative to where the window reports itself now, as in [`Self::window_move`].
    fn window_resize(&mut self, viewport: ViewportId, limits: &Limits, core: &mut CxCore<'_>) {
        let ctx = core.ctx.clone();
        let Some(facts) = windows::WindowHost::of(&ctx).facts(viewport) else { return };
        let outer = facts.outer.unwrap_or(facts.inner);
        let phase = |index: usize| core.results.get(&(GRIP, Some(index as u64))).map(|result| result.press.phase);
        let change = |rect: Rect, phase: GesturePhase, grip: usize| {
            let (dx, dy, _) = GRIPS[grip];
            PlacementChange { rect, phase, pointer: None, kind: PlacementKind::Size(edge_of(dx, dy)), continued: false }
        };
        let resized = |sizing: &WindowSize, desktop: Pos2| {
            let (dx, dy, _) = GRIPS[sizing.grip];
            limits.resized(sizing.start, dx, dy, desktop - sizing.pressed)
        };
        let Some(mut sizing) = self.mode.sizing else {
            let started = (0..GRIPS.len()).find_map(|index| match phase(index)? {
                PressPhase::DragStart { start, pos, .. } => Some((index, start.origin, pos)),
                _ => None,
            });
            let Some((grip, origin, pos)) = started else { return };
            let sizing = WindowSize { grip, pressed: facts.inner.min + origin.to_vec2(), start: outer, at: outer };
            self.reports.push(change(outer, GesturePhase::Begin, grip));
            let at = resized(&sizing, facts.inner.min + pos.to_vec2());
            self.reports.push(change(at, GesturePhase::Update, grip));
            self.mode.sizing = Some(WindowSize { at, ..sizing });
            self.mode.held = Some(grip);
            return;
        };
        let ended = match phase(sizing.grip) {
            Some(PressPhase::Cancel { .. }) => Some(GesturePhase::Cancel),
            Some(PressPhase::Down { .. } | PressPhase::Drag { .. } | PressPhase::Held { .. } | PressPhase::DragStart { .. }) => None,
            // A release, or a press that ended in a pass the container did not run.
            _ => Some(GesturePhase::Commit),
        };
        if let Some(ended) = ended {
            let rect = if ended == GesturePhase::Cancel { sizing.start } else { sizing.at };
            self.reports.push(change(rect, ended, sizing.grip));
            self.mode.sizing = None;
            self.mode.held = None;
            return;
        }
        let moved = pointer::pass_pointer(&ctx)
            .events
            .iter()
            .rev()
            .find_map(|event| if let PointerEvent::Moved { at, .. } = event { Some(*at) } else { None });
        let Some(at) = moved.map(|at| resized(&sizing, facts.inner.min + at.to_vec2())).filter(|at| *at != sizing.at) else { return };
        sizing.at = at;
        self.reports.push(change(at, GesturePhase::Update, sizing.grip));
        self.mode.sizing = Some(sizing);
    }

    /// The move of an OS window whose app runs it (`Overlay::moved_by_app`): a drag of the move band keeps the
    /// pressed point under the pointer, and each step is reported through `placement_changed` as a layer's move is,
    /// from Begin to Commit or Cancel. Where the platform tracks the OS's moves ([`Caps::OS_MOVES_TRACKED`]) the OS
    /// moves the window, from the press: `StartMove` hands it the press event, which a later pass no longer has. The
    /// window is where the system pointer is less the pressed point, and the move is reported once the pointer has
    /// passed the click distance, so a click on the band moves and docks nothing; it ends with the release the window
    /// host adds once the system tells the button is up. Elsewhere the move starts past the click distance, the app
    /// moves the window to this pass's pointer, relative to where the window reports itself, and Escape puts it back.
    /// The drag's desktop pointer is the pressed point of the window where the move put it.
    fn window_move(&mut self, viewport: ViewportId, core: &mut CxCore<'_>) {
        let ctx = core.ctx.clone();
        let Some(phase) = core.results.get(&(MOVE_BAND, None)).map(|result| result.press.phase) else { return };
        let host = windows::WindowHost::of(&ctx);
        let Some(facts) = host.facts(viewport) else { return };
        let outer = facts.outer.unwrap_or(facts.inner);
        // Where the inner area lies in the outer rect.
        let inset = facts.inner.min - outer.min;
        let change = |rect: Rect, phase: GesturePhase| PlacementChange { rect, phase, pointer: None, kind: PlacementKind::Move, continued: false };
        let pointer_of = |moving: &WindowMove| moving.at.min + inset + moving.grab;
        let Some(mut moving) = self.mode.moving else {
            let tracked = windows::shell_of(&ctx, viewport).caps.contains(Caps::OS_MOVES_TRACKED);
            let mut moving = WindowMove { grab: Vec2::ZERO, start: outer, at: outer, tracked, begun: !tracked };
            match phase {
                PressPhase::Down { start, .. } if tracked => {
                    moving.grab = start.origin.to_vec2();
                    windows::act(&ctx, viewport, WindowAction::StartMove);
                }
                PressPhase::DragStart { start, pos, .. } if !tracked => {
                    moving.grab = start.origin.to_vec2();
                    self.reports.push(change(outer, GesturePhase::Begin));
                    // As every later step: from the system pointer where the platform tells it (see below).
                    moving.at = match windows::system_pointer(&ctx) {
                        Some(pointer) => Rect::from_min_size(pointer - moving.grab - inset, outer.size()),
                        None => outer.translate(pos - start.origin),
                    };
                    windows::act(&ctx, viewport, WindowAction::SetOuterPosition(moving.at.min));
                    self.reports.push(change(moving.at, GesturePhase::Update));
                }
                _ => return,
            }
            windows::note_moved_pointer(&ctx, viewport, Some(pointer_of(&moving)));
            self.mode.moving = Some(moving);
            return;
        };
        let ended = match phase {
            PressPhase::Cancel { .. } => Some(GesturePhase::Cancel),
            PressPhase::Down { .. } | PressPhase::Drag { .. } | PressPhase::Held { .. } | PressPhase::DragStart { .. } => None,
            // A release, or a press that ended in a pass the container did not run.
            _ => Some(GesturePhase::Commit),
        };
        if let Some(ended) = ended {
            if ended == GesturePhase::Cancel && !moving.tracked {
                windows::act(&ctx, viewport, WindowAction::SetOuterPosition(moving.start.min));
                moving.at = moving.start;
            } else if moving.begun
                && !moving.tracked
                && let Some(to) = windows::system_pointer(&ctx).map(|pointer| pointer - moving.grab - inset).filter(|to| *to != moving.at.min)
            {
                // The release lands the window where the pointer let go, past the last pass that moved it.
                windows::act(&ctx, viewport, WindowAction::SetOuterPosition(to));
                moving.at = Rect::from_min_size(to, moving.at.size());
            }
            if moving.begun {
                self.reports.push(change(moving.at, ended));
            }
            windows::note_moved_pointer(&ctx, viewport, None);
            self.mode.moving = None;
            return;
        }
        let to = if moving.tracked {
            // AppKit sends no pointer events while it moves the window: the next pass reads the system pointer again.
            crate::runtime::egui::repaint::request(&ctx);
            host.desktop_pointer().map(|pointer| pointer - moving.grab - inset)
        } else if let Some(pointer) = windows::system_pointer(&ctx) {
            // The system pointer is in desktop points and does not depend on where the window is, so the window lands
            // with the pressed point under it whatever moves the platform has applied by now. A pointer event is
            // relative to the window as it was when the system sent it; the window moves between passes, so a place
            // derived from an event lags further each pass and the window falls behind the pointer.
            Some(pointer - moving.grab - inset)
        } else {
            // A platform that tells no system pointer: a pointer move of this pass is relative to where the window was
            // when the system sent it, taken as where the window reports itself now.
            let moved = pointer::pass_pointer(&ctx)
                .events
                .iter()
                .rev()
                .find_map(|event| if let PointerEvent::Moved { at, .. } = event { Some(*at) } else { None });
            moved.map(|at| facts.inner.min + (at.to_vec2() - moving.grab) - inset)
        };
        let Some(to) = to.filter(|to| *to != moving.at.min) else { return };
        if !moving.begun {
            if to.distance(moving.start.min) <= crate::runtime::frame::slop(&ctx) {
                return;
            }
            moving.begun = true;
            self.reports.push(change(moving.start, GesturePhase::Begin));
        }
        moving.at = Rect::from_min_size(to, moving.at.size());
        if !moving.tracked {
            windows::act(&ctx, viewport, WindowAction::SetOuterPosition(to));
        }
        windows::note_moved_pointer(&ctx, viewport, Some(pointer_of(&moving)));
        self.reports.push(change(moving.at, GesturePhase::Update));
        self.mode.moving = Some(moving);
    }

    /// A viewport mount in an OS window reports the window's moves and resizes from its facts, through the
    /// `placement_changed` a layer reports its own through: one `Commit` per change, as the platform tells no gesture
    /// phase. The first facts are the window's placement, not a change; the steps of a move the app runs are that
    /// move's own ([`Self::window_move`]).
    pub(super) fn window_placed(&mut self, props: &ContainerProps, core: &mut CxCore<'_>) {
        if !matches!(props.mount, Mount::Viewport { .. }) {
            return;
        }
        let Some(viewport) = os_window(props, &core.ctx, core.id, &core.env) else { return };
        let Some(facts) = windows::WindowHost::of(&core.ctx).facts(viewport) else { return };
        let now = facts.outer.unwrap_or(facts.inner);
        let placed = &mut core.memory.shown::<WindowPlaced>(sub(sub(core.id, "body"), "window_placed")).0;
        let before = placed.replace(now);
        if self.mode.moving.is_some() || self.mode.sizing.is_some() {
            return;
        }
        let Some(before) = before.filter(|before| *before != now) else { return };
        let kind = resized_edge(before, now).map_or(PlacementKind::Move, PlacementKind::Size);
        // The size of a window its caller sizes is the caller's own: the window reports only its moves.
        if matches!(kind, PlacementKind::Size(_)) && overlay_of(props).is_some_and(|overlay| overlay.resize.by_caller) {
            return;
        }
        self.reports.push(PlacementChange { rect: now, phase: GesturePhase::Commit, pointer: None, kind, continued: false });
    }

    /// The move band and the grips (16.3). A layer's move and resize the layer; those of an OS window (`os_window`)
    /// stand by `WindowParts` and join the window's exclusive list, which the hit-test sink reads. A layer's parts never
    /// join it, or the sink would move the OS window from a dialog inside it, except the band of a layer that moves the
    /// window it is shown in (`Overlay::moves_window`): that band is a drag area of the window. `held` is the grip a
    /// drag holds (`WindowMode::held`), which stays declared until its release.
    pub(super) fn move_parts<C: Container>(props: &ContainerProps, regions: &Regions, rect: Rect, held: Option<usize>, pcx: &mut PartsCx<'_, C>) {
        let ctx = pcx.ui.ctx().clone();
        let reason = "the Move and Size commands of the window command set do the same";
        let os = os_window(props, &ctx, pcx.id, &pcx.env);
        let stands = match (&props.mount, os) {
            (_, Some(viewport)) => WindowParts::of(&ctx, viewport),
            (Mount::Layer(_) | Mount::Viewport { .. } | Mount::FullWindow, None) => WindowParts::LAYER,
            // A container of an embedded shell: no OS window to move or resize.
            (Mount::Window { .. } | Mount::Pane(_) | Mount::Inline, None) => WindowParts::NONE,
        };
        let exclusive = |hit: WindowHit| os.is_some().then_some(hit);
        let moves_window = matches!(&props.mount, Mount::Layer(overlay) | Mount::Viewport { fallback: overlay, .. } if overlay.moves_window);
        // A full-window container in an OS window of its own: its fallback layer covers the main window, so neither
        // moves nor resizes, but the window is one the user moves by its title row and sizes by its edges, as any
        // window. Without them a window the app frames had no band and no grips (photocraft#16905).
        let own_full_window = os.is_some() && matches!(&props.mount, Mount::Viewport { fallback, .. } if fallback.is_full_window());
        // A layer moves when its overlay says so; other mounts follow the header's band.
        let movable = match &props.mount {
            // A band that moves the window below follows the rule of the window's own band (a title bar's); where the
            // platform answers the hit test, the sink reads it from the exclusive list.
            _ if moves_window => WindowParts::of(&ctx, pass::current_viewport(&ctx)).band,
            _ if own_full_window => stands.band,
            Mount::Layer(overlay) | Mount::Viewport { fallback: overlay, .. } => overlay.movable && stands.band,
            _ => props.header.is_some_and(|header| header.band == crate::contract::DragBand::Move) && stands.band,
        };
        // A layer's band reaches its frame's edges; an OS window's stays in its header, clear of the edges the
        // platform hit-tests for resizing.
        let band = move_band(props, regions, rect, os.is_none() && !moves_window);
        if props.header.is_some() && movable && band.is_positive() {
            // The band lies under the line's own parts and children, like the reorder band, so they keep their presses.
            // On a window the OS frames (macOS `Inset`) it keeps the arrow, as an OS title bar does: the OS's resize
            // areas cover its ends and top edge, and a cursor set over them fights the OS's.
            let moved = os.or(moves_window.then(|| pass::current_viewport(&ctx)));
            let os_bar = moved.is_some_and(|viewport| windows::shell_of(&ctx, viewport).window.chrome == WindowChrome::Inset);
            let hand = pcx.env.get::<crate::contract::MoveBandCursor>() == crate::contract::MoveBandCursor::Hand;
            let mut part = pcx.part(MOVE_BAND, band).sense(craft_behavior::Sense::DRAG).pointer_only(reason);
            if !os_bar && hand {
                part = part.cursor(CursorIcon::Grab).drag_cursor(CursorIcon::Grabbing);
            }
            if let Some(hit) = exclusive(WindowHit::Move).or(moves_window.then_some(WindowHit::Move)) {
                part.exclusive(hit);
            }
            // A layer that continues a press begun elsewhere takes it before step 8 reads this pass's presses, so its
            // move begins in this pass from the window's own rect (`window_drag`). Only a layer can: the OS gives an
            // OS window no press that went down in another window.
            if let Some(grab) = overlay_of(props).and_then(|overlay| overlay.continue_move).filter(|_| os.is_none() && !moves_window) {
                crate::primitives::sensing::hand_over_press(&ctx, pcx.id, MOVE_BAND.into(), rect.min + grab);
            }
        }
        let edges = match &props.mount {
            _ if own_full_window => ResizeEdges::ALL,
            Mount::Layer(overlay) | Mount::Viewport { fallback: overlay, .. } => overlay.resize.edges,
            Mount::Window { resize } => resize.edges,
            _ => ResizeEdges::NONE,
        };
        if edges == ResizeEdges::NONE || !stands.grips {
            return;
        }
        // Every grip lies inside the frame, where a window has pixels and a layer's own rect takes the pointer: an edge
        // grip `handle` deep, a corner grip twice that along each of its edges, the edge grips between the corners.
        // They stack above the body (`over`), so a root in the body never covers them (8.3), and each takes exactly its
        // rect (`exact_hit`), as the platform's hit test does for an exclusive grip. The top corners of a layer that
        // keeps a band over its header (`top_band`) are only as tall as the band, so no grip lies over the header.
        let handle = grip_band(pcx.tokens);
        let corner = 2.0 * handle;
        let top = top_band(props, &ctx, pcx.id, pcx.tokens);
        let pinned = overlay_of(props).map_or(Pinned::NONE, |overlay| Pinned::of(overlay, &ctx, pcx.id, pcx.env.text_direction().is_rtl()));
        let caller_sizes = overlay_of(props).is_some_and(|overlay| overlay.resize.by_caller);
        let grip_name = props.resize_name.as_ref().map(|name| pcx.env.resolve(name).as_str().to_owned());
        for (index, (dx, dy, cursor)) in GRIPS.into_iter().enumerate() {
            if held != Some(index) && !grip_stands(edges, pinned, dx, dy) {
                continue;
            }
            let span = |edge: i8, low: f32, high: f32, deep: f32| match edge {
                -1 => (low, low + deep),
                1 => (high - deep, high),
                _ => (low + corner, high - corner),
            };
            let at_corner = dx != 0 && dy != 0;
            let deep = if at_corner { corner } else { handle };
            let tall = if at_corner && dy == -1 && top > 0.0 { top } else { deep };
            let (x0, x1) = span(dx, rect.min.x, rect.max.x, deep);
            let (y0, y1) = span(dy, rect.min.y, rect.max.y, tall);
            let level = if at_corner { HitLevel::Handle } else { HitLevel::Edge };
            let part = pcx
                .part(GRIP.at(index as u64), Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1)))
                .sense(craft_behavior::Sense::DRAG)
                .priority(HitPriority::new(level, 0))
                .cursor(cursor)
                .pointer_only(reason)
                .exact_hit()
                .over();
            let part = match &grip_name {
                Some(name) => part.node(crate::runtime::access::Role::SPLITTER, name.as_str()),
                None => part,
            };
            // The grips of a window its caller sizes are the app's (`window_resize`), never the platform's.
            if let Some(hit) = exclusive(WindowHit::Resize(edge_of(dx, dy))).filter(|_| !caller_sizes) {
                part.exclusive(hit);
            }
        }
    }

    /// Tells the platform whether the OS may move the container's OS window from where the pointer is (21.2
    /// `WindowDrag`; macOS `NSWindow.movable`): only where a press would go to the move band under the press-target
    /// rule (8.3), so a press on a control over the band never drags the window. Bands of one window combine: any band
    /// that would take the press allows the move. A window whose move the app runs ([`Self::window_move`]) writes
    /// nothing: its move starts from the band's press, and macOS keeps such a window movable, which AppKit needs to
    /// move it for the hand-off and to resize it from its edges. A pass with no band writes nothing; `AppShell` writes
    /// `false` for a pass with no title bar.
    pub(super) fn window_movable(&self, viewport: ViewportId, core: &CxCore<'_>) {
        let Some(band) = core.results.get(&(MOVE_BAND, None)) else { return };
        let ctx = &core.ctx;
        if windows::app_moves(ctx, viewport) {
            return;
        }
        let takes_press = band.hovered
            && pointer::with_hover(ctx, pass::current_viewport(ctx), |under, _| {
                under
                    .and_then(|under| under.press.click.or(under.press.drag))
                    .is_some_and(|(winner, root)| root == core.id && winner.part.is_some_and(|part| part.key == MOVE_BAND))
            });
        let this_pass = viewports::pass_number(ctx);
        let allowed = viewports::movable_record(ctx).is_some_and(|(pass, movable)| pass == this_pass && movable);
        viewports::write_movable(ctx, allowed || takes_press);
    }
}

/// The move band of a container whose header drags: an inline header's band spans the header's whole line (header,
/// body and end region), beneath the line's children, so every free point of the line drags; a header on a line of
/// its own is the band. With `to_edges` (a layer: a dialog, a floating group) a header on a line of its own drags
/// from the frame's top and side edges down to where the body starts, so the padding around the title moves the
/// layer as a title bar does, not only the title's line (photocraft#17112).
fn move_band(props: &ContainerProps, regions: &Regions, rect: Rect, to_edges: bool) -> Rect {
    if !props.header.is_some_and(|header| header.inline) {
        if !to_edges {
            return regions.header;
        }
        let bottom = regions.header.max.y.max(regions.body.min.y.min(rect.max.y));
        return Rect::from_min_max(rect.min, Pos2::new(rect.max.x, bottom)).intersect(rect);
    }
    let line = regions.header.union(regions.body);
    let line = if props.end.is_some() { line.union(regions.end) } else { line };
    line.intersect(rect)
}
