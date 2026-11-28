//! The drag session (8.4): one per context. It holds the payload, the preview, the held modifiers, the operation and the
//! mark of the drop target that would take the payload, as data. The drop-target registrations of a pass resolve in the
//! next pass, like the press arena's shapes. The ghost layer paints from this data (`contract::ghost`).

use crate::runtime::egui::areas::{self, Layer};
use crate::runtime::egui::{files, pass, viewports};
use crate::runtime::env::{Env, EnvView, Label};
use crate::runtime::frame::PerViewport;
use crate::runtime::overlay::{self, LayerKeys};
use crate::runtime::pass_state;
use crate::runtime::windows::{self, WindowHost};
use crate::runtime::{cursor, frame, pointer};
use crate::style::UiTokens;
use craft_behavior::keys::drop_operation;
use craft_behavior::{Buttons, CancelReason, CopyKeys, Cursor, CursorIcon, CursorPriority, DropOperation, PointerButton, PressCancel};
use craft_keymap::Mods;
use egui::{Context, Id, Pos2, Rect, Vec2, ViewportId};
use std::any::{Any, TypeId};
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::Arc;

/// What the ghost layer paints beside the pointer during a drag.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum DragPreview {
    /// Nothing.
    #[default]
    None,
    /// A badge with a short text ("3 files").
    Badge(Label),
    /// Nothing beside the pointer: the payload's name shows inside the area of a region mark, as a panel or group
    /// being moved names the place it would take. With no mark, nothing shows.
    Named(Label),
    /// An outline of `size` centred on the pointer, with an optional label inside its top corner: the ghost of a part
    /// dragged out of a row (a thumbnail).
    #[non_exhaustive]
    Frame {
        /// The size of the frame, in points.
        size: Vec2,
        /// A name shown inside the frame.
        label: Option<Label>,
    },
}

impl DragPreview {
    /// A frame of `size` with an optional `label`.
    pub fn frame(size: Vec2, label: Option<Label>) -> Self {
        DragPreview::Frame { size, label }
    }
}

/// Where a drop would land, as geometry in the target's egui layer space.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum DropMark {
    /// An insertion line.
    #[non_exhaustive]
    Line {
        /// One end.
        from: Pos2,
        /// The other end.
        to: Pos2,
    },
    /// The payload goes into this rect.
    Into(Rect),
    /// The payload will occupy this area once dropped, such as the rect a docked panel takes beside a group.
    #[non_exhaustive]
    Region {
        /// The rect the payload takes.
        area: Rect,
        /// Where in it the payload goes in, such as the tab position of a panel joining a group.
        line: Option<[Pos2; 2]>,
    },
}

crate::stays!(Arc<crate::style::UiTokens>, DropOperation, TargetReach);

impl crate::runtime::Shift for DropMark {
    fn shifted(&self, by: &egui::RecordShift) -> Self {
        self.mapped(|point| by.pos(point))
    }

    fn snapped(&self, to: &Self, within: f32) -> Self {
        let pos = |value: Pos2, to: Pos2| crate::runtime::Shift::snapped(&value, &to, within);
        let rect = |value: Rect, to: Rect| Rect::from_min_max(pos(value.min, to.min), pos(value.max, to.max));
        match (self, to) {
            (DropMark::Line { from, to: end }, DropMark::Line { from: to_from, to: to_end }) => {
                DropMark::Line { from: pos(*from, *to_from), to: pos(*end, *to_end) }
            }
            (DropMark::Into(target), DropMark::Into(to_target)) => DropMark::Into(rect(*target, *to_target)),
            (DropMark::Region { area, line }, DropMark::Region { area: to_area, line: to_line }) => DropMark::Region {
                area: rect(*area, *to_area),
                line: match (line, to_line) {
                    (Some([a, b]), Some([to_a, to_b])) => Some([pos(*a, *to_a), pos(*b, *to_b)]),
                    _ => *line,
                },
            },
            _ => *self,
        }
    }
}

impl DropMark {
    /// An insertion line from `from` to `to`.
    pub const fn line(from: Pos2, to: Pos2) -> Self {
        DropMark::Line { from, to }
    }

    /// The payload will occupy `area`.
    pub const fn region(area: Rect) -> Self {
        DropMark::Region { area, line: None }
    }

    /// The payload will occupy `area` and goes in at the line from `from` to `to`.
    pub const fn region_at(area: Rect, from: Pos2, to: Pos2) -> Self {
        DropMark::Region { area, line: Some([from, to]) }
    }

    /// The mark with every point passed through `map`.
    pub(crate) fn mapped(self, map: impl Fn(Pos2) -> Pos2) -> Self {
        let rect = |rect: Rect| Rect::from_two_pos(map(rect.min), map(rect.max));
        match self {
            DropMark::Line { from, to } => DropMark::Line { from: map(from), to: map(to) },
            DropMark::Into(target) => DropMark::Into(rect(target)),
            DropMark::Region { area, line } => DropMark::Region { area: rect(area), line: line.map(|line| line.map(&map)) },
        }
    }
}

/// A file the OS drags over the window or drops on it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct OsFile {
    /// The file's name; empty while the OS tells only a hovered file's type.
    pub name: String,
    /// Where the file is, on platforms that give a path.
    pub path: Option<PathBuf>,
    /// The file's media type, as far as the OS knows it.
    pub mime: String,
    /// The file's content, on platforms that give no path; read only for a dropped file.
    pub bytes: Option<Arc<[u8]>>,
}

