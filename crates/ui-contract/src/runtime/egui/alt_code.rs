//! Alt+numpad code entry, folded into the input events in the raw-input step.

use crate::runtime::env::AltCodePages;

/// The Alt+numpad entry in progress in one viewport.
#[derive(Default)]
pub(crate) struct AltCodeEntry;

/// Folds Alt+numpad entry out of `events` and into text events, decoding with `pages`.
#[allow(dead_code)] // P3(shell-w1): sh.K5
pub(crate) fn fold(entry: &mut AltCodeEntry, events: &mut Vec<egui::Event>, pages: AltCodePages) {
    let _ = (entry, events, pages);
    todo!("sh.K5")
}
