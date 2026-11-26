//! The open state of a container's layer with no binding, kept in its opener's slot, and `Popup`.

use super::opener::{Opener, Opening, Trigger};
use super::popups::{HOLD, PopupHandle, dwell, with_popups};
use crate::contract::{Component, Cx};
use crate::runtime::frame;
use crate::runtime::memory::{MemoryStage, sub};
use crate::runtime::overlay::{self};
use egui::Id;

/// Open state a container with no `open` binding keeps in its opener's slot, and the readers of a popup's frame.
#[derive(Clone, Copy, Default)]
struct OpenerOpen {
    open: bool,
    /// The root frame an opener's trigger was last applied in: a frame's passes apply it once.
    applied: u64,
    /// The last root frame a real run of the container read the slot: the layer ran, shown or closed. `None` until a
    /// real run reads it, which a programmatic open before the container ever ran leaves.
    seen: Option<u64>,
    /// The last root frame something opened or closed the slot (a trigger, a hold, `Popup`, the layer's own close).
    written: Option<u64>,
    /// The opener that opened the layer by id and trigger, and how it fired.
    by: Option<(Id, Trigger, Opening)>,
}

/// The note a replay of the container's run re-issues: its first opener, so the slot's last-shown frame is the replay's
/// pass ([`refresh_seen`]). An id does not move: a replay at another place keeps it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SeenNote(Id);

impl crate::runtime::shift::Shift for SeenNote {
    fn shifted(&self, _: &egui::RecordShift) -> Self {
        *self
    }

    fn snapped(&self, _: &Self, _: f32) -> Self {
        *self
    }
}

/// The container `note` names ran in this pass: its opener slot's last-shown frame becomes the replay's, so the next real
/// run finds no gap (`opener_open`).
fn refresh_seen(ctx: &egui::Context, note: &SeenNote) {
    let frame = crate::runtime::egui::pass::root_frame_nr(ctx);
    let mut own = MemoryStage::open_after(ctx, note.0, note.0);
    mark_shown(own.session::<OpenerOpen>(sub(note.0, "opens")), frame);
    own.write_back();
}

/// The container was shown in `frame`, by a real run or by a replay of one. A slot still open after a frame the
/// container did not run, with nothing having opened it since, reads closed (see `opener_open`), and the slot's
/// last-shown frame becomes this pass's. A replay applies it too: a view that was absent and shows again must not keep
/// a layer its absence left open.
fn mark_shown(slot: &mut OpenerOpen, frame: u64) {
    if slot.open
        && let Some(seen) = slot.seen
        && seen.saturating_add(1) < frame
        && slot.written.is_none_or(|written| written <= seen)
    {
        slot.open = false;
        slot.by = None;
    }
    slot.seen = Some(frame);
}

