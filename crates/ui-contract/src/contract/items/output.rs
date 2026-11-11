//! What a collection's caller reads from `Cx` after the item step: its selection and response.

use crate::contract::collection::{CollectionResponse, ItemsOut};
use crate::contract::{Collection, Cx};

impl<C: Collection> Cx<'_, C> {
    /// The selection the item step holds this pass: the app's binding when it has one, else the one the item step
    /// keeps. The collection's own `update` reads it as the pass began; its `finish` reads it with the pass's
    /// selection changes applied.
    pub fn collection_selection(&self) -> Option<&C::Selection> {
        self.core.collection_selection.as_ref().and_then(|held| held.downcast_ref::<C::Selection>())
    }

    /// The output of a collection: its response, every event in drawing order and the positions it showed.
    pub fn collection_response(&mut self) -> CollectionResponse<C::Key, C::ItemEvent> {
        let events = self
            .core
            .collection_events
            .take()
            .and_then(|held| held.downcast::<ItemsOut<C::Key, C::ItemEvent>>().ok())
            .map(|held| *held)
            .unwrap_or_default();
        CollectionResponse::new(self.core.response.clone(), events, self.core.visible_rows.clone(), self.core.gestures)
    }
}