/// The payload of a drag that comes from the OS: files held over the window, then the files dropped. Targets take it
/// like any payload, with `Pointer::drop_target` or `Pointer::window_drop_target`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct OsFiles {
    /// The files, in the order the OS lists them.
    pub files: Vec<OsFile>,
}

/// What a component registers with `Pointer::drop_target`.
pub trait DropTarget<P: ?Sized> {
    /// What a drop on this target carries to the component's caller.
    type Drop;

    /// `None` rejects. `at` is the pointer in the component's egui layer space, so a target picks its sub-area (a port,
    /// a split side) from it; `op` is the operation the held modifiers pick, which a target may refuse.
    fn accept(&self, payload: &P, at: Pos2, op: DropOperation) -> Option<Self::Drop>;

    /// Where `drop` would land, a value the ghost layer paints.
    fn mark(&self, drop: &Self::Drop) -> DropMark;

    /// Whether the target takes OS file drops (`OsFiles`) while a modal layer above it holds input, as an app whose
    /// alerts let files through (VectorCraft placed files dropped under its alert). By default a modal refuses them.
    fn under_modal(&self) -> bool {
        false
    }

    /// The held key that makes a drop on this target a copy: the `op` that [`accept`](Self::accept) gets, and the
    /// pointer over the target, follow it.
    fn copy_keys(&self) -> CopyKeys {
        CopyKeys::Alt
    }
}

/// What a component registers with `Pointer::window_drop_target`: a target over the whole window, optionally waiting
/// for a pointer position before it accepts.
pub trait WindowDropTarget<P> {
    /// What a drop on this target carries to the component's caller.
    type Drop;

    /// Whether this target waits for a pointer position before taking a drop.
    fn requires_pointer(&self) -> bool {
        false
    }

    /// `None` rejects. `op` is the operation the held modifiers pick, which a target may refuse.
    fn accept(&self, payload: &P, op: DropOperation) -> Option<Self::Drop>;

    /// Where `drop` would land, a value the ghost layer paints.
    fn mark(&self, drop: &Self::Drop) -> DropMark;

    /// Whether the target takes OS file drops (`OsFiles`) while a modal layer holds input, as
    /// [`DropTarget::under_modal`] says.
    fn under_modal(&self) -> bool {
        false
    }
}

/// Where a drop target lies: a rect on an egui layer, in that layer's space. A component's own target lies over its
/// visible rect on its layer; a component that takes drops over an area it does not draw (a preset browser's canvas)
/// names the area with [`DropZone::new`] and registers with [`zone_target`].
#[derive(Clone, Copy, Debug, PartialEq, crate::contract::MemoProps)]
pub struct DropZone {
    layer: Layer,
    rect: Rect,
}

impl DropZone {
    /// The zone `rect` on `layer` (the `layer_id` of the `Ui` that draws the area), in that layer's space.
    pub const fn new(layer: crate::runtime::egui::areas::Layer, rect: Rect) -> Self {
        DropZone { layer, rect }
    }

    /// The zone's rect, in its layer's space.
    pub const fn rect(&self) -> Rect {
        self.rect
    }
}

/// The window a drag moves along with the pointer when its source is a container that opens one. Targets never see
/// the pointer inside the window, so the targets under the moving unit take the drop; the container's layer is looked
/// through the same way ([`moving_layer`]).
#[derive(Clone, Copy, Default)]
struct Moving {
    viewport: Option<ViewportId>,
}

struct Session {
    source: Id,
    /// The unit the source moves.
    moving: Moving,
    /// The viewport whose pass started the session and the button whose press it follows: only that viewport's end
    /// hook reads the button, so another window repainting mid-drag does not end the session. Its release ends the
    /// session, and its window loss cancels it.
    viewport: ViewportId,
    button: PointerButton,
    payload: Arc<dyn Any + Send + Sync>,
    /// More payloads of the same drag, one per type, which targets of their types take: a collection's row drag maps
    /// to them.
    also: Vec<Arc<dyn Any + Send + Sync>>,
    preview: DragPreview,
    /// The env of the region the drag started in, which its preview paints with; `None` for files from the OS.
    env: Option<Env>,
    /// The mark of the target that would take a drop now, in screen space of the viewport that registered it.
    mark: Option<ShownMark>,
    /// The root frame in which the source viewport saw the release while a target in another window held the
    /// drop: the session lives until that window's pass takes it.
    released: Option<u64>,
    /// A target took the drop.
    taken: bool,
    /// The session carries files from the OS: no button holds it, and the OS ends it by dropping the files or
    /// taking them out of the window.
    os: bool,
    /// The root frame in which the OS dropped the files.
    dropped: Option<u64>,
    /// A file drop has arrived but no pointer position is available yet.
    waiting_for_pointer: bool,
    /// Where the OS dropped the files: the pass that takes a drop after the frame of its arrival places it there, not
    /// at a pointer that has moved on (files hovering again for a drag of their own).
    dropped_at: Option<Pos2>,
    /// Files the OS holds over the window in the pass that takes this session's drop: a drag of their own, which
    /// starts when the drop ends this session (`end_pass`).
    hovering_next: Vec<OsFile>,
    /// The root frame the session started in, the only one in which [`amend`] changes its payload.
    began: u64,
}

/// The mark of the target that would take a drop, with the viewport that registered it and the tokens of the region
/// the target lies in, which the mark paints with.
#[derive(Clone)]
struct ShownMark {
    viewport: ViewportId,
    mark: DropMark,
    tokens: Arc<UiTokens>,
}

