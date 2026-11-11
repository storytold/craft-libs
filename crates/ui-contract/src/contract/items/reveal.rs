//! Bringing a row into view: a requested reveal and the scroll that does it.

use super::ItemsHooks;
use crate::contract::Collection;
use crate::contract::collection::LeadSource;
use craft_behavior::{ItemLayout, ScrollPos};

impl<C: Collection> ItemsHooks<'_, '_, C> {
    /// The item the collection was asked to reveal (`Collection::reveal`, a one-shot request of the pass): the lead
    /// moves to it and it scrolls into view.
    pub(super) fn requested_reveal(&mut self, comp: &C, layout: &C::ItemLayout, view: f32) {
        if let Some(at) = comp.reveal().and_then(|key| self.index_of(key)).filter(|&at| !self.source.section_stop(at)) {
            self.move_lead(at, LeadSource::Keys);
            self.reveal_with(layout, at, view);
        }
    }

    /// Brings row `at` into view: the collection's own scroll moves, or, without one, the page's scroll owner is asked
    /// at the end of the item step (`page_reveal`).
    pub(super) fn reveal_with(&mut self, layout: &C::ItemLayout, at: usize, view: f32) {
        if !layout.shown(at) {
            return;
        }
        if !self.scrolls {
            self.page_reveal = Some(at);
            return;
        }
        let rect = layout.rect(at, ScrollPos { x: 0.0, y: self.offset });
        if rect.min.y < 0.0 {
            self.offset += f64::from(rect.min.y);
        } else if rect.max.y > view {
            self.offset += f64::from(rect.max.y - view);
        }
        self.offset = self.offset.clamp(0.0, self.max_offset(view));
    }
}
