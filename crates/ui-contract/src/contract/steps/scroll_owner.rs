//! Requests to the enclosing scroll owner: reveals and the page and end moves a component left untaken.

use crate::contract::cx::CxCore;
use crate::runtime::egui::repaint;
use crate::runtime::pass_state;
use crate::runtime::wheel;
use craft_behavior::NavMove;
use egui::{Context, Id, Rect};

/// What the component left untaken that a scroll region serves: page and end moves go to the innermost scroll
/// owner around the component, and an AccessKit request to show the component reveals its rect there. Both are
/// applied at the owner's step 4 of the next pass.
pub(super) fn route_to_scroll_owner(core: &mut CxCore<'_>) {
    let mut moves = Vec::new();
    while let Some(mv) = core.intents.take_scroll_move() {
        moves.push(mv);
    }
    let reveal = core.intents.take_scroll_access(true).is_some();
    if moves.is_empty() && !reveal {
        return;
    }
    let Some(owner) = core.env.scroll_owner().or_else(|| wheel::nearest_scroll_owner(&core.ctx, core.rect)) else { return };
    pass_state::with::<ScrollMoves, _>(&core.ctx, |queue| queue.0.extend(moves.into_iter().map(|mv| (owner, mv))));
    if reveal {
        pass_state::with::<Reveals, _>(&core.ctx, |reveals| reveals.0.push(Reveal { owner, to: RevealTo::Rect(core.rect) }));
    }
    repaint::request(&core.ctx);
}

/// A request to bring something into view, read by the scroll owner that step 4 of its next pass names.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Reveal {
    pub owner: Id,
    pub to: RevealTo,
}

/// What a reveal brings into view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RevealTo {
    /// A rect, by the least move that shows it, at once: a focused or AccessKit-shown component.
    Rect(Rect),
    /// A `Lazy` child of the owner's body, by its id: the body glides until the child starts at the view's start.
    Child(Id),
}

#[derive(Default)]
pub(crate) struct Reveals(pub(super) Vec<Reveal>);

/// The rect reveal addressed to `owner`, once.
pub(crate) fn take_reveal(ctx: &Context, owner: Id) -> Option<Rect> {
    pass_state::with::<Reveals, _>(ctx, |reveals| {
        let at = reveals.0.iter().position(|reveal| reveal.owner == owner && matches!(reveal.to, RevealTo::Rect(_)))?;
        match reveals.0.remove(at).to {
            RevealTo::Rect(rect) => Some(rect),
            RevealTo::Child(_) => None,
        }
    })
}

/// Queues a reveal request for `owner`.
#[cfg(feature = "testing")]
pub(crate) fn queue_reveal(ctx: &Context, owner: Id, rect: Rect) {
    pass_state::with::<Reveals, _>(ctx, |reveals| reveals.0.push(Reveal { owner, to: RevealTo::Rect(rect) }));
}

/// The rect reveal addressed to `owner`, left queued.
pub(crate) fn peek_reveal(ctx: &Context, owner: Id) -> Option<Rect> {
    pass_state::with::<Reveals, _>(ctx, |reveals| {
        reveals.0.iter().find_map(|reveal| match reveal.to {
            RevealTo::Rect(rect) if reveal.owner == owner => Some(rect),
            _ => None,
        })
    })
}

/// Asks the scroll body `owner` to bring its `Lazy` child `child` to the view's start; a later request replaces it.
pub(crate) fn queue_child_reveal(ctx: &Context, owner: Id, child: Id) {
    pass_state::with::<Reveals, _>(ctx, |reveals| {
        reveals.0.retain(|reveal| !(reveal.owner == owner && matches!(reveal.to, RevealTo::Child(_))));
        reveals.0.push(Reveal { owner, to: RevealTo::Child(child) });
    });
    repaint::request(ctx);
}

/// The child reveal addressed to `owner`: taken, or left queued when `quiet`.
pub(crate) fn child_reveal(ctx: &Context, owner: Id, quiet: bool) -> Option<Id> {
    pass_state::with::<Reveals, _>(ctx, |reveals| {
        let at = reveals.0.iter().position(|reveal| reveal.owner == owner && matches!(reveal.to, RevealTo::Child(_)))?;
        let reveal = if quiet { reveals.0[at] } else { reveals.0.remove(at) };
        match reveal.to {
            RevealTo::Child(child) => Some(child),
            RevealTo::Rect(_) => None,
        }
    })
}

/// A page or end move a focused component left untaken, addressed to the scroll owner around it (9.2 rule 5).
#[derive(Default)]
pub(crate) struct ScrollMoves(Vec<(Id, NavMove)>);

/// The scroll moves addressed to `owner`, in the order they arrived.
pub(crate) fn take_scroll_moves(ctx: &Context, owner: Id) -> Vec<NavMove> {
    pass_state::with::<ScrollMoves, _>(ctx, |moves| {
        let (mine, rest) = std::mem::take(&mut moves.0).into_iter().partition(|(id, _)| *id == owner);
        moves.0 = rest;
        mine.into_iter().map(|(_, mv)| mv).collect()
    })
}

/// The scroll moves addressed to `owner`, left queued.
pub(crate) fn peek_scroll_moves(ctx: &Context, owner: Id) -> Vec<NavMove> {
    pass_state::with::<ScrollMoves, _>(ctx, |moves| moves.0.iter().filter(|(id, _)| *id == owner).map(|(_, mv)| *mv).collect())
}
