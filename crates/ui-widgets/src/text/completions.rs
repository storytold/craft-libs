//! The completion list of a text control (18.2): the popup shows the candidates the text adapter holds, and a click
//! on a row hands the pick back to the adapter, which accepts it in its next run.

use crate::contract::{
    CollectionEvent, Component, Cx, Overlay, OverlayUi, Overlays as _, Show as _, ShowCollection as _, WordSetCx as _, lead_position,
};
use crate::display::{Divider, Text};
use crate::list::{List, ListRow};
use crate::primitives::TextKey;
use crate::runtime::env::Label;
use crate::style::TypeRole;
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

/// Shows the `completions` popup of a component whose text part is `key`: one option row per candidate, its mark before
/// the label, and under the rows, past a divider, the highlighted candidate's description. The popup is a dropdown: the
/// host hands the text part's list keys to the list, whose lead is the highlight, and the opener keeps the key focus.
/// An activation (Tab, a click, and Enter with `enter`) picks the candidate the adapter accepts in its next run; without
/// `enter` the field keeps Enter. A text control that takes completion calls this from its `overlay` hook.
pub(crate) fn show<C: Component, V: 'static>(ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, C>, key: &TextKey<V>, enter: bool) {
    let popup = cx.popup("completions");
    let candidates = cx.text(key).map_or_else(Vec::new, |text| text.completions.clone());
    let labels: Vec<Label> = candidates.iter().map(|candidate| cx.env.resolve(&candidate.label)).collect();
    let marks: Vec<Option<Label>> = candidates.iter().map(|candidate| candidate.mark.as_ref().map(|mark| cx.env.resolve(mark))).collect();
    let descriptions: Vec<Option<Label>> =
        candidates.iter().map(|candidate| candidate.description.as_ref().map(|text| cx.env.resolve(text))).collect();
    // The mark slot is as wide as the widest mark, so the labels line up.
    let widest = marks.iter().flatten().max_by_key(|mark| mark.as_str().chars().count()).cloned();
    let name = cx.label();
    // New candidates are new rows: the highlight returns to the first.
    let stamp = {
        let mut hasher = DefaultHasher::new();
        labels.iter().for_each(|label| label.as_str().hash(&mut hasher));
        hasher.finish()
    };
    let focus = Overlay::dropdown().focus.list_enter(enter);
    let overlay = Overlay::dropdown().focus(focus);
    let events = ui.show(popup, overlay, |ui, _| {
        let list = List::options(name.clone()).rows_stamp(stamp).show_items(ui, &Candidates { labels: &labels }, &mut |at, _| {
            let row = ListRow::new(labels[*at].clone()).option();
            match (&marks[*at], &widest) {
                (Some(mark), Some(widest)) => row.lead_text(mark.clone(), widest.clone()),
                (None, Some(widest)) => row.lead_text(Label::Empty, widest.clone()),
                _ => row,
            }
        });
        let lead = lead_position(ui.ctx(), list.response.id);
        if let Some(description) = lead.and_then(|at| descriptions.get(at)).and_then(Option::as_ref) {
            Divider::horizontal().show(ui);
            Text::new(description.clone()).role(TypeRole::Caption).show(ui);
        }
        list.events
    });
    let picked = events.into_iter().flatten().find_map(|event| match event {
        CollectionEvent::Activate(at) => Some(at),
        _ => None,
    });
    if let Some(at) = picked {
        cx.pick_completion(at);
    }
}
