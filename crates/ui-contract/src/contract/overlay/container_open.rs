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
    /// The opener that opened the layer by id and trigger, and how it fired.
    by: Option<(Id, Trigger, Opening)>,
}

/// The open state of a container with no binding: the slot of its first opener, with the triggers of every opener
/// applied once for the frame. A container with no opener keeps its own slot.
pub(crate) fn opener_open(stage: &mut MemoryStage, first: &Opener) -> bool {
    let ctx = stage.ctx().clone();
    let frame = crate::runtime::egui::pass::root_frame_nr(&ctx);
    let held = first.all().find(|opener| opener.trigger == Trigger::Hold).map(|opener| {
        let started = dwell(&ctx, stage, opener.id, "dwell", opener.pressed, HOLD);
        (opener, started)
    });
    let slot = stage.session::<OpenerOpen>(sub(first.id, "opens"));
    if let Some(opener) = first.all().find(|opener| opener.fired) {
        if slot.applied != frame {
            slot.open = !slot.open;
            slot.by = Some((opener.id, opener.trigger, opener.opening));
        }
        slot.applied = frame;
        overlay::claim_activation(&ctx, opener.id);
    } else if let Some((opener, true)) = held
        && !slot.open
    {
        slot.open = true;
        slot.by = Some((opener.id, opener.trigger, Opening::Plain));
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

/// Writes the opener's slot: the container closed itself (Escape, a press outside, a row ran).
pub(crate) fn set_opener_open(stage: &mut MemoryStage, opener: &Opener, open: bool) {
    stage.session::<OpenerOpen>(sub(opener.id, "opens")).open = open;
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
        let slot = cx.core.memory.session::<OpenerOpen>(sub(opener, "opens"));
        slot.open = to.unwrap_or(!slot.open);
        slot.by = None;
        overlay::claim_activation(&cx.core.ctx, opener);
        frame::request_repaint(&cx.core.ctx);
    }

    /// Whether the layer `popup` shows first in this pass: true in the passes of the frame it first shows, false
    /// in the frames after, so what a body resets on opening resets once.
    pub fn opening<C: Component>(popup: PopupHandle, cx: &Cx<'_, C>) -> bool {
        overlay::opening(&cx.core.ctx, popup.id)
    }
}