/// The open state of a container with no binding: the slot of its first opener, with the triggers of every opener
/// applied once for the frame. A container with no opener keeps its own slot.
pub(crate) fn opener_open(stage: &mut MemoryStage, first: &Opener) -> bool {
    let ctx = stage.ctx().clone();
    let frame = crate::runtime::egui::pass::root_frame_nr(&ctx);
    let held = first.all().find(|opener| opener.trigger == Trigger::Hold).map(|opener| {
        // The dwell is the opener's: recorded under its id, it goes with the opener's eviction whether the container
        // shows or not.
        let mut own = if stage.is_peek() { MemoryStage::open_peek(&ctx, opener.id) } else { MemoryStage::open_after(&ctx, opener.id, opener.id) };
        let started = dwell(&ctx, &mut own, opener.id, "dwell", opener.pressed, HOLD);
        own.write_back();
        (opener, started)
    });
    let real = !stage.is_peek();
    let slot = stage.session::<OpenerOpen>(sub(first.id, "opens"));
    // A layer does not outlive its anchor's absence: one that was open when the container stopped running (its opener
    // left with the row it acts on, or the panel holding both stopped drawing) never ran its anchor-gone close, so it
    // reads closed when it runs again after a gap, unless something opened it since. A peek reads it too, and expires the
    // same state: a peek's stage is a copy, so what this pass measures is what its real run will read, even where the
    // opener is clicked in the pass the container returns.
    mark_shown(slot, frame);
    if real {
        // A memoized view holding this container replays it without running it (nothing in it changed), so the replay
        // re-issues this note: the container does not read as gone in the pass it runs again and close a menu it shows.
        crate::runtime::replay::journal(&ctx, || SeenNote(first.id), refresh_seen);
    }
    if let Some(opener) = first.all().find(|opener| opener.fired) {
        if slot.applied != frame {
            // A context request opens, also over the open layer: a new right-click moves it to the pointer.
            slot.open = opener.trigger == Trigger::Context || !slot.open;
            slot.by = Some((opener.id, opener.trigger, opener.opening));
            slot.written = Some(frame);
        }
        slot.applied = frame;
        overlay::claim_activation(&ctx, opener.id);
    } else if let Some((opener, true)) = held
        && !slot.open
    {
        slot.open = true;
        slot.by = Some((opener.id, opener.trigger, Opening::Plain));
        slot.written = Some(frame);
    }
    for opener in first.all().filter(|opener| opener.clicked) {
        overlay::claim_activation(&ctx, opener.id);
    }
    let (open, by) = (slot.open, slot.by);
    // A peek stage reads the slot the real stage of the pass holds, so it sees none of what that stage decided:
    // the opener the layer opened by stays as the real stage recorded it.
    if !stage.is_peek() {
        with_popups(&ctx, |popups| match by {
            Some(by) if open => popups.opened_by.insert(first.id, by),
            _ => popups.opened_by.remove(&first.id),
        });
    }
    open
}

/// Whether the container hung from the opener `opener` (the id of its response, [`Overlay::opened_by`]) is open, as
/// its slot holds it: an opener reports its expanded state by it.
///
/// [`Overlay::opened_by`]: super::Overlay::opened_by
pub fn opened_from(ctx: &egui::Context, opener: Id) -> bool {
    MemoryStage::peek_session::<OpenerOpen>(ctx, sub(opener, "opens")).open
}

/// Writes the opener's slot: the container closed itself (Escape, a press outside, a row ran).
pub(crate) fn set_opener_open(stage: &mut MemoryStage, opener: &Opener, open: bool) {
    let frame = crate::runtime::egui::pass::root_frame_nr(stage.ctx());
    let slot = stage.session::<OpenerOpen>(sub(opener.id, "opens"));
    slot.open = open;
    slot.written = Some(frame);
}

/// The open state of a container's layer that has an opener and no binding, driven from the opener's side.
pub struct Popup;

impl Popup {
    /// Opens the container hung from `opener` (a [`PopupHandle`] or the id of the opener's response).
    pub fn open<C: Component>(opener: impl Into<Id>, cx: &mut Cx<'_, C>) {
        Self::write(opener.into(), cx, Some(true));
    }

    /// Closes the container hung from `opener`.
    pub fn close<C: Component>(opener: impl Into<Id>, cx: &mut Cx<'_, C>) {
        Self::write(opener.into(), cx, Some(false));
    }

    /// Opens the container hung from `opener` when it is closed, else closes it.
    pub fn toggle<C: Component>(opener: impl Into<Id>, cx: &mut Cx<'_, C>) {
        Self::write(opener.into(), cx, None);
    }

    fn write<C: Component>(opener: Id, cx: &mut Cx<'_, C>, to: Option<bool>) {
        let frame = crate::runtime::egui::pass::root_frame_nr(&cx.core.ctx);
        let slot = cx.core.memory.session::<OpenerOpen>(sub(opener, "opens"));
        slot.open = to.unwrap_or(!slot.open);
        slot.by = None;
        slot.written = Some(frame);
        overlay::claim_activation(&cx.core.ctx, opener);
        frame::request_repaint(&cx.core.ctx);
    }

    /// Whether the layer `popup` shows first in this pass: true in the passes of the frame it first shows, false
    /// in the frames after, so what a body resets on opening resets once.
    pub fn opening<C: Component>(popup: PopupHandle, cx: &Cx<'_, C>) -> bool {
        overlay::opening(&cx.core.ctx, popup.id)
    }
}
