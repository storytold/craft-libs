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

/// Whether an egui popup of the running viewport is open: an app's own popup or menu, which reads its keys from egui.
pub(crate) fn popup_open(ctx: &Context) -> bool {
    egui::Popup::is_any_open(ctx)
}

/// Whether egui holds a label text selection, which egui copies on a copy or cut event.
pub(crate) fn label_selection(ctx: &Context) -> bool {
    ctx.with_plugin::<egui::text_selection::LabelSelectionState, _>(|state| state.has_selection()).unwrap_or(false)
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

/// H7: egui's begin pass set a focus direction from an unmodified arrow or the Tab it kept. The focus service clears
/// it before any component runs unless egui focus is on an app's own widget (`FocusedRecord::host_focus`) and the
/// service moves no focus in the pass: egui would otherwise move focus off a framework stop that the same arrow
/// reaches as an intent.
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
