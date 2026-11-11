//! Step 15, settle.

use super::apply::{ValueEdit, apply_values};
use super::scroll_owner::{Reveal, Reveals};
use crate::contract::{Component, Cx, ValueOf};
use crate::runtime::egui::{memory as store, repaint};
use crate::runtime::focus::{self};
use crate::runtime::wheel;

/// Step 15, settle: child edits adopted with `cx.adopt` join the pass's events and are applied, and the open state of the
/// component's popups is written. A reveal request goes to the nearest scroll owner; anything queued for the next pass requests a repaint.
pub(crate) fn settle<C: Component>(comp: &mut C, cx: &mut Cx<'_, C>) -> ValueEdit {
    let adopted = std::mem::take(&mut cx.core.paint.borrow_mut().adopted);
    for event in adopted.into_iter().filter_map(|adopted| adopted.take::<ValueOf<C>>()) {
        if let Some(event) = cx.absorb(event) {
            cx.events.push(event);
        }
    }
    let edit = apply_values(comp, cx);
    let owner = cx.core.id;
    if C::OVERLAYS {
        crate::contract::overlay::write_open(&mut *cx.core.memory, owner);
    }
    if focus::take_reveal_of(&cx.core.ctx, owner) {
        cx.core.reveal.get_or_insert(cx.core.rect);
    }
    if let Some(rect) = cx.core.reveal.take()
        && let Some(owner) = cx.core.env.scroll_owner().or_else(|| wheel::nearest_scroll_owner(&cx.core.ctx, rect))
    {
        store::Service::<Reveals>::run(&cx.core.ctx, |reveals| reveals.0.push(Reveal { owner, rect }));
        repaint::request(&cx.core.ctx);
    }
    edit
}
