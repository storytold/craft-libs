//! Requests to the enclosing scroll owner: reveals and the page and end moves a component left untaken.

use crate::contract::cx::CxCore;
use crate::runtime::egui::repaint;
use crate::runtime::pass_state;
use craft_behavior::{MoveMods, NavMove};
use egui::{Context, Id, Rect, Vec2};

/// What the component left untaken that a scroll region serves: page and end moves go to the innermost scroll
/// owner around the component, and an AccessKit request to show the component reveals its rect there. Both are
/// applied at the owner's step 4 of the next pass.
pub(super) fn route_to_scroll_owner(core: &mut CxCore<'_>) {
    let scrolls = |mv: NavMove, _| matches!(mv, NavMove::PageUp | NavMove::PageDown | NavMove::Start | NavMove::End);
    let mut moves = Vec::new();
    if let Some(collection) = core.env.fallback_collection().filter(|id| *id != core.id) {
        while let Some(mv) = core.intents.take_move() {
            moves.push(mv);
        }
        if !moves.is_empty() {
            pass_state::with::<ScrollMoves, _>(&core.ctx, |queue| queue.0.extend(moves.drain(..).map(|(mv, mods)| (collection, mv, mods, true))));
            repaint::request(&core.ctx);
        }
    }
    while let Some(mv) = core.intents.take_move_where(scrolls) {
        moves.push(mv);
    }
    let reveal = core.intents.take_scroll_access(true).is_some();
    if moves.is_empty() && !reveal {
        return;
    }
    let Some(owner) = core.env.scroll_owner() else { return };
    pass_state::with::<ScrollMoves, _>(&core.ctx, |queue| queue.0.extend(moves.into_iter().map(|(mv, mods)| (owner, mv, mods, false))));
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

/// Steps the scrolling body `owner` by one of its units: toward the right of the screen (`right`) or the left. `view`
/// is the rect the body shows this pass and `units` the rects of the units its content is made of (a ribbon's groups),
/// in any order. The step brings the nearest unit that lies past the view's edge on that side whole into view, by the
/// least move that shows it, at the owner's next pass, as a reveal does. Returns whether there was such a unit: at the
/// end of the content nothing moves. Half a point past the edge counts as inside: less than a pixel shows at any scale.
pub fn scroll_step(ctx: &Context, owner: Id, view: Rect, units: &[Rect], right: bool) -> bool {
    let past = |unit: &&Rect| if right { unit.max.x > view.max.x + 0.5 } else { unit.min.x < view.min.x - 0.5 };
    let nearest = if right {
        units.iter().filter(past).min_by(|a, b| a.min.x.total_cmp(&b.min.x))
    } else {
        units.iter().filter(past).max_by(|a, b| a.max.x.total_cmp(&b.max.x))
    };
    let Some(unit) = nearest else { return false };
    pass_state::with::<Reveals, _>(ctx, |reveals| reveals.0.push(Reveal { owner, to: RevealTo::Rect(*unit) }));
    repaint::request(ctx);
    true
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
/// Each move carries whether it came through a region's fallback collection (`ContainerProps::fallback_collection`).
pub(crate) struct ScrollMoves(Vec<(Id, NavMove, MoveMods, bool)>);

/// The scroll moves addressed to `owner`, in the order they arrived: taken, or left queued when `quiet`.
pub(crate) fn scroll_moves(ctx: &Context, owner: Id, quiet: bool) -> Vec<(NavMove, MoveMods)> {
    pass_state::with::<ScrollMoves, _>(ctx, |moves| {
        let mine = moves.0.iter().filter(|(id, ..)| *id == owner).map(|(_, mv, mods, _)| (*mv, *mods)).collect();
        if !quiet {
            moves.0.retain(|(id, ..)| *id != owner);
        }
        mine
    })
}

/// Moves with their modifiers, in the order they came.
type Moves = Vec<(NavMove, MoveMods)>;

/// [`scroll_moves`] apart: the page and end moves a component inside left for its scroll owner, and the moves a
/// region's fallback collection takes as its own keys (every move, page and end included, L-319).
pub(crate) fn scroll_and_fallback_moves(ctx: &Context, owner: Id, quiet: bool) -> (Moves, Moves) {
    pass_state::with::<ScrollMoves, _>(ctx, |moves| {
        let (mut scrolls, mut fallback) = (Vec::new(), Vec::new());
        for &(id, mv, mods, through_fallback) in &moves.0 {
            if id == owner {
                if through_fallback { fallback.push((mv, mods)) } else { scrolls.push((mv, mods)) }
            }
        }
        if !quiet {
            moves.0.retain(|(id, ..)| *id != owner);
        }
        (scrolls, fallback)
    })
}

/// Targets with queued moves, to wake settled regions containing them.
pub(crate) fn move_targets(ctx: &Context) -> Vec<Id> {
    pass_state::with::<ScrollMoves, _>(ctx, |moves| moves.0.iter().map(|(id, ..)| *id).collect())
}

/// The smallest move of `view` that shows `rect`, per axis in screen points (positive: toward larger coordinates), and
/// the part of `rect` the moved view shows: what a scroll owner moves for a reveal and passes on to the scroll owner
/// around it. Along an axis the view has no extent on it moves nothing, and along an axis `rect` spans the view on
/// it already shows all it can: a canvas control laid over the whole canvas, rulers included, moves no view inside them.
pub(crate) fn reveal_move(rect: Rect, view: Rect) -> (Vec2, Rect) {
    let into = |low: f32, high: f32, view_low: f32, view_high: f32| {
        if view_high <= view_low || (low <= view_low && high >= view_high) {
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