/// A drop a target took: the source's `drag_ended` reads it in the next frame.
struct Ended {
    source: Id,
    payload: TypeId,
    op: DropOperation,
    frame: u64,
}

#[derive(Default)]
pub(crate) struct Board {
    session: Option<Session>,
    ended: Option<Ended>,
    /// The sessions started on this context, for tests.
    started: u64,
}

/// A target that accepted in one pass.
struct Registration {
    owner: Id,
    ordinal: u32,
    layer: Layer,
    /// The overlay layer whose body registered the target; `None` for the window's own content.
    scope: Option<Id>,
    mark: DropMark,
    /// What a drop on the target does with the keys held now; the pointer shows it while the target wins.
    op: DropOperation,
    /// The tokens of the target's region.
    tokens: Arc<UiTokens>,
    reach: TargetReach,
}

/// Where a drop target takes drops beyond its own mark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TargetReach {
    /// The target covers the whole window: any target over a component beats it.
    pub(crate) window: bool,
    /// The target takes OS files under a modal layer (`DropTarget::under_modal`).
    pub(crate) under_modal: bool,
}

/// A viewport's registrations: this pass's, and the target that won last pass.
#[derive(Default)]
pub(crate) struct Targets {
    this: Vec<Registration>,
    winner: Option<(Id, u32)>,
    /// The winner is a target over a component, not the whole window.
    component: bool,
    winner_window: bool,
    pointer: Option<Pos2>,
}

fn board<R>(ctx: &Context, f: impl FnOnce(&mut Board) -> R) -> R {
    pass_state::with::<Board, _>(ctx, f)
}

/// Starts a session for `source` with `payload`; `env` is the source's, which the preview paints with. A running
/// session stays: one per context. A container shown in an OS window of its own runs in its parent's pass, but the
/// press that starts its drag is its window's: the session follows that window's pointer.
#[doc(hidden)]
pub fn start(
    ctx: &Context,
    source: Id,
    env: &EnvView<'_>,
    payload: Arc<dyn Any + Send + Sync>,
    also: Vec<Arc<dyn Any + Send + Sync>>,
    preview: DragPreview,
) {
    let viewport = WindowHost::of(ctx).viewport_of(source).unwrap_or_else(|| pass::current_viewport(ctx));
    let began = pass::root_frame_nr(ctx);
    board(ctx, |board| {
        if board.session.is_none() {
            let buttons = pointer::buttons_of(ctx, viewport);
            let button = [PointerButton::Primary, PointerButton::Secondary, PointerButton::Middle, PointerButton::Extra1, PointerButton::Extra2]
                .into_iter()
                .find(|button| buttons.contains(Buttons::of(*button)))
                .unwrap_or(PointerButton::Primary);
            board.session = Some(Session {
                source,
                moving: Moving { viewport: Some(viewports::viewport_id(source)) },
                viewport,
                button,
                payload,
                also,
                preview,
                env: Some(env.to_env()),
                mark: None,
                released: None,
                taken: false,
                os: false,
                dropped: None,
                waiting_for_pointer: false,
                dropped_at: None,
                hovering_next: Vec::new(),
                began,
            });
            board.ended = None;
            board.started += 1;
        }
    });
    crate::runtime::egui::repaint::request(ctx);
}

fn operation(ctx: &Context, mods: Mods, copy: CopyKeys) -> DropOperation {
    drop_operation(windows::os_of(ctx), mods, copy)
}

/// Changes the running session's payload of type `P` (or its further payload of that type) with `change`, in the frame
/// the session started only: a host completes what a drag its component started carries with what only the host knows
/// (a dock measures the panel a tab drag floats). Returns whether a payload changed; from the next frame on, the
/// payload is fixed and nothing changes.
pub fn amend<P: Clone + Send + Sync + 'static>(ctx: &Context, change: impl FnOnce(&mut P)) -> bool {
    let frame = pass::root_frame_nr(ctx);
    board(ctx, |board| {
        let Some(session) = board.session.as_mut().filter(|session| session.began == frame) else { return false };
        let slot = if session.payload.is::<P>() { Some(&mut session.payload) } else { session.also.iter_mut().find(|also| also.is::<P>()) };
        let Some(slot) = slot else { return false };
        let Some(mut payload) = slot.downcast_ref::<P>().cloned() else { return false };
        change(&mut payload);
        *slot = Arc::new(payload);
        true
    })
}

/// The active drag payload of type `P`, from any viewport. For code without a `Cx`.
pub fn active<P: Send + Sync + 'static>(ctx: &Context) -> Option<Arc<P>> {
    crate::runtime::replay::note_input(ctx, crate::runtime::inputs::Input::Drag);
    active_of(ctx, TypeId::of::<P>())?.downcast::<P>().ok()
}

/// The active drag payload whose type is `payload`: the session's own payload, else the one of its `also`. For a
/// target that picks the payload types it takes at run time.
pub(crate) fn active_of(ctx: &Context, payload: TypeId) -> Option<Arc<dyn Any + Send + Sync>> {
    crate::runtime::replay::note_input(ctx, crate::runtime::inputs::Input::Drag);
    let (own, also) = board(ctx, |board| board.session.as_ref().map(|session| (session.payload.clone(), session.also.clone())))?;
    if Any::type_id(&*own) == payload {
        return Some(own);
    }
    also.into_iter().find(|also| Any::type_id(&**also) == payload)
}

