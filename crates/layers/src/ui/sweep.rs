//! The sweep helper the toggle columns of the Layers and Layer Comps panels share: the cells of a sweep as events,
//! grouped the same way in both panels.

use std::sync::Arc;

use craft_behavior::EditEvent;
use craft_ui::contract::{SweepApply, ToggleEdit};

/// The cells of a sweep over `column` as events in `out`. The cells of one sweep are `Update`s of the gesture of its
/// first, opened by a `Begin` and closed by one `Commit` on release; the cells of one pass share one event, in the order
/// crossed. An Escape reports the cells as one `Cancel` of that gesture with the state each had before; a `Cancel` joins
/// the last event only when its `before` equals that event's. A plain press is a `Set`. Alt on the first press of a
/// show-only column shows only that row and drops the rest of the sweep.
// P3(framework-01-w2): L-95 decision, `sweep_events`
#[allow(dead_code)] // P3(framework-01-w2): sweep_events
pub(crate) fn sweep_events<K: Clone, C: Copy + PartialEq, O: SweepOut<K, C>>(
    sweep: &mut SweepApply,
    column: C,
    edit: EditEvent<ToggleEdit<K>>,
    label: &mut Arc<str>,
    out: &mut O,
) {
    let _ = (sweep, column, edit, label, out);
    todo!("framework-01-w2")
}

/// Where [`sweep_events`] writes a panel's toggle events.
// P3(framework-01-w2): L-95 decision, `SweepOut`
#[allow(dead_code)] // P3(framework-01-w2): SweepOut
pub(crate) trait SweepOut<K, C> {
    /// The last event when it is a toggle: its column, its edit and its rows.
    fn last_toggle(&mut self) -> Option<(C, &EditEvent<bool>, &mut Vec<K>)>;

    /// A toggle of `rows` in `column`.
    fn push_toggle(&mut self, rows: Vec<K>, column: C, edit: EditEvent<bool>);

    /// Alt on the first press of a show-only column: show only `key`. A plain column never gives it.
    fn show_only(&mut self, key: K) {
        let _ = key;
    }

    /// The cancel of a show-only press on `key`. A plain column never gives it.
    fn undo_show_only(&mut self, key: K) {
        let _ = key;
    }
}
