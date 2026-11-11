//! The focus service's fixture: egui widgets for stops, the egui focus, and the events the key and press
//! stages read.

use crate::runtime::egui::{fixture, focus as egui_focus};
use crate::runtime::focus::{self, FocusPolicy};
use crate::runtime::keys;
use crate::runtime::pointer;
use craft_behavior::focus::EscapeStep;
use craft_behavior::{CancelRule, DocKey, HitLevel, HitPriority, PartRef, Sense, Shape};
use craft_keymap::Chord;
use egui::{Context, Event, Id, Key, Modifiers, Pos2, Rect, Ui};
use std::sync::Arc;

pub use crate::runtime::focus::registration::{Group, Stop};
pub use crate::runtime::keys::Intents;

/// Shows a focusable egui widget of `id`, as a component root's sensing core does, and returns its rect.
pub fn widget(ui: &mut Ui, id: Id) -> Rect {
    fixture::focusable_widget(ui, id)
}

/// The widget egui holds key focus on.
pub fn egui_focused(ctx: &Context) -> Option<Id> {
    egui_focus::focused(ctx)
}

/// Registers a shape of `owner` at `rect` on the layer of `ui` that senses a press as `sense` says, as a
/// component's sensing core does; the next pass's presses resolve over it.
pub fn press_shape(ui: &Ui, rect: Rect, sense: Sense, owner: Id) {
    let shape = Shape::new(rect.into(), sense, owner, None, HitPriority::new(HitLevel::Background, 0), 0);
    let layer = fixture::layer_of(ui);
    pointer::record_shapes(ui.ctx(), layer, owner, [shape]);
    pointer::record_widgets(ui.ctx(), layer, owner, &[owner], sense.contains(Sense::CLICK));
}

/// A primary button press or release at `pos`.
pub fn primary_button(pos: Pos2, pressed: bool) -> Event {
    fixture::primary_button(pos, pressed)
}

/// A key release.
pub fn key_release(key: Key, modifiers: Modifiers) -> Event {
    fixture::key_release(key, modifiers)
}

pub use crate::runtime::egui::fixture::AccessActionKind;

/// An AccessKit action request for the node numbered by `target`.
pub fn access_request(kind: AccessActionKind, target: Id) -> Event {
    fixture::access_request(kind, target)
}

/// An AccessKit `SetValue` request of `value` for the node of `target`.
pub fn set_value(target: Id, value: f64) -> Event {
    fixture::access_set_value(target, value)
}

/// A globally unique id named by `name`.
pub fn named_id(name: &str) -> Id {
    fixture::named_id(name)
}

/// Registers `stop` in the innermost open scope of the running pass.
pub fn register_stop(ctx: &Context, stop: Stop) {
    focus::register_stop(ctx, stop);
}

/// Registers a roving group of the running pass.
pub fn register_group(ctx: &Context, group: Group) {
    focus::register_group(ctx, group);
}

/// Opens the scope `id` for the stops registered until the matching [`pop_scope`].
pub fn push_scope(ctx: &Context, id: Id, policy: FocusPolicy, label: Option<Arc<str>>) {
    focus::push_scope(ctx, id, policy, label);
}

/// Marks the stops registered from here on in the innermost open scope as its body's.
pub fn begin_body(ctx: &Context) {
    focus::begin_body(ctx);
}

/// Closes the innermost open scope.
pub fn pop_scope(ctx: &Context) {
    focus::pop_scope(ctx);
}

/// Registers the component that `FocusTarget::Doc(key)` focuses.
pub fn register_doc(ctx: &Context, key: DocKey, id: Id) {
    focus::register_doc(ctx, key, id);
}

/// Whether the focus stop is the component `id`.
pub fn is_focused(ctx: &Context, id: Id) -> bool {
    focus::is_focused(ctx, id)
}

/// `is_focused` with focus visible.
pub fn focus_visible(ctx: &Context, id: Id) -> bool {
    focus::focus_visible(ctx, id)
}

/// The part of `id` that is the focus stop, when one is.
pub fn active_part(ctx: &Context, id: Id) -> Option<PartRef> {
    focus::active_part(ctx, id)
}

/// The text part that commits its draft in this pass because a press moved focus away from it.
pub fn commit_now(ctx: &Context) -> Option<Id> {
    focus::commit_now(ctx)
}

/// The component the last focus request or Tab move asked the enclosing scroll owner to reveal.
pub fn take_reveal(ctx: &Context) -> Option<(Id, Option<PartRef>)> {
    focus::take_reveal(ctx)
}

/// The intents routed to `id` this pass, with horizontal moves read in the component's direction.
pub fn intents_of(ctx: &Context, id: Option<Id>, rtl: bool) -> Intents {
    keys::intents_of(ctx, id, rtl)
}

/// Writes back what a component left untaken.
pub fn settle(ctx: &Context, id: Option<Id>, intents: &Intents) {
    keys::settle(ctx, id, intents);
}

/// Takes an untaken `Submit` for the innermost scope with a default action.
pub fn take_submit(ctx: &Context) -> bool {
    keys::take_submit(ctx)
}

/// What an untaken Escape does in a scope.
pub fn scope_escape(ctx: &Context, rule: CancelRule, press_captured: bool, scope_clearable: bool) -> Option<EscapeStep> {
    keys::scope_escape(ctx, rule, press_captured, scope_clearable)
}

/// Whether the claimed chord arrived this pass for `id`, consuming it.
pub fn take_claimed(ctx: &Context, id: Id, chord: Chord) -> bool {
    keys::take_claimed(ctx, id, chord)
}

/// A state-dependent claim of `owner`.
pub fn claim(ctx: &Context, owner: Id, chord: Chord) {
    keys::claim(ctx, owner, chord);
}

/// The member whose `Activate` the group `root` reads this pass.
pub fn activated_member(ctx: &Context, root: Id) -> Option<Id> {
    keys::activated_member(ctx, root)
}

/// Registers an AccessKit node so a request that targets it reaches `owner`.
pub fn register_node(ctx: &Context, node: Id, owner: Id, part: Option<PartRef>, focusable: bool) {
    keys::register_node(ctx, node, owner, part, focusable);
}

/// The rect `id`'s context request of this pass opens at, when it made one.
pub fn context_request(ctx: &Context, id: Id) -> Option<Rect> {
    keys::context_request(ctx, id)
}
