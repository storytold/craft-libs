//! A component shown inside another as the owner's own control (`PaintUi::inner`): the inner root's node folds into
//! the owner's, as the text widget's node does for a text part. The inner component reports no node of its own and
//! its key focus is the owner's node; the owner's node takes the inner value text when it has none, and sends the
//! requests it receives to the inner component. The inner component's sub-nodes attach to the owner's node.

use crate::runtime::egui::memory as store;
use crate::runtime::memory::sub;
use egui::{Context, Id};

/// The owner the inner component `inner` folds into.
#[derive(Clone, Copy)]
struct Owner(Id);

/// What the inner component tells its owner's report.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Folded {
    /// The inner component: the owner's node stands for its focus stop.
    pub inner: Id,
    /// The inner component's value text.
    pub value: Option<String>,
}

fn owner_slot(inner: Id) -> Id {
    sub(inner, "fold_owner")
}

fn text_slot(widget: Id) -> Id {
    sub(widget, "fold_text")
}

fn folded_slot(owner: Id) -> Id {
    sub(owner, "folded_inner")
}

/// Shows the component `inner` as a control of `owner`.
pub(crate) fn request(ctx: &Context, inner: Id, owner: Id) {
    store::put(ctx, owner_slot(inner), Owner(owner));
}

/// Shows the text widget `widget` as the text control of `owner`.
pub(crate) fn request_text(ctx: &Context, widget: Id, owner: Id) {
    request(ctx, widget, owner);
    store::put(ctx, text_slot(widget), ());
}

/// Whether `id` is a text widget that folds into its owner, as opposed to an inner component.
#[cfg(feature = "testing")]
pub(crate) fn is_text_widget(ctx: &Context, id: Id) -> bool {
    store::peek_clone::<()>(ctx, text_slot(id)).is_some()
}

/// The owner `inner` folds into, if it is shown as an owner's control.
pub(crate) fn owner_of(ctx: &Context, inner: Id) -> Option<Id> {
    store::peek_clone::<Owner>(ctx, owner_slot(inner)).map(|owner| owner.0)
}

/// The inner component's facts for `owner`'s report this pass.
pub(crate) fn offer(ctx: &Context, owner: Id, folded: Folded) {
    store::put(ctx, folded_slot(owner), folded);
}

/// The facts the inner component offered `owner` this pass.
pub(crate) fn take(ctx: &Context, owner: Id) -> Option<Folded> {
    store::take::<Folded>(ctx, folded_slot(owner))
}
