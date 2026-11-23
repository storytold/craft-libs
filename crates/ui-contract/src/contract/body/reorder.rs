//! The reorder band: a section header whose section the stacking keys move.

use super::BodyHooks;
use crate::contract::cx::CxCore;
use crate::contract::{ContainerProps, DragBand};
use crate::runtime::commands;
use crate::runtime::keys;
use craft_behavior::{Axis, PartKey, ReorderKeys, keys as key_table};

#[doc(hidden)]
pub const REORDER_BAND: PartKey = PartKey::new("reorder_band");

/// The scope a container with a reorder band joins in its regions' command chain: no command belongs to it, so the
/// chain tells `commands::focus_within` that focus is inside the container.
pub(super) const REORDER_SCOPE: commands::CommandScope = commands::CommandScope::new("reorder");

impl BodyHooks<'_> {
    /// The stacking keys (`keys::REORDER_KEYS`) while focus is anywhere in a section whose header is a reorder band. A
    /// section has no stop of its own, so the keys would reach the stops in it: the container holds the chords for
    /// itself and turns them into `Intent::MoveItem` for its own update.
    pub(super) fn reorder_keys(&mut self, props: &ContainerProps, core: &mut CxCore<'_>) {
        if !props.header.is_some_and(|header| header.band == DragBand::Reorder) {
            return;
        }
        let (id, ctx) = (core.id, core.ctx.clone());
        let within = commands::focus_within(&ctx, id);
        for chord in key_table::REORDER_KEYS {
            if within {
                keys::capture(&ctx, id, chord);
            }
            while keys::take_claimed(&ctx, id, chord) {
                // A section's order is a vertical stack, moved by the stacking keys whatever scheme its rows use.
                if let Some(intent) = key_table::reorder_intent(chord.key, chord.mods, ReorderKeys::Stacking, Axis::Y) {
                    core.intents.push(intent);
                }
            }
        }
    }
}