/// How many sessions have started on this context.
#[doc(hidden)]
pub fn started(ctx: &Context) -> u64 {
    crate::runtime::replay::note_input(ctx, crate::runtime::inputs::Input::Drag);
    board(ctx, |board| board.started)
}

/// What a drag reader sees of the session, as the tracked input `Drag` compares it: the sessions started, and the
/// payloads of the running one.
pub(crate) fn state(ctx: &Context) -> (u64, Option<(usize, Vec<usize>)>) {
    board(ctx, |board| {
        let payloads = board.session.as_ref().map(|session| {
            (Arc::as_ptr(&session.payload) as *const () as usize, session.also.iter().map(|also| Arc::as_ptr(also) as *const () as usize).collect())
        });
        (board.started, payloads)
    })
}

/// The mark of the target that would take a drop now, in screen space: `None` while no target accepts the payload.
pub fn mark(ctx: &Context) -> Option<DropMark> {
    board(ctx, |board| board.session.as_ref().and_then(|session| session.mark.as_ref().map(|shown| shown.mark)))
}

/// Whether the running session carries files from the OS.
fn carries_os_files(ctx: &Context) -> bool {
    board(ctx, |board| board.session.as_ref().is_some_and(|session| session.os))
}

/// Whether a session runs.
pub(crate) fn running(ctx: &Context) -> bool {
    board(ctx, |board| board.session.is_some())
}

/// Ends the session without a drop; the source's press ends with it, so nobody gets a drop.
pub fn cancel(ctx: &Context, reason: CancelReason) {
    if board(ctx, |board| board.session.take()).is_none() {
        return;
    }
    let press = match reason {
        CancelReason::Escape => PressCancel::Escape,
        CancelReason::Removed => PressCancel::Removed,
        _ => PressCancel::FocusLost,
    };
    pointer::cancel_press(ctx, press);
    frame::note_cancelled_press(ctx);
}

/// Ends the session `source` started without a drop, for a host that ends `source`'s press
/// (`primitives::cancel_press_of`); a session another source started runs on.
pub(crate) fn cancel_of(ctx: &Context, source: Id) {
    board(ctx, |board| {
        if board.session.as_ref().is_some_and(|session| session.source == source) {
            board.session = None;
        }
    });
}

/// The begin hook: the window losing focus ends the session.
pub(crate) fn begin(ctx: &Context) {
    if source_viewport(ctx) == Some(pass::current_viewport(ctx)) && pointer::with_pass_raw(ctx, |pointer| pointer.focus_lost) {
        cancel(ctx, CancelReason::Blur);
    }
    os_files(ctx);
}

/// The session of files the OS drags over the window (8.4): files held over the window start one, the dropped files
/// replace its payload in the frame of the drop, and files that leave without a drop end it. The OS sends the files
/// only to the window under the pointer, so the session runs in that window: a window the files move to takes it, and
/// only a pass of its own window, which no longer sees them, ends it. A drop whose files are still being read
/// (`Seen::reading`) keeps the session, and its targets, until the pass that receives the files: that pass is the
/// drop's.
fn os_files(ctx: &Context) {
    let seen = files::seen(ctx);
    let frame = pass::root_frame_nr(ctx);
    let viewport = pass::current_viewport(ctx);
    let hovering = !seen.hovered.is_empty();
    // A session whose window closed has no pass left to end it.
    let closed = source_viewport(ctx).is_some_and(|window| !pass::live_viewports(ctx).contains(&window));
    let position = os_files_pointer(ctx, viewport, pointer::frame_raw(ctx).pos);
    let positioned = position.is_some();
    // A window target can take the drop immediately only if the pointer has not moved since resolution.
    let window_winner = PerViewport::<Targets>::with(ctx, viewport, |targets| targets.winner_window && targets.pointer == position);
    let mut repaint = false;
    board(ctx, |board| match &mut board.session {
        None if hovering || !seen.dropped.is_empty() => {
            let has_drop = !seen.dropped.is_empty();
            if has_drop {
                repaint = true;
            }
            let mut session = os_session(if has_drop { seen.dropped } else { seen.hovered }, viewport, frame);
            session.dropped = (has_drop && positioned).then_some(frame.saturating_add(1));
            session.dropped_at = position.filter(|_| has_drop);
            session.waiting_for_pointer = has_drop && !positioned;
            board.session = Some(session);
            board.ended = None;
            board.started += 1;
        }
        Some(session) if session.os => {
            if !seen.dropped.is_empty() {
                let hovered = session.payload.downcast_ref::<OsFiles>();
                let files = seen.dropped.into_iter().map(|file| with_hovered_type(file, hovered)).collect();
                session.payload = Arc::new(OsFiles { files });
                session.viewport = viewport;
                session.dropped_at = position;
                if window_winner {
                    session.dropped = Some(frame);
                } else if positioned {
                    // Resolve targets at the drop position before any previously winning window target can take it.
                    session.dropped = Some(frame.saturating_add(1));
                    repaint = true;
                } else {
                    session.waiting_for_pointer = true;
                }
            } else if session.waiting_for_pointer && (positioned || window_winner) {
                // Targets register this pass and take the drop in the next, after their winner is resolved.
                session.waiting_for_pointer = false;
                session.dropped = Some(frame.saturating_add(1));
                session.dropped_at = position;
                repaint = true;
            } else if hovering {
                session.viewport = viewport;
                if session.dropped == Some(frame) {
                    session.hovering_next = seen.hovered;
                }
            } else if (session.viewport == viewport || closed)
                && !seen.reading
                && !session.waiting_for_pointer
                && session.dropped.is_none_or(|drop| drop < frame)
            {
                board.session = None;
            }
        }
        _ => {}
    });
    // The OS sends the window no pointer events while it drags files over it (winit 0.30), so nothing else asks for
    // the passes that follow the pointer to the target under it.
    if hovering || repaint {
        crate::runtime::egui::repaint::request(ctx);
    }
}

