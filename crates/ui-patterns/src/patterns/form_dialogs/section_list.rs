//! `SectionList`: the "sections beside the shown section" arrangement of the PDF export dialog, the preferences
//! body and the properties dialog's list of tabs.

use crate::contract::{
    ActivateOn, Bind, CollectionEvent, Gap, Pad, Props as _, Scroll, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing,
};
use crate::display::Divider;
use crate::layout_components::{Row, Stack};
use crate::list::{List, ListRow};
use crate::runtime::env::Label;
use crate::style::SurfaceKind;
use craft_behavior::{ItemSource, Selection, SelectionChange};
use egui::Ui;
use std::hash::Hash;

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// What a section list draws under its list, in the list's column.
type Foot<'a> = Box<dyn FnMut(&mut Ui) + 'a>;

/// A single-select, click-activated, content-sized list at its natural height without a header and a vertical divider at the start edge,
/// and the body of the shown section in a filling, vertically scrolled stack beside them; the list stays put while the
/// body scrolls.
///
/// - Keys: the list's own (arrows move, Enter and click pick); a pick is reported in the pass of the pick only.
/// - Body: the caller draws the shown section; the list is not inside the scrolled stack.
#[must_use = "show the list with `show`"]
#[derive(crate::contract::Pattern)]
#[pattern(probe(SectionList<'static, usize>))]
pub struct SectionList<'a, K: Clone + Eq + Hash + Send + Sync + 'static> {
    label: Label,
    source: &'a dyn ItemSource<K>,
    shown: K,
    id: Option<egui::Id>,
    body_label: Option<Label>,
    foot: Option<Foot<'a>>,
    framed: bool,
}

impl<'a, K: Clone + Eq + Hash + Send + Sync + 'static> SectionList<'a, K> {
    /// The list labelled `label` over the sections of `source`, with `shown` picked.
    pub fn new(label: impl Into<Label>, source: &'a dyn ItemSource<K>, shown: K) -> Self {
        SectionList { label: label.into(), source, shown, id: None, body_label: None, foot: None, framed: false }
    }

    /// The list's id, for a search field that drives it (`TextField::drives`).
    pub fn id(mut self, id: egui::Id) -> Self {
        self.id = Some(id);
        self
    }

    /// The accessible label of the body, the shown section's title.
    pub fn body_label(mut self, label: impl Into<Label>) -> Self {
        self.body_label = Some(label.into());
        self
    }

    /// Content under the list, in the list's column.
    pub fn foot(mut self, foot: impl FnMut(&mut Ui) + 'a) -> Self {
        self.foot = Some(Box::new(foot));
        self
    }

    /// Draws the list's column in a frame as tall as the body, with no divider beside it: a desktop preferences
    /// window's page list.
    pub fn framed(mut self) -> Self {
        self.framed = true;
        self
    }

    /// Shows the list with a row per section from `row` and `body` beside it; the section picked this pass.
    // PLAN: ty.X70 gives `body: impl FnOnce`; a container runs its body again for a measuring pass, so `body` and
    // `foot` are `FnMut`.
    pub fn show(self, ui: &mut Ui, mut row: impl FnMut(&K) -> ListRow<'static>, mut body: impl FnMut(&mut Ui)) -> Option<K> {
        let SectionList { label, source, shown, id, body_label, mut foot, framed } = self;
        let mut picked = None;
        // A framed list stands apart from the body; an unframed one has the divider between them.
        let mut columns = Row::new().fill();
        if framed {
            columns = columns.gap(Gap::Section);
        }
        columns.show(ui, |ui, _| {
            let mut list = |ui: &mut Ui| {
                let mut sections = List::<K>::new(label.clone())
                    .select(SelectMode::Single)
                    .activate(ActivateOn::Click)
                    .allow_empty(false)
                    .header(false)
                    .sizing(Sizing::Content)
                    .natural_height(true)
                    .selection(Bind::given(Selection::single(shown.clone())));
                if let Some(id) = id {
                    sections = sections.id(id);
                }
                let listed = sections.show_items(ui, source, &mut |key, _| row(key));
                for event in listed.events {
                    if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
                        && let Some(key) = keys.into_iter().last()
                    {
                        picked = Some(key);
                    }
                }
            };
            if framed {
                Stack::new().surface(SurfaceKind::Frame).pad(Pad::Tight).sizing(Sizing::FillHeight).show(ui, |ui, _| {
                    list(ui);
                    if let Some(foot) = foot.as_mut() {
                        foot(ui);
                    }
                });
            } else {
                match foot.as_mut() {
                    Some(foot) => {
                        Stack::new().show(ui, |ui, _| {
                            list(ui);
                            foot(ui);
                        });
                    }
                    None => list(ui),
                }
                Divider::vertical().show(ui);
            }
            let mut shown_body = Stack::new().fill().scroll(Scroll::vertical());
            if let Some(label) = &body_label {
                shown_body = shown_body.label(label.clone());
            }
            shown_body.show(ui, |ui, _| body(ui));
        });
        picked
    }
}
