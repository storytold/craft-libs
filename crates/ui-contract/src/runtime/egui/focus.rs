//! egui focus, `move_focus` and the surrender rule of the focus the runtime gives.

use egui::{Context, EventFilter, FocusDirection, Id, Pos2, SurrenderFocusOn};

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

/// egui's focus candidates of the running pass from the `seen`th on (`Memory::focus_candidates`): the widgets
/// interested in key focus, in the order they registered.
pub(crate) fn candidates_since(ctx: &Context, seen: usize) -> Vec<egui::FocusCandidate> {
    ctx.memory(|memory| memory.focus_candidates().get(seen..).map(<[_]>::to_vec).unwrap_or_default())
}

/// The widget `id` as drawn so far in the running pass; `None` when no widget has the id. egui takes the focus from a
/// widget that is not focusable or not enabled when it is drawn, and from an id no interactive widget has at the pass's
/// end.
pub(crate) fn drawn_widget(ctx: &Context, id: Id) -> Option<egui::WidgetRect> {
    ctx.viewport(|viewport| viewport.this_pass.widgets.get(id).copied())
}

/// Whether the widget egui holds key focus on keeps Tab by its event filter (`Memory::focus_lock_filter`).
pub(crate) fn focused_keeps_tab(ctx: &Context) -> bool {
    ctx.memory(|memory| memory.focus_lock_filter()).is_some_and(|filter| filter.tab)
}

/// Whether egui holds a label text selection, which egui copies on a copy or cut event.
pub(crate) fn label_selection(ctx: &Context) -> bool {
    ctx.with_plugin::<egui::text_selection::LabelSelectionState, _>(|state| state.has_selection()).unwrap_or(false)
}

/// Gives `id` key focus with `filter`, applied from this pass: a filter set afterwards acts a pass late. egui never
/// takes this focus away on a press or click elsewhere: the runtime moves it on press (8.3). The end hook leaves
/// the focus of an app's own widget to egui's rule (`leave_to_egui`).
pub(crate) fn focus_with_filter(ctx: &Context, id: Id, filter: KeyFilter) {
    let filter =
        EventFilter { tab: filter.tab, horizontal_arrows: filter.horizontal_arrows, vertical_arrows: filter.vertical_arrows, escape: filter.escape };
    ctx.memory_mut(|memory| {
        memory.request_focus_with_filter(id, filter);
        memory.set_focus_surrender(id, SurrenderFocusOn::Never);
    });
}

/// Keeps egui from taking the focus of `id`, if it holds it, on a press or click elsewhere, until the focus moves or a
/// request gives it again.
pub(crate) fn never_surrender(ctx: &Context, id: Id) {
    ctx.memory_mut(|memory| memory.set_focus_surrender(id, SurrenderFocusOn::Never));
}

/// Leaves the focus of the app's own widget `id`, if it holds it, to egui's rule, the app's `surrender_focus_on`, with
/// the key filter it has.
pub(crate) fn leave_to_egui(ctx: &Context, id: Id) {
    ctx.memory_mut(|memory| {
        if memory.has_focus(id) {
            let filter = memory.focus_lock_filter().unwrap_or_default();
            memory.request_focus_with_filter(id, filter);
        }
    });
}

/// Gives a component's widget `id` back the focus egui dropped in this pass (egui drops the focus of a disabled
/// widget), as the runtime's focus: egui never surrenders it on a press or click elsewhere.
pub(crate) fn keep_focus(memory: &mut egui::Memory, id: Id) {
    memory.request_focus(id);
    memory.set_focus_surrender(id, SurrenderFocusOn::Never);
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