/// A session of `files` the OS holds over `viewport`, started in root frame `frame`.
fn os_session(files: Vec<OsFile>, viewport: ViewportId, frame: u64) -> Session {
    Session {
        source: Id::NULL.with("craft_ui::os_files"),
        moving: Moving::default(),
        viewport,
        button: PointerButton::Primary,
        payload: Arc::new(OsFiles { files }),
        also: Vec::new(),
        preview: DragPreview::None,
        env: None,
        mark: None,
        released: None,
        taken: false,
        os: true,
        dropped: None,
        waiting_for_pointer: false,
        dropped_at: None,
        hovering_next: Vec::new(),
        began: frame,
    }
}

/// `file` with the media type the OS gave the hovered file at the same path, which egui does not repeat on a drop.
fn with_hovered_type(mut file: OsFile, hovered: Option<&OsFiles>) -> OsFile {
    let held = hovered.and_then(|hovered| hovered.files.iter().find(|held| held.path.is_some() && held.path == file.path));
    if let Some(held) = held {
        file.mime.clone_from(&held.mime);
    }
    file
}

/// The viewport the running session started in.
fn source_viewport(ctx: &Context) -> Option<ViewportId> {
    board(ctx, |board| board.session.as_ref().map(|session| session.viewport))
}

/// Whether the running session started in a window other than `viewport`. That window holds the pointer while its
/// button is down, so `viewport` gets no pointer events of its own and still holds the place it saw the pointer last.
pub(crate) fn runs_elsewhere(ctx: &Context, viewport: ViewportId) -> bool {
    source_viewport(ctx).is_some_and(|source| source != viewport)
}

/// The pointer in `viewport` for a drag that started in another window: the desktop pointer mapped into this
/// window, when it lies inside. `None` without a session, in the source window, or outside this window.
#[doc(hidden)]
pub fn cross_window_pointer(ctx: &Context, viewport: ViewportId) -> Option<Pos2> {
    runs_elsewhere(ctx, viewport).then(|| WindowHost::of(ctx).pointer_in(viewport)).flatten()
}

/// The pointer of the running session in screen space, whatever its payload: the pointer `dragging_of` pairs with a
/// payload, where the button went up in the pass of its release (`PointerFrame::pick_pos`); for an OS drop, where the
/// files were dropped in the pass that takes it. `None` without a session or a pointer.
pub(crate) fn session_pointer(ctx: &Context) -> Option<Pos2> {
    let frame = pass::root_frame_nr(ctx);
    let (os, dropped_at) =
        board(ctx, |board| board.session.as_ref().map(|session| (session.os, session.dropped_at.filter(|_| session.dropped == Some(frame)))))?;
    let own = pointer::frame_raw(ctx).pick_pos;
    if os { dropped_at.or_else(|| os_files_pointer(ctx, pass::current_viewport(ctx), own)) } else { own }
}

/// The pointer of OS files over `viewport`: the system pointer where the platform tells one, since the OS sends the
/// window no pointer events while it drags files over it; else `own`, the pass's own pointer. A drag's noted source
/// pointer (`windows::note_source_pointer`) is from the pass before, for the other windows, so it never stands in for
/// the pass's own.
fn os_files_pointer(ctx: &Context, viewport: ViewportId, own: Option<Pos2>) -> Option<Pos2> {
    if windows::system_pointer(ctx).is_some() { WindowHost::of(ctx).pointer_in(viewport) } else { own }
}

/// The running session's payload whose type is `payload` and the pointer in screen space. For files the OS drags, the
/// pointer is the platform's desktop pointer in this window when it tells one (outside this window means no
/// pointer), else the frame's: the OS sends the window no pointer events while it drags files over it (winit 0.30).
pub(crate) fn dragging_of(ctx: &Context, payload: TypeId) -> Option<(Arc<dyn Any + Send + Sync>, Pos2)> {
    let payload = active_of(ctx, payload)?;
    Some((payload, session_pointer(ctx)?))
}

/// The layer the running session's source moves with the pointer when the source is a container, which hit tests for
/// drop targets look through. Read from the layers shown last frame at each hit test, not when the session starts: a
/// container shown first in the pass its drag starts (a window floated to continue a press) has its layer recorded
/// only after its body ran (`overlay::record`).
pub(crate) fn moving_layer(ctx: &Context) -> Option<Layer> {
    let source = board(ctx, |board| board.session.as_ref().map(|session| session.source))?;
    overlay::entry(ctx, source).map(|entry| entry.layer)
}

/// Whether the running session's source is the window of the running viewport: its targets take no drop of the
/// window's own move.
pub(crate) fn moves_viewport(ctx: &Context) -> bool {
    let viewport = pass::current_viewport(ctx);
    board(ctx, |board| board.session.as_ref().is_some_and(|session| session.moving.viewport == Some(viewport)))
}

