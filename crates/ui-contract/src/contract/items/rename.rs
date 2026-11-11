//! Renaming an item: the rename field opened by the rename command or by `request_rename`.

use super::request;
use super::{ItemsHooks, Renaming};
use crate::contract::collection::{CollectionEvent, CollectionProps, LeadSource, Rename};
use crate::contract::{Collection, Cx};
use craft_behavior::{EditEvent, Mixed};

impl<C: Collection> ItemsHooks<'_, '_, C> {
    /// Opens the rename field of the item at `at`.
    pub(super) fn begin_rename(&mut self, cx: &mut Cx<'_, C>, at: usize) {
        let key = self.source.key(at);
        let draft = self.source.text(at).map(|text| text.into_owned()).unwrap_or_default();
        let gesture = crate::runtime::edit::next_gesture_id(&cx.core.ctx);
        let before = Rename { key: key.clone(), name: draft.clone() };
        self.events.push(CollectionEvent::Rename(EditEvent::begin(gesture, Mixed::Same(before), cx.core.label_arc(), None)));
        self.renaming = Renaming { key: Some(key), draft, gesture, focused: false };
    }

    /// A rename requested from outside (`request_rename`): the lead moves to the item, the item is revealed and its
    /// rename field opens.
    pub(super) fn requested_rename(&mut self, comp: &C, cx: &mut Cx<'_, C>, props: &CollectionProps, layout: &C::ItemLayout, view: f32) {
        let Some(key) = request::take_rename::<C::Key>(&cx.core.ctx, cx.core.id) else { return };
        if let Some(at) = self.index_of(&key)
            && !self.source.section_stop(at)
            && props.rename
            && comp.can_rename(&key)
        {
            self.move_lead(at, LeadSource::Keys);
            self.reveal_with(layout, at, view);
            self.begin_rename(cx, at);
        }
    }
}
