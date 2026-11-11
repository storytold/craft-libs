//! The completion list of a text control (18.2): the popup shows the candidates the text adapter holds, and a click
//! on a row hands the pick back to the adapter, which accepts it in its next run.

use crate::contract::{CollectionEvent, Component, Cx, Overlay, OverlayUi, Overlays as _, ShowCollection as _, WordSetCx as _};
use crate::list::{List, ListRow};
use crate::primitives::TextKey;
use crate::runtime::env::Label;
use craft_behavior::{DisplayOrder, ItemSource};
use std::borrow::Cow;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};

/// The labels of the candidates as a collection source, keyed by position.
struct Candidates<'a> {
    labels: &'a [Label],
}

impl DisplayOrder<usize> for Candidates<'_> {
    fn len(&self) -> usize {
        self.labels.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.labels.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.labels[at].as_str()))
    }
}

impl ItemSource<usize> for Candidates<'_> {}

/// Shows the `completions` popup of a component whose text part is `key`: one option row per candidate. The popup is
/// a dropdown: the host hands the text part's list keys to the list, whose lead is the highlight, and the opener keeps
/// the key focus. An activation (Enter, Tab or a click) picks the candidate the adapter accepts in its next run. A text
/// control that takes completion calls this from its `overlay` hook.
pub(crate) fn show<C: Component, V: 'static>(ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, C>, key: &TextKey<V>) {
    let popup = cx.popup("completions");
    let candidates = cx.text(key).map_or_else(Vec::new, |text| text.completions.clone());
    let labels: Vec<Label> = candidates.iter().map(|candidate| cx.env.resolve(&candidate.label)).collect();
    let name = cx.label();
    // New candidates are new rows: the highlight returns to the first.
    let stamp = {
        let mut hasher = DefaultHasher::new();
        labels.iter().for_each(|label| label.as_str().hash(&mut hasher));
        hasher.finish()
    };
    let events = ui.show(popup, Overlay::dropdown(), |ui, _| {
        List::options(name.clone())
            .rows_stamp(stamp)
            .show_items(ui, &Candidates { labels: &labels }, &mut |at, _| ListRow::new(labels[*at].clone()).option())
            .events
    });
    let picked = events.into_iter().flatten().find_map(|event| match event {
        CollectionEvent::Activate(at) => Some(at),
        _ => None,
    });
    if let Some(at) = picked {
        cx.pick_completion(at);
    }
}
