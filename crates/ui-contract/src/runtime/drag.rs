//! The drag session (8.4): one per context. It holds the payload, the preview, the held modifiers, the operation and the
//! mark of the drop target that would take the payload, as data. The drop-target registrations of a pass resolve in the
//! next pass, like the press arena's shapes. The ghost layer paints from this data (`contract::ghost`).

use crate::runtime::egui::areas::{self, Layer};
use crate::runtime::egui::{files, pass, viewports};
use crate::runtime::env::Label;
use crate::runtime::frame::PerViewport;
use crate::runtime::overlay::{self, LayerKeys};
use crate::runtime::pass_state;
use crate::runtime::windows::{self, WindowHost};
use crate::runtime::{cursor, frame, pointer};
use craft_behavior::keys::drop_operation;
use craft_behavior::{Buttons, CancelReason, Cursor, CursorIcon, CursorPriority, DropOperation, PointerButton, PressCancel};
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
    /// A frame-sized outline centred on the pointer, with an optional label inside its top corner: the ghost of a
    /// panel or window being moved.
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
    /// This rect is split, the payload taking one side.
    Split(Rect),
}

impl DropMark {
    /// An insertion line from `from` to `to`.
    pub const fn line(from: Pos2, to: Pos2) -> Self {
        DropMark::Line { from, to }
    }

    /// The mark with every point passed through `map`.
    pub(crate) fn mapped(self, map: impl Fn(Pos2) -> Pos2) -> Self {
        let rect = |rect: Rect| Rect::from_two_pos(map(rect.min), map(rect.max));
        match self {
            DropMark::Line { from, to } => DropMark::Line { from: map(from), to: map(to) },
            DropMark::Into(target) => DropMark::Into(rect(target)),
            DropMark::Split(target) => DropMark::Split(rect(target)),
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
}

/// What a component registers with `Pointer::window_drop_target`: a target over the whole window, which needs no
/// pointer position.
pub trait WindowDropTarget<P> {
    /// What a drop on this target carries to the component's caller.
    type Drop;

    /// `None` rejects. `op` is the operation the held modifiers pick, which a target may refuse.
    fn accept(&self, payload: &P, op: DropOperation) -> Option<Self::Drop>;

    /// Where `drop` would land, a value the ghost layer paints.
    fn mark(&self, drop: &Self::Drop) -> DropMark;
}

/// Where a drop target lies: a rect on an egui layer, in that layer's space. A component's own target lies over its
/// visible rect on its layer; a component that takes drops over an area it does not draw (a preset browser's canvas)
/// names the area with [`DropZone::new`] and registers with [`zone_target`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropZone {
    layer: Layer,
    rect: Rect,
}

impl DropZone {
    /// The zone `rect` on `layer` (the `layer_id` of the `Ui` that draws the area), in that layer's space.
    pub const fn new(layer: egui::LayerId, rect: Rect) -> Self {
        DropZone { layer, rect }
    }

    /// The zone's rect, in its layer's space.
    pub const fn rect(&self) -> Rect {
        self.rect
    }
}

/// What a drag moves along with the pointer when its source is a container: the container's layer, and its window
/// where it opens one. Targets never see the pointer through the layer or inside the window, so the targets under
/// the moving unit take the drop.
#[derive(Clone, Copy, Default)]
struct Moving {
    layer: Option<Layer>,
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
    /// A second payload of the same drag, which targets of its type take: a collection's row drag maps to it.
    also: Option<Arc<dyn Any + Send + Sync>>,
    preview: DragPreview,
    /// The mark of the target that would take a drop now, in screen space of the viewport that registered it.
    mark: Option<(ViewportId, DropMark)>,
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
    /// The root frame the session started in, the only one in which [`amend`] changes its payload.
    began: u64,
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
    /// The target covers the whole window: any target over a component beats it.
    window: bool,
}

/// A viewport's registrations: this pass's, and the target that won last pass.
#[derive(Default)]
pub(crate) struct Targets {
    this: Vec<Registration>,
    winner: Option<(Id, u32)>,
}

fn board<R>(ctx: &Context, f: impl FnOnce(&mut Board) -> R) -> R {
    pass_state::with::<Board, _>(ctx, f)
}

/// Starts a session for `source` with `payload`. A running session stays: one per context. A container shown in an OS
/// window of its own runs in its parent's pass, but the press that starts its drag is its window's: the session follows
/// that window's pointer.
#[doc(hidden)]
pub fn start(ctx: &Context, source: Id, payload: Arc<dyn Any + Send + Sync>, also: Option<Arc<dyn Any + Send + Sync>>, preview: DragPreview) {
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
                moving: Moving { layer: overlay::entry(ctx, source).map(|entry| entry.layer), viewport: Some(viewports::viewport_id(source)) },
                viewport,
                button,
                payload,
                also,
                preview,
                mark: None,
                released: None,
                taken: false,
                os: false,
                dropped: None,
                began,
            });
            board.ended = None;
            board.started += 1;
        }
    });
    crate::runtime::egui::repaint::request(ctx);
}

