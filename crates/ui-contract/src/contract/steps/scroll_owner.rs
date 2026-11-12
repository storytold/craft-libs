//! Requests to the enclosing scroll owner: reveals and the page and end moves a component left untaken.

use crate::contract::cx::CxCore;
use crate::runtime::egui::repaint;
use crate::runtime::pass_state;
use crate::runtime::wheel;
use craft_behavior::{MoveMods, NavMove};
use egui::{Context, Id, Rect, Vec2};

/// What the component left untaken that a scroll region serves: page and end moves go to the innermost scroll
/// owner around the component, and an AccessKit request to show the component reveals its rect there. Both are
/// applied at the owner's step 4 of the next pass.
pub(super) fn route_to_scroll_owner(core: &mut CxCore<'_>) {
    let scrolls = |mv: NavMove, _| matches!(mv, NavMove::PageUp | NavMove::PageDown | NavMove::Start | NavMove::End);
    let mut moves = Vec::new();
    while let Some(mv) = core.intents.take_move_where(scrolls) {
        moves.push(mv);
    }
    let reveal = core.intents.take_scroll_access(true).is_some();
    if moves.is_empty() && !reveal {
        return;
    }
    let Some(owner) = core.env.scroll_owner().or_else(|| wheel::nearest_scroll_owner(&core.ctx, core.rect)) else { return };
    pass_state::with::<ScrollMoves, _>(&core.ctx, |queue| queue.0.extend(moves.into_iter().map(|(mv, mods)| (owner, mv, mods))));
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

/// A page or end move a focused component left untaken, with its modifiers, addressed to the scroll owner around it
/// (9.2 rule 5).
#[derive(Default)]
pub(crate) struct ScrollMoves(Vec<(Id, NavMove, MoveMods)>);

/// The scroll moves addressed to `owner`, in the order they arrived: taken, or left queued when `quiet`.
pub(crate) fn scroll_moves(ctx: &Context, owner: Id, quiet: bool) -> Vec<(NavMove, MoveMods)> {
    pass_state::with::<ScrollMoves, _>(ctx, |moves| {
        let mine = moves.0.iter().filter(|(id, ..)| *id == owner).map(|(_, mv, mods)| (*mv, *mods)).collect();
        if !quiet {
            moves.0.retain(|(id, ..)| *id != owner);
        }
        mine
    })
}

/// The smallest move of `view` that shows `rect`, per axis in screen points (positive: toward larger coordinates), and
/// the part of `rect` the moved view shows: what a scroll owner moves for a reveal and passes on to the scroll owner
/// around it. Along an axis the view has no extent on it moves nothing.
pub(crate) fn reveal_move(rect: Rect, view: Rect) -> (Vec2, Rect) {
    let into = |low: f32, high: f32, view_low: f32, view_high: f32| {
        if view_high <= view_low {
            0.0
        } else if low < view_low {
            low - view_low
        } else if high > view_high {
            high - view_high
        } else {
            0.0
        }
    };
    let by = Vec2::new(into(rect.min.x, rect.max.x, view.min.x, view.max.x), into(rect.min.y, rect.max.y, view.min.y, view.max.y));
    (by, rect.translate(-by).intersect(view))
}
