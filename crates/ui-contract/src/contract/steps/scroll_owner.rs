//! Requests to the enclosing scroll owner: reveals and the page and end moves a component left untaken.

use crate::contract::cx::CxCore;
use crate::runtime::egui::{memory as store, repaint};
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
    store::Service::<ScrollMoves>::run(&core.ctx, |queue| queue.0.extend(moves.into_iter().map(|mv| (owner, mv))));
    if reveal {
        store::Service::<Reveals>::run(&core.ctx, |reveals| reveals.0.push(Reveal { owner, rect: core.rect }));
    }
    repaint::request(&core.ctx);
}

/// A request to bring a rect into view, read by the scroll owner that step 4 of its next pass names.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Reveal {
    pub owner: Id,
    pub rect: Rect,
}

#[derive(Default)]
pub(super) struct Reveals(pub(super) Vec<Reveal>);

/// The reveal request addressed to `owner`, once.
pub(crate) fn take_reveal(ctx: &Context, owner: Id) -> Option<Rect> {
    store::Service::<Reveals>::run(ctx, |reveals| {
        let at = reveals.0.iter().position(|reveal| reveal.owner == owner)?;
        Some(reveals.0.remove(at).rect)
    })
}

/// Queues a reveal request for `owner`.
#[cfg(feature = "testing")]
pub(crate) fn queue_reveal(ctx: &Context, owner: Id, rect: Rect) {
    store::Service::<Reveals>::run(ctx, |reveals| reveals.0.push(Reveal { owner, rect }));
}

/// The reveal request addressed to `owner`, left queued.
pub(crate) fn peek_reveal(ctx: &Context, owner: Id) -> Option<Rect> {
    store::Service::<Reveals>::run(ctx, |reveals| reveals.0.iter().find(|reveal| reveal.owner == owner).map(|reveal| reveal.rect))
}

/// A page or end move a focused component left untaken, addressed to the scroll owner around it (9.2 rule 5).
#[derive(Default)]
struct ScrollMoves(Vec<(Id, NavMove)>);

/// The scroll moves addressed to `owner`, in the order they arrived.
pub(crate) fn take_scroll_moves(ctx: &Context, owner: Id) -> Vec<NavMove> {
    store::Service::<ScrollMoves>::run(ctx, |moves| {
        let (mine, rest) = std::mem::take(&mut moves.0).into_iter().partition(|(id, _)| *id == owner);
        moves.0 = rest;
        mine.into_iter().map(|(_, mv)| mv).collect()
    })
}

/// The scroll moves addressed to `owner`, left queued.
pub(crate) fn peek_scroll_moves(ctx: &Context, owner: Id) -> Vec<NavMove> {
    store::Service::<ScrollMoves>::run(ctx, |moves| moves.0.iter().filter(|(id, _)| *id == owner).map(|(_, mv)| *mv).collect())
}
