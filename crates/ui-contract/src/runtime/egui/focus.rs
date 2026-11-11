//! egui focus, `move_focus` and `surrender_focus_on`.

use egui::{Context, EventFilter, FocusDirection, Id, Pos2, SurrenderFocusOn};

/// Stops egui from taking focus away on its own: focus on press is the runtime's (8.3).
pub(crate) fn never_surrender_focus(ctx: &Context) {
    ctx.options_mut(|options| options.input_options.surrender_focus_on = SurrenderFocusOn::Never);
}

/// The key filter a focused component keeps: egui keeps the keys it lists from moving focus. Arrows and Enter
/// stay in raw input for egui's `TextEdit`; Escape is the runtime's, so the filter keeps it only for a
/// component that claims it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct KeyFilter {
    pub(crate) tab: bool,
    pub(crate) horizontal_arrows: bool,
    pub(crate) vertical_arrows: bool,
    pub(crate) escape: bool,
}

pub(crate) fn filter(tab: bool, horizontal_arrows: bool, vertical_arrows: bool, escape: bool) -> KeyFilter {
    KeyFilter { tab, horizontal_arrows, vertical_arrows, escape }
}

/// The widget egui holds key focus on.
/// Whether `id` holds egui's keyboard focus.
pub(crate) fn has_focus(ctx: &Context, id: Id) -> bool {
    ctx.memory(|memory| memory.has_focus(id))
}

pub(crate) fn focused(ctx: &Context) -> Option<Id> {
    ctx.memory(|memory| memory.focused())
}

/// Gives `id` key focus with `filter`, applied from this pass: a filter set afterwards acts a pass late.
pub(crate) fn focus_with_filter(ctx: &Context, id: Id, filter: KeyFilter) {
    let filter =
        EventFilter { tab: filter.tab, horizontal_arrows: filter.horizontal_arrows, vertical_arrows: filter.vertical_arrows, escape: filter.escape };
    ctx.memory_mut(|memory| memory.request_focus_with_filter(id, filter));
}

/// Takes key focus from `id` if it holds it.
pub(crate) fn surrender(ctx: &Context, id: Id) {
    ctx.memory_mut(|memory| memory.surrender_focus(id));
}

/// H7: egui's begin pass set a focus direction from an unmodified arrow; the runtime's focus service is the
/// only mover, so the direction is cleared before any component runs.
pub(crate) fn clear_directional_focus(ctx: &Context) {
    ctx.memory_mut(|memory| memory.move_focus(FocusDirection::None));
}

/// The press origin of the running pass's primary press.
pub(crate) fn primary_press_origin(ctx: &Context) -> Option<Pos2> {
    ctx.input(|input| if input.pointer.primary_pressed() { input.pointer.press_origin() } else { None })
}

/// The egui key filter of a text part that keeps `keep`, as the focus service gives it on a focus move.
pub(crate) fn event_filter(keep: crate::runtime::focus::KeepKeys, escape: bool) -> EventFilter {
    use crate::runtime::focus::KeepKeys;
    EventFilter {
        tab: keep == KeepKeys::Tab,
        horizontal_arrows: matches!(keep, KeepKeys::InlineArrows | KeepKeys::BlockArrows),
        vertical_arrows: keep == KeepKeys::BlockArrows,
        escape,
    }
}