fn operation(ctx: &Context, mods: Mods) -> DropOperation {
    drop_operation(windows::os_of(ctx), mods)
}

/// Changes the running session's payload of type `P` (or its second payload of that type) with `change`, in the frame
/// the session started only: a host completes what a drag its component started carries with what only the host knows
/// (a dock measures the panel a tab drag floats). Returns whether a payload changed; from the next frame on, the
/// payload is fixed and nothing changes.
pub fn amend<P: Clone + Send + Sync + 'static>(ctx: &Context, change: impl FnOnce(&mut P)) -> bool {
    let frame = pass::root_frame_nr(ctx);
    board(ctx, |board| {
        let Some(session) = board.session.as_mut().filter(|session| session.began == frame) else { return false };
        let slot = if session.payload.is::<P>() { Some(&mut session.payload) } else { session.also.as_mut().filter(|also| also.is::<P>()) };
        let Some(slot) = slot else { return false };
        let Some(mut payload) = slot.downcast_ref::<P>().cloned() else { return false };
        change(&mut payload);
        *slot = Arc::new(payload);
        true
    })
}

/// The active drag payload of type `P`, from any viewport. For code without a `Cx`.
pub fn active<P: Send + Sync + 'static>(ctx: &Context) -> Option<Arc<P>> {
    active_of(ctx, TypeId::of::<P>())?.downcast::<P>().ok()
}

/// The active drag payload whose type is `payload`: the session's own payload, else its `also`. For a target that
/// picks the payload types it takes at run time.
pub(crate) fn active_of(ctx: &Context, payload: TypeId) -> Option<Arc<dyn Any + Send + Sync>> {
    let (own, also) = board(ctx, |board| board.session.as_ref().map(|session| (session.payload.clone(), session.also.clone())))?;
    if Any::type_id(&*own) == payload {
        return Some(own);
    }
    also.filter(|also| Any::type_id(&**also) == payload)
}

/// How many sessions have started on this context.
#[doc(hidden)]
pub fn started(ctx: &Context) -> u64 {
    board(ctx, |board| board.started)
}

