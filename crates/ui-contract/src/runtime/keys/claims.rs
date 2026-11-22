//! Chord claims and captures, and the held Escape of an edit run.

use super::current;
use craft_keymap::Chord;

crate::stays!(Chord);
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

/// Reads the earliest claimed chord of `id` that is one of `chords`, consuming it: the claimed chords of a frame come
/// back in the order they were pressed.
pub(crate) fn take_first_claimed(ctx: &Context, id: Id, chords: &[Chord]) -> Option<Chord> {
    if crate::runtime::measure::active(ctx) {
        return None;
    }
    current(ctx, |stage| {
        let at = stage.claimed.iter().position(|(owner, claimed)| *owner == id && chords.contains(claimed))?;
        Some(stage.claimed.remove(at).1)
    })
}

/// Records that `owner` has an open edit run: the next Escape cancels the run and stays with the component. Like the
/// claims below, a replay of the run that recorded it records it again: a focused view replays while nothing reaches it.
pub(crate) fn hold_escape(ctx: &Context, owner: Id) {
    crate::runtime::replay::journal(ctx, || owner, |ctx, &owner| hold_escape(ctx, owner));
    current(ctx, |stage| stage.open_edits.push(owner));
}

/// A state-dependent claim of `owner`, held from the next frame's input hook on while `owner` is the focused
/// component.
#[doc(hidden)]
pub fn claim(ctx: &Context, owner: Id, chord: Chord) {
    crate::runtime::replay::journal(ctx, || (owner, chord), |ctx, &(owner, chord)| claim(ctx, owner, chord));
    current(ctx, |stage| stage.claims.push((owner, chord)));
}

/// Holds `chord` for `owner` from the next input hook on, whichever component has focus: the key stage withholds
/// it from every component and `take_claimed` reads it. Used while a container is in a mode that owns the arrow
/// keys, Enter and Escape (the window move and size modes, 16.3); the claim lapses when it is not renewed.
#[doc(hidden)]
pub fn capture(ctx: &Context, owner: Id, chord: Chord) {
    crate::runtime::replay::journal(ctx, || (owner, chord), |ctx, &(owner, chord)| capture(ctx, owner, chord));
    // The captured chord reaches `owner` wherever the focus is: a run that captures reads what is sent.
    crate::runtime::replay::note_input(ctx, crate::runtime::inputs::Input::Sent);
    current(ctx, |stage| {
        if !stage.captures.contains(&(owner, chord)) {
            stage.captures.push((owner, chord));
        }
    });
}
