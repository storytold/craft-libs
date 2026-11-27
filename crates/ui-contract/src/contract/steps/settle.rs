//! Step 15, settle.

use super::apply::{ValueEdit, apply_values};
use super::scroll_owner::{Reveal, RevealTo, Reveals};
use crate::contract::{Component, Cx, ValueOf};
use crate::runtime::egui::repaint;
use crate::runtime::focus::{self};
use crate::runtime::pass_state;

/// Step 15, settle: child edits adopted with `cx.adopt` join the pass's events and are applied, and the open state of the
/// component's popups is written. A reveal request goes to the scroll owner around the component, from a component
/// `in_flow` (one that takes room in its parent's flow); anything queued for the next pass requests a repaint.
pub(crate) fn settle<C: Component>(comp: &mut C, cx: &mut Cx<'_, C>, in_flow: bool) -> ValueEdit {
    let adopted = std::mem::take(&mut cx.core.paint.borrow_mut().adopted);
    for event in adopted.into_iter().filter_map(|adopted| adopted.take::<ValueOf<C>>()) {
        if let Some(event) = cx.absorb(event) {
            cx.events.push(event);
        }
    }
    let edit = apply_values(comp, cx);
    let id = cx.core.id;
    if C::OVERLAYS {
        crate::contract::overlay::write_open(&mut *cx.core.memory, id);
    }
    if let Some(part) = focus::take_reveal_of(&cx.core.ctx, id) {
        // Focus on a part reveals the part, as a pin of a canvas overlay that spans the canvas.
        let rect = part.and_then(|part| cx.core.results.get(&(part.key, part.index))).map_or(cx.core.rect, |result| result.rect);
        cx.core.reveal.get_or_insert(rect);
    }
    // The owner is the innermost scrolling body or collection the component is shown in, as its scope names it: a
    // region beside it on screen holds none of it, and with no owner around it nothing scrolls. A component on its own
    // layer or window starts a scope with no owner, so its reveals end at it.
    if let Some(rect) = cx.core.reveal.take()
        && in_flow
        && let Some(owner) = cx.core.env.scroll_owner()
    {
        pass_state::with::<Reveals, _>(&cx.core.ctx, |reveals| reveals.0.push(Reveal { owner, to: RevealTo::Rect(rect) }));
        repaint::request(&cx.core.ctx);
    }
    edit
}
