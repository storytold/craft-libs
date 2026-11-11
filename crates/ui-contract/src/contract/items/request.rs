//! Requests to a collection's item step from outside it, taken at the collection's next pass.

use crate::runtime::egui::memory as store;
use egui::{Context, Id};

/// The rename requests not yet taken: the collection and the key to rename.
struct RenameRequests<K>(Vec<(Id, K)>);

impl<K> Default for RenameRequests<K> {
    fn default() -> Self {
        RenameRequests(Vec::new())
    }
}

/// Asks the collection `collection` (the id it is built with, `Props::id`) to start renaming the item `key`. At its
/// next item step the lead moves to the item, the item is revealed and its rename field opens, as for the rename
/// key. A request for a key the source lacks, or for a collection that cannot rename the item, is dropped. A second
/// request for the same collection replaces the first.
pub fn request_rename<K: Clone + Send + Sync + 'static>(ctx: &Context, collection: Id, key: &K) {
    if !crate::runtime::measure::effect(ctx, "request_rename") {
        return;
    }
    store::Service::<RenameRequests<K>>::run(ctx, |requests| {
        requests.0.retain(|(id, _)| *id != collection);
        requests.0.push((collection, key.clone()));
    });
    crate::runtime::egui::repaint::request_of(ctx, crate::runtime::egui::pass::current_viewport(ctx));
}

/// The rename request addressed to `collection`, once.
pub(super) fn take_rename<K: Send + Sync + 'static>(ctx: &Context, collection: Id) -> Option<K> {
    store::Service::<RenameRequests<K>>::run(ctx, |requests| {
        let at = requests.0.iter().position(|(id, _)| *id == collection)?;
        Some(requests.0.remove(at).1)
    })
}

/// The lead's position as the item step left it at the end of its last pass.
#[derive(Clone, Copy)]
struct LeadFact(Option<usize>);

fn lead_slot(collection: Id) -> Id {
    crate::runtime::memory::sub(collection, "lead_position")
}

pub(super) fn record_lead(ctx: &Context, collection: Id, at: Option<usize>) {
    let slot = lead_slot(collection);
    store::put(ctx, slot, LeadFact(at));
    crate::runtime::memory::adopt::<LeadFact>(ctx, collection, slot);
}

/// The position of the lead of `collection` after its last item step, `None` while it has none.
#[doc(hidden)]
pub fn lead_position(ctx: &Context, collection: Id) -> Option<usize> {
    store::peek_clone::<LeadFact>(ctx, lead_slot(collection)).and_then(|fact| fact.0)
}