/// Whether the running session's source is the window of `viewport`.
#[doc(hidden)]
pub fn moves(ctx: &Context, viewport: ViewportId) -> bool {
    board(ctx, |board| board.session.as_ref().is_some_and(|session| session.moving.viewport == Some(viewport)))
}

/// Whether `source` started the running session, which after its release lives until a target in another window has
/// taken the drop.
#[doc(hidden)]
pub fn sourced_by(ctx: &Context, source: Id) -> bool {
    board(ctx, |board| board.session.as_ref().is_some_and(|session| session.source == source))
}

/// Starts a session whose source is the container `source`, as the title row of a dock window does, with no payload
/// of its own. For trials.
#[cfg(feature = "testing")]
pub(crate) fn begin_window_drag(ctx: &Context, source: Id) {
    start(ctx, source, &Env::of_root(ctx).view(), Arc::new(()), Vec::new(), DragPreview::None);
}

/// Sets the mark of the target that would take a drop, as one in another window would. For trials.
#[cfg(feature = "testing")]
pub(crate) fn show_mark(ctx: &Context, mark: Option<DropMark>) {
    board(ctx, |board| {
        if let Some(session) = &mut board.session {
            let tokens = Env::of_root(ctx).tokens.clone();
            session.mark = mark.map(|mark| ShownMark { viewport: ViewportId::from_hash_of("testing.mark"), mark, tokens });
        }
    });
}

/// Records that `owner`'s `ordinal`th target on `layer` accepts at the pointer a drop doing `op`, with its mark in
/// layer space, the tokens of the target's region and its `reach`.
pub(crate) fn register(ctx: &Context, owner: Id, layer: Layer, mark: DropMark, op: DropOperation, tokens: Arc<UiTokens>, reach: TargetReach) {
    crate::runtime::replay::journal(
        ctx,
        || (owner, layer, mark, op, tokens.clone(), reach),
        |ctx, (owner, layer, mark, op, tokens, reach)| register(ctx, *owner, *layer, *mark, *op, tokens.clone(), *reach),
    );
    let mark = match areas::layer_map(ctx, layer) {
        Some(map) => mark.mapped(|point| map.to_screen(point)),
        None => mark,
    };
    let scope = overlay::drawing_parent(ctx);
    PerViewport::<Targets>::with(ctx, pass::current_viewport(ctx), |targets| {
        let ordinal = targets.this.iter().filter(|registration| registration.owner == owner).count() as u32;
        targets.this.push(Registration { owner, ordinal, layer, scope, mark, op, tokens, reach });
    });
}

/// Whether `owner`'s `ordinal`th target was the one that would take a drop at the pointer in the last pass.
#[doc(hidden)]
pub fn winning(ctx: &Context, owner: Id, ordinal: u32) -> bool {
    PerViewport::<Targets>::with(ctx, pass::current_viewport(ctx), |targets| targets.winner == Some((owner, ordinal)))
}

/// The ordinal the next registration of `owner` gets in this pass.
#[doc(hidden)]
pub fn next_ordinal(ctx: &Context, owner: Id) -> u32 {
    PerViewport::<Targets>::with(ctx, pass::current_viewport(ctx), |targets| {
        targets.this.iter().filter(|registration| registration.owner == owner).count() as u32
    })
}

/// The operation the pass's pick modifiers pick now over a target that copies with `copy`: the release's in the drop
/// pass, so a drop's operation and its modifiers agree.
pub(crate) fn current_operation(ctx: &Context, copy: CopyKeys) -> DropOperation {
    operation(ctx, pointer::frame_raw(ctx).pick_mods, copy)
}

/// Whether the session's button was released in this pass.
fn released(ctx: &Context) -> bool {
    let viewport = pass::current_viewport(ctx);
    let frame = pass::root_frame_nr(ctx);
    let (os, button) =
        board(ctx, |board| board.session.as_ref().map(|session| (session.os, session.button))).unwrap_or((false, PointerButton::Primary));
    if os {
        return board(ctx, |board| board.session.as_ref().is_some_and(|session| session.dropped == Some(frame)));
    }
    pointer::frame_raw(ctx).release == Some(button)
        || board(ctx, |board| board.session.as_ref().is_some_and(|session| session.released.is_some() && session.viewport != viewport))
}

/// Takes the drop of the payload whose type is `payload` for `owner`'s `ordinal`th target when it won last pass and the
/// session's button was released in this one: the drop ends the session's chance of another target taking it, and the
/// source reads `op` in the next frame.
pub(crate) fn take_drop(ctx: &Context, owner: Id, ordinal: u32, op: DropOperation, payload: TypeId) -> bool {
    let won = PerViewport::<Targets>::with(ctx, pass::current_viewport(ctx), |targets| targets.winner == Some((owner, ordinal)));
    if !won || !released(ctx) {
        return false;
    }
    let frame = pass::root_frame_nr(ctx);
    board(ctx, |board| {
        let Some(session) = board.session.as_mut() else { return false };
        if session.taken || board.ended.as_ref().is_some_and(|ended| ended.frame == frame) {
            return false;
        }
        session.taken = true;
        board.ended = Some(Ended { source: session.source, payload, op, frame });
        true
    })
}

