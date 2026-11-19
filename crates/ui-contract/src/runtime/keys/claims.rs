//! Chord claims and captures, and the held Escape of an edit run.

use super::current;
use craft_keymap::Chord;
use egui::{Context, Id};

/// Reads the claimed chord: whether it arrived this pass for `id`, consuming it. Also true for a chord
/// withheld from the component's text or editor part.
#[doc(hidden)]
pub fn take_claimed(ctx: &Context, id: Id, chord: Chord) -> bool {
    if crate::runtime::measure::active(ctx) {
        return false;
    }
    current(ctx, |stage| match stage.claimed.iter().position(|(owner, claimed)| *owner == id && *claimed == chord) {
        Some(at) => {
            stage.claimed.remove(at);
            true
        }
        None => false,
    })
}

/// Records that `owner` has an open edit run: the next Escape cancels the run and stays with the component.
pub(crate) fn hold_escape(ctx: &Context, owner: Id) {
    current(ctx, |stage| stage.open_edits.push(owner));
}

/// A state-dependent claim of `owner`, held from the next frame's input hook on while `owner` is the focused
/// component.
#[doc(hidden)]
pub fn claim(ctx: &Context, owner: Id, chord: Chord) {
    current(ctx, |stage| stage.claims.push((owner, chord)));
}

/// Hands the first of `chords` (a fired command's keys, in binding order) that the focused component claims, as it is
/// focused now, to the claim's owner in the next pass, as a press of the chord arrives: a command run from a menu or
/// the palette whose key a component claims (a canvas tool's Delete) reaches the component as that key, so key and
/// menu share one path. A menu pick returns focus in its pass before its command fires. `false` when the focused
/// component claims none of them.
pub(crate) fn fire_claimed(ctx: &Context, chords: &[Chord]) -> bool {
    let Some(rec) = crate::runtime::focus::focused_rec(ctx) else { return false };
    let root = (rec.stop.root != rec.stop.id && !rec.stop.text).then_some(rec.stop.root);
    let found = current(ctx, |stage| {
        let held = |chord: &Chord| {
            let owns = |owner: &Id| *owner == rec.stop.id || Some(*owner) == root;
            let state = stage.last_claims.iter().chain(&stage.claims).find(|(owner, held)| held == chord && owns(owner));
            state.map(|(owner, _)| *owner).or_else(|| rec.stop.keys.claims.contains(chord).then_some(rec.stop.id))
        };
        chords.iter().find_map(|chord| held(chord).map(|owner| (owner, *chord)))
    });
    let Some(claim) = found else { return false };
    current(ctx, |stage| stage.fired_claims.push(claim));
    crate::runtime::egui::repaint::request_of(ctx, crate::runtime::egui::pass::current_viewport(ctx));
    true
}

/// Holds `chord` for `owner` from the next input hook on, whichever component has focus: the key stage withholds
/// it from every component and `take_claimed` reads it. Used while a container is in a mode that owns the arrow
/// keys, Enter and Escape (the window move and size modes, 16.3); the claim lapses when it is not renewed.
#[doc(hidden)]
pub fn capture(ctx: &Context, owner: Id, chord: Chord) {
    current(ctx, |stage| {
        if !stage.captures.contains(&(owner, chord)) {
            stage.captures.push((owner, chord));
        }
    });
}
