//! What a collection's caller reads from `Cx` after the item step: its selection and response.

use crate::contract::collection::{CollectionResponse, ColumnRequest, HeaderMenuRequest, ItemsOut};
use crate::contract::{Collection, ColumnCommands, Cx};
use craft_behavior::ColumnKey;

impl<C: Collection> Cx<'_, C> {
    /// The selection the item step holds this pass: the app's binding when it has one, else the one the item step
    /// keeps. The collection's own `update` reads it as the pass began; its `finish` reads it with the pass's
    /// selection changes applied.
    pub fn collection_selection(&self) -> Option<&C::Selection> {
        self.core.collection_selection.as_ref().and_then(|held| held.downcast_ref::<C::Selection>())
    }

    /// The header menu, from the item step's header: the request while the menu may be open (a secondary click on a
    /// header cell or the empty header, the Menu key on a focused header) and whether it was made in this pass. The
    /// collection that shows the menu opens its popup on a fresh request and gives the picks back with
    /// `column_request`.
    pub fn header_menu(&self) -> Option<(&HeaderMenuRequest, bool)> {
        self.core.header.menu.as_ref().map(|menu| (menu, self.core.header.asked))
    }

    /// The columns the user hid from the header menu; the header leaves them out.
    pub fn hidden_columns(&self) -> &[ColumnKey] {
        &self.core.header.hidden
    }

    /// Whether the header command `command` can act on the header menu's column.
    pub fn column_command_enabled(&self, command: ColumnCommands) -> bool {
        self.core.header.enabled.contains(&command)
    }

    /// Gives a header menu pick back to the header, which applies it at the end of this pass and reports what changed.
    pub fn column_request(&mut self, request: ColumnRequest) {
        self.core.column_requests.push(request);
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