/// The mark of the target that would take a drop now.
#[doc(hidden)]
pub fn mark(ctx: &Context) -> Option<DropMark> {
    board(ctx, |board| board.session.as_ref().and_then(|session| session.mark.map(|(_, mark)| mark)))
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

/// The begin hook: the window losing focus ends the session.
pub(crate) fn begin(ctx: &Context) {
    if source_viewport(ctx) == Some(pass::current_viewport(ctx)) && pointer::with_pass_pointer(ctx, |pointer| pointer.focus_lost) {
        cancel(ctx, CancelReason::Blur);
    }
    os_files(ctx);
}

/// The session of files the OS drags over the window (8.4): files held over the window start one, the dropped files
/// replace its payload in the frame of the drop, and files that leave without a drop end it.
fn os_files(ctx: &Context) {
    let seen = files::seen(ctx);
    let frame = pass::root_frame_nr(ctx);
    let viewport = pass::current_viewport(ctx);
    let started = board(ctx, |board| match &mut board.session {
        None if !seen.hovered.is_empty() => {
            board.session = Some(Session {
                source: Id::NULL.with("craft_ui::os_files"),
                moving: Moving::default(),
                viewport,
                button: PointerButton::Primary,
                payload: Arc::new(OsFiles { files: seen.hovered }),
                also: None,
                preview: DragPreview::None,
                mark: None,
                released: None,
                taken: false,
                os: true,
                dropped: None,
                began: frame,
            });
            board.ended = None;
            board.started += 1;
            true
        }
        Some(session) if session.os => {
            if !seen.dropped.is_empty() {
                let hovered = session.payload.downcast_ref::<OsFiles>();
                let files = seen.dropped.into_iter().map(|file| with_hovered_type(file, hovered)).collect();
                session.payload = Arc::new(OsFiles { files });
                session.dropped = Some(frame);
            } else if seen.hovered.is_empty() && session.dropped != Some(frame) {
                board.session = None;
            }
            false
        }
        _ => false,
    });
    if started {
        crate::runtime::egui::repaint::request(ctx);
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

/// The pointer in `viewport` for a drag that started in another window: the desktop pointer mapped into this
/// window, when it lies inside. `None` without a session, in the source window, or outside this window.
#[doc(hidden)]
pub fn cross_window_pointer(ctx: &Context, viewport: ViewportId) -> Option<Pos2> {
    let source = source_viewport(ctx)?;
    (source != viewport).then(|| WindowHost::of(ctx).pointer_in(viewport)).flatten()
}

/// The running session's payload whose type is `payload` and the pointer in screen space.
pub(crate) fn dragging_of(ctx: &Context, payload: TypeId) -> Option<(Arc<dyn Any + Send + Sync>, Pos2)> {
    let payload = active_of(ctx, payload)?;
    Some((payload, pointer::frame(ctx).pos?))
}

/// The layer the running session's source moves with the pointer, which hit tests for drop targets look through.
pub(crate) fn moving_layer(ctx: &Context) -> Option<Layer> {
    board(ctx, |board| board.session.as_ref().and_then(|session| session.moving.layer))
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

/// Starts a session whose source is the container `source`, as the title row of a dock window does, with no payload
/// of its own. For trials.
#[cfg(feature = "testing")]
pub(crate) fn begin_window_drag(ctx: &Context, source: Id) {
    start(ctx, source, Arc::new(()), None, DragPreview::None);
}

/// Sets the mark of the target that would take a drop, as one in another window would. For trials.
#[cfg(feature = "testing")]
pub(crate) fn show_mark(ctx: &Context, mark: Option<DropMark>) {
    board(ctx, |board| {
        if let Some(session) = &mut board.session {
            session.mark = mark.map(|mark| (ViewportId::from_hash_of("testing.mark"), mark));
        }
    });
}

/// Records that `owner`'s `ordinal`th target on `layer` accepts at the pointer, with its mark in layer space. A
/// `window` target covers the whole window.
pub(crate) fn register(ctx: &Context, owner: Id, layer: Layer, mark: DropMark, window: bool) {
    crate::runtime::replay::journal(ctx, || move |ctx: &Context| register(ctx, owner, layer, mark, window));
    let mark = match areas::layer_map(ctx, layer) {
        Some(map) => mark.mapped(|point| map.to_screen(point)),
        None => mark,
    };
    let scope = overlay::drawing_parent(ctx);
    PerViewport::<Targets>::with(ctx, pass::current_viewport(ctx), |targets| {
        let ordinal = targets.this.iter().filter(|registration| registration.owner == owner).count() as u32;
        targets.this.push(Registration { owner, ordinal, layer, scope, mark, window });
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

/// The operation the held modifiers pick now.
pub(crate) fn current_operation(ctx: &Context) -> DropOperation {
    operation(ctx, pointer::frame(ctx).mods)
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
    pointer::frame(ctx).release == Some(button)
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
/// anything), and returns the drop in the pass of the release when it won the pass before.
pub(crate) fn target_of<T: DropTarget<dyn Any + Send + Sync>>(
    ctx: &Context,
    owner: Id,
    zone: DropZone,
    payload_type: TypeId,
    target: &T,
) -> Option<T::Drop> {
    let (payload, screen) = dragging_of(ctx, payload_type)?;
    let at = areas::layer_map(ctx, zone.layer).map_or(screen, |map| map.to_layer(screen));
    let top = match moving_layer(ctx) {
        Some(moving) => areas::layer_at_without(ctx, screen, moving),
        None => areas::layer_at(ctx, screen),
    };
    if moves_viewport(ctx) || !zone.rect.contains(at) || top != Some(zone.layer) {
        return None;
    }
    let op = current_operation(ctx);
    let drop = target.accept(&*payload, at, op)?;
    let ordinal = next_ordinal(ctx, owner);
    register(ctx, owner, zone.layer, target.mark(&drop), false);
    take_drop(ctx, owner, ordinal, op, payload_type).then_some(drop)
}

/// `Pointer::drop_target` over a zone the caller names instead of its own visible rect: the drop of a payload of type
/// `P` released over `zone`, with the same layer order, operation and mark as a component's own target. `owner` is the
/// caller's stable id; it registers in every pass a drag may reach the zone.
pub fn zone_target<P: Send + Sync + 'static, T: DropTarget<P>>(ctx: &Context, owner: Id, zone: DropZone, target: &T) -> Option<T::Drop> {
    target_of(ctx, owner, zone, TypeId::of::<P>(), &Typed::new(target))
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
    // A modal holds input: targets on layers below it, and on the window, never accept.
    let scopes: Vec<Option<Id>> =
        PerViewport::<Targets>::with(ctx, viewport, |targets| targets.this.iter().map(|registration| registration.scope).collect());
    let open: Vec<bool> = scopes.iter().map(|scope| overlay::key_layer(ctx, scope.as_slice(), None) != LayerKeys::Blocked).collect();
    let winner = PerViewport::<Targets>::with(ctx, viewport, |targets| {
        // The topmost layer in the order the overlay host handed egui wins, then the target registered last: a target
        // shown later lies over an enclosing one.
        let best = targets
            .this
            .iter()
            .enumerate()
            .filter(|(at, _)| open[*at])
            .max_by_key(|(at, registration)| (!registration.window, areas::layer_position(ctx, registration.layer), *at))
            .map(|(_, registration)| registration);
        let best = best.map(|registration| ((registration.owner, registration.ordinal), registration.mark));
        targets.winner = best.map(|(key, _)| key);
        targets.this.clear();
        best.map(|(_, mark)| mark)
    });
    let op = current_operation(ctx);
    let over_this_window = pointer::frame(ctx).pos.is_some();
    let primary_down = pointer::with_pass_pointer(ctx, |pointer| pointer.primary_down);
    let buttons = pointer::frame(ctx).buttons;
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
        windows::note_source_pointer(ctx, viewport, pointer::frame(ctx).pos);
    }
    let foreign_target = is_source && !held && foreign_winner(ctx, viewport);
    let running = board(ctx, |board| {
        if let Some(session) = &mut board.session {
            let expired = session.released.is_some_and(|released| frame > released + 1);
            let ended_here = is_source && !held && session.released.is_none();
            if expired || (ended_here && !foreign_target) {
                board.session = None;
            } else if ended_here {
                session.released = Some(frame);
            }
        }
        if let Some(session) = &mut board.session {
            match winner {
                Some(mark) => session.mark = Some((viewport, mark)),
                None if session.mark.is_some_and(|(marked, _)| marked == viewport) => session.mark = None,
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
        match winner {
            None => cursor::request(ctx, Cursor::System(CursorIcon::NotAllowed), CursorPriority::Drag),
            Some(_) => {
                let icon = match op {
                    DropOperation::Copy => CursorIcon::Copy,
                    DropOperation::Link => CursorIcon::Alias,
                    _ => CursorIcon::Grabbing,
                };
                cursor::request(ctx, Cursor::System(icon), CursorPriority::DropTarget);
            }
        }
    }
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
    pub mark: Option<DropMark>,
}

/// The running session's ghost data.
pub(crate) fn ghost(ctx: &Context) -> Option<Ghost> {
    let viewport = pass::current_viewport(ctx);
    board(ctx, |board| {
        board.session.as_ref().map(|session| Ghost {
            preview: session.preview.clone(),
            mark: session.mark.and_then(|(marked, mark)| (marked == viewport).then_some(mark)),
        })
    })
}