/// `target` of `owner` over `zone`, for the payload whose type is `payload_type`: it accepts only where the pointer
/// lies in the zone and no layer lies over the zone's layer there (the layer a dragged container moves is not over
/// anything), and returns the drop in the pass of the release when it won the pass before. Its mark paints with
/// `tokens`, the target's region's.
pub(crate) fn target_of<T: DropTarget<dyn Any + Send + Sync>>(
    ctx: &Context,
    owner: Id,
    zone: DropZone,
    tokens: Arc<UiTokens>,
    payload_type: TypeId,
    target: &T,
) -> Option<T::Drop> {
    let (payload, screen) = dragging_of(ctx, payload_type)?;
    let at = areas::layer_map(ctx, zone.layer).map_or(screen, |map| map.to_layer(screen));
    let top = if carries_os_files(ctx) && target.under_modal() {
        // A target that takes OS files under a modal sees through the modal's scrim, not through its body.
        areas::layer_at_under_modal(ctx, screen, overlay::top_modal_in(ctx, pass::current_viewport(ctx)).and_then(|modal| modal.cover))
    } else {
        match moving_layer(ctx) {
            Some(moving) => areas::layer_at_without(ctx, screen, moving),
            None => areas::layer_at(ctx, screen),
        }
    };
    if moves_viewport(ctx) || !zone.rect.contains(at) || top != Some(zone.layer) {
        return None;
    }
    let op = current_operation(ctx, target.copy_keys());
    let drop = target.accept(&*payload, at, op)?;
    let ordinal = next_ordinal(ctx, owner);
    register(ctx, owner, zone.layer, target.mark(&drop), op, tokens, TargetReach { window: false, under_modal: target.under_modal() });
    take_drop(ctx, owner, ordinal, op, payload_type).then_some(drop)
}

/// `Pointer::drop_target` over a zone the caller names instead of its own visible rect: the drop of a payload of type
/// `P` released over `zone`, with the same layer order, operation and mark as a component's own target. `owner` is the
/// caller's stable id; it registers in every pass a drag may reach the zone. The mark paints in the tokens of `ui`, the
/// `Ui` that draws the zone.
pub fn zone_target<P: Send + Sync + 'static, T: DropTarget<P>>(ui: &egui::Ui, owner: Id, zone: DropZone, target: &T) -> Option<T::Drop> {
    target_of(ui.ctx(), owner, zone, Env::of(ui).tokens.clone(), TypeId::of::<P>(), &Typed::new(target))
}

/// A target of payload type `P` seen through the payload's erased type.
pub(crate) struct Typed<'t, P, T>(&'t T, PhantomData<fn(&P)>);

impl<'t, P, T> Typed<'t, P, T> {
    pub(crate) fn new(target: &'t T) -> Self {
        Typed(target, PhantomData)
    }
}

impl<P: 'static, T: DropTarget<P>> DropTarget<dyn Any + Send + Sync> for Typed<'_, P, T> {
    type Drop = T::Drop;

    fn accept(&self, payload: &(dyn Any + Send + Sync), at: Pos2, op: DropOperation) -> Option<T::Drop> {
        self.0.accept(payload.downcast_ref::<P>()?, at, op)
    }

    fn mark(&self, drop: &T::Drop) -> DropMark {
        self.0.mark(drop)
    }

    fn under_modal(&self) -> bool {
        self.0.under_modal()
    }

    fn copy_keys(&self) -> CopyKeys {
        self.0.copy_keys()
    }
}

/// The operation of the drop of `source`'s payload of type `P` that the previous frame took.
pub(crate) fn ended<P: 'static>(ctx: &Context, source: Id) -> Option<DropOperation> {
    let frame = pass::root_frame_nr(ctx);
    board(ctx, |board| {
        board
            .ended
            .as_ref()
            .filter(|ended| ended.source == source && ended.payload == TypeId::of::<P>() && ended.frame + 1 == frame)
            .map(|ended| ended.op)
    })
}

/// The end hook: resolves this pass's registrations for the next pass, records the mark, requests the cursor,
/// and ends the session once its button is up. The session ends in the source window; when a target in another
/// window holds the drop, it waits there for one more frame so that window's pass takes it.
pub(crate) fn end_pass(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    let position = os_files_pointer(ctx, viewport, pointer::frame_raw(ctx).pos);
    // A modal holds input: targets on layers below it, and on the window, never accept, except OS files on a target
    // that takes them there.
    let os = carries_os_files(ctx);
    let scopes: Vec<(Option<Id>, bool)> = PerViewport::<Targets>::with(ctx, viewport, |targets| {
        targets.this.iter().map(|registration| (registration.scope, os && registration.reach.under_modal)).collect()
    });
    let open: Vec<bool> =
        scopes.iter().map(|(scope, through)| *through || overlay::key_layer(ctx, scope.as_slice(), None) != LayerKeys::Blocked).collect();
    let winner = PerViewport::<Targets>::with(ctx, viewport, |targets| {
        // The topmost layer in the order the overlay host handed egui wins, then the target registered last: a target
        // shown later lies over an enclosing one.
        let best = targets
            .this
            .iter()
            .enumerate()
            .filter(|(at, _)| open[*at])
            .max_by_key(|(at, registration)| (!registration.reach.window, areas::layer_position(ctx, registration.layer), *at))
            .map(|(_, registration)| registration);
        targets.component = best.is_some_and(|registration| !registration.reach.window);
        targets.winner_window = best.is_some_and(|registration| registration.reach.window);
        targets.pointer = position;
        let best =
            best.map(|registration| ((registration.owner, registration.ordinal), (registration.mark, registration.op, registration.tokens.clone())));
        targets.winner = best.as_ref().map(|(key, _)| *key);
        targets.this.clear();
        best.map(|(_, won)| won)
    });
    let over_this_window = pointer::frame_raw(ctx).pos.is_some();
    let primary_down = pointer::with_pass_raw(ctx, |pointer| pointer.primary_down);
    let buttons = pointer::frame_raw(ctx).buttons;
    let frame = pass::root_frame_nr(ctx);
    let held = board(ctx, |board| board.session.as_ref().map(|session| (session.os, session.button, session.dropped))).is_none_or(
        |(os, button, dropped)| match (os, button) {
            (true, _) => dropped != Some(frame),
            (_, PointerButton::Primary) => primary_down,
            _ => buttons.contains(Buttons::of(button)),
        },
    );
    let source = source_viewport(ctx);
    let is_source = source == Some(viewport);
    if is_source {
        windows::note_source_pointer(ctx, viewport, pointer::frame_raw(ctx).pos);
    }
    let elsewhere = is_source && held_elsewhere(ctx);
    let foreign_target = is_source && !held && (elsewhere || foreign_winner(ctx, viewport));
    let running = board(ctx, |board| {
        if let Some(session) = &mut board.session {
            let expired = session.released.is_some_and(|released| frame > released + 1);
            let ended_here = is_source && !held && session.released.is_none();
            if expired || (ended_here && !foreign_target) {
                let next = std::mem::take(&mut session.hovering_next);
                board.session = (!next.is_empty()).then(|| os_session(next, viewport, frame));
                board.started += u64::from(board.session.is_some());
            } else if ended_here {
                session.released = Some(frame);
            }
        }
        if let Some(session) = &mut board.session {
            match &winner {
                Some((mark, _, tokens)) => session.mark = Some(ShownMark { viewport, mark: *mark, tokens: tokens.clone() }),
                None if session.mark.as_ref().is_some_and(|shown| shown.viewport == viewport) => session.mark = None,
                None => {}
            }
        }
        board.session.is_some()
    });
    if !running {
        windows::clear_source_pointer(ctx);
    }
    if running && is_source {
        // The windows the pointer may be over resolve their targets again.
        for other in pass::live_viewports(ctx).into_iter().filter(|other| *other != viewport) {
            crate::runtime::egui::repaint::request_of(ctx, other);
        }
    }
    if running && over_this_window {
        if let Some((_, op, _)) = winner {
            let icon = match op {
                DropOperation::Copy => CursorIcon::Copy,
                DropOperation::Link => CursorIcon::Alias,
                _ => CursorIcon::Grabbing,
            };
            cursor::request(ctx, Cursor::System(icon), CursorPriority::DropTarget);
        } else if elsewhere {
            let icon = match current_operation(ctx, CopyKeys::Alt) {
                DropOperation::Copy => CursorIcon::Copy,
                DropOperation::Link => CursorIcon::Alias,
                _ => CursorIcon::Grabbing,
            };
            cursor::request(ctx, Cursor::System(icon), CursorPriority::DropTarget);
        } else {
            cursor::request(ctx, Cursor::System(CursorIcon::NotAllowed), CursorPriority::Drag);
        }
    }
}

/// Whether a component's target in another viewport than the running one lies under the pointer: one that won that
/// viewport's last pass, or one its running pass registered. The main window's pass runs around the passes of its OS
/// windows, so what it registered in this frame, at the pointer the source window saw last, comes before their end
/// hooks: a release in an OS window right as the pointer reaches the main window's target still drops there.
pub(crate) fn held_elsewhere(ctx: &Context) -> bool {
    let viewport = pass::current_viewport(ctx);
    pass::live_viewports(ctx).into_iter().filter(|other| *other != viewport).any(|other| {
        PerViewport::<Targets>::with(ctx, other, |targets| {
            (targets.winner.is_some() && targets.component) || targets.this.iter().any(|registration| !registration.reach.window)
        })
    })
}

/// Whether a viewport other than `viewport` has a target that took the pointer in its last pass.
fn foreign_winner(ctx: &Context, viewport: ViewportId) -> bool {
    pass::live_viewports(ctx)
        .into_iter()
        .filter(|other| *other != viewport)
        .any(|other| PerViewport::<Targets>::with(ctx, other, |targets| targets.winner.is_some()))
}

/// What the ghost layer paints: the session's preview and the mark of the target that would take a drop.
pub(crate) struct Ghost {
    pub preview: DragPreview,
    /// The env of the region the drag started in.
    pub env: Option<Env>,
    /// The mark of the target that would take a drop, and the tokens of the target's region.
    pub mark: Option<(DropMark, Arc<UiTokens>)>,
}

/// The running session's ghost data for the running viewport. The mark paints in the window whose target registered
/// it, and also in the window the session moves, mapped to the same desktop place: that window lies over the mark,
/// which would otherwise show only through it.
pub(crate) fn ghost(ctx: &Context) -> Option<Ghost> {
    let viewport = pass::current_viewport(ctx);
    let (preview, env, mark, moves_this) = board(ctx, |board| {
        board
            .session
            .as_ref()
            .map(|session| (session.preview.clone(), session.env.clone(), session.mark.clone(), session.moving.viewport == Some(viewport)))
    })?;
    let mark = mark.and_then(|shown| {
        let offset = if shown.viewport == viewport {
            Vec2::ZERO
        } else if moves_this {
            WindowHost::of(ctx).offset_between(shown.viewport, viewport)?
        } else {
            return None;
        };
        Some((shown.mark.mapped(|point| point + offset), shown.tokens))
    });
    Some(Ghost { preview, env, mark })
}
