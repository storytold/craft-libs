//! `PresetFooter`: the actions, the thumbnail size slider and the count of a preset panel's footer.

use super::words::PresetWords;
use crate::contract::{Bind, EditResponse, ListDrag, NoValue, Outcome, Show as _, ShowContainer as _};
use crate::display::Text;
use crate::layout::RowAlign;
use crate::layout_components::Row;
use crate::runtime::env::Env;
use crate::sections::{FooterAction, FooterBar};
use crate::tile_grid::{ThumbnailSlider, TileZoom};
use craft_behavior::IconId;
use egui::{Response, Ui};
use std::marker::PhantomData;
use std::sync::Arc;

#[cfg(feature = "testing")]
mod stories;

/// What a [`PresetFooter`] reports. A browser's footer reports these as `PresetEvent::Footer`, in order with its
/// other events.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PresetFooterEvent<K> {
    /// The New button.
    New,
    /// The New Group button.
    NewGroup,
    /// The Delete button.
    Delete,
    /// A host action added with [`PresetFooter::action`] was pressed; `index` counts the host actions in the order
    /// given.
    #[non_exhaustive]
    Custom {
        /// The action's index.
        index: usize,
    },
    /// Presets or groups dragged from a list were dropped on a host action that accepts drops.
    #[non_exhaustive]
    CustomDrop {
        /// The action's index among the host actions.
        index: usize,
        /// The dragged keys.
        keys: Vec<K>,
    },
}

/// What a [`PresetFooter`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PresetFooterOutput<K> {
    /// The footer row's response.
    pub response: Response,
    /// What was pressed or dropped this pass, in order.
    pub events: Vec<PresetFooterEvent<K>>,
    /// The size slider's edit; `None` when the slider is off.
    pub zoom: Option<EditResponse<f64>>,
}

impl<K> Outcome for PresetFooterOutput<K> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        match &self.zoom {
            Some(zoom) => crate::testing::StoryOutput::of_edit(&self.response, &zoom.events),
            None => crate::testing::StoryOutput::of_edit(&self.response, &craft_behavior::EditEvents::<f64>::new()),
        }
    }
}

type Leading<'a> = Arc<dyn Fn(&mut Ui) + 'a>;

/// The host's choices for a preset footer: the pieces it shows and the actions it adds. A plain value, so the footer
/// can be built from it each time its region runs.
#[derive(Clone)]
#[must_use]
pub struct PresetFooterOptions<'a> {
    slider: bool,
    leading: Option<Leading<'a>>,
    can_delete: bool,
    new_group: bool,
    actions: Vec<FooterAction>,
}

impl Default for PresetFooterOptions<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> PresetFooterOptions<'a> {
    /// The footer with the slider, Delete, New and New Group.
    pub fn new() -> Self {
        PresetFooterOptions { slider: true, leading: None, can_delete: true, new_group: true, actions: Vec::new() }
    }

    /// Whether the size slider is shown (default yes).
    pub fn slider(mut self, slider: bool) -> Self {
        self.slider = slider;
        self
    }

    /// The host's own controls before the slider (a "Current Tool Only" check box). The footer runs it each time its
    /// region runs.
    pub fn leading(mut self, add: impl Fn(&mut Ui) + 'a) -> Self {
        self.leading = Some(Arc::new(add));
        self
    }

    /// Whether Delete can be clicked, which is when something is selected (default yes).
    pub fn can_delete(mut self, can_delete: bool) -> Self {
        self.can_delete = can_delete;
        self
    }

    /// Whether the New Group button is shown (default yes).
    pub fn new_group(mut self, new_group: bool) -> Self {
        self.new_group = new_group;
        self
    }

    /// A host action before New Group, reported as [`PresetFooterEvent::Custom`] with its index.
    pub fn action(mut self, action: FooterAction) -> Self {
        self.actions.push(action);
        self
    }
}

/// The footer of a preset panel: the host's own controls, the thumbnail size slider and the count, then the action
/// buttons Delete, New and New Group and the host's own actions.
///
/// - **Buttons.** Delete (disabled by `can_delete(false)`), New, New Group
///   (left out by `new_group(false)`), then the host's [`FooterAction`]s. Their names come from [`PresetWords`].
/// - **Host actions.** A disabled action takes no click. An action that `accepts_drops` takes presets dragged from
///   any preset list and reports [`PresetFooterEvent::CustomDrop`].
/// - **Slider.** Edits the zoom binding within `range` (the ladder by default); omitted by `slider(false)`.
/// - **Options.** The host's choices come as one [`PresetFooterOptions`] value.
#[must_use = "show the footer with `show`"]
#[derive(crate::contract::Pattern)]
#[pattern(probe(PresetFooter<'static, u32>))]
pub struct PresetFooter<'a, K> {
    zoom: Bind<'a, f64>,
    range: TileZoom,
    count: Option<usize>,
    options: PresetFooterOptions<'a>,
    keys: PhantomData<fn() -> K>,
}

impl<'a, K> PresetFooter<'a, K> {
    /// A footer whose slider edits the thumbnail zoom `zoom` (`PresetState::zoom`).
    pub fn new(zoom: Bind<'a, f64>) -> Self {
        PresetFooter { zoom, range: TileZoom::Ladder, count: None, options: PresetFooterOptions::new(), keys: PhantomData }
    }

    /// How the slider steps and what it spans; use the browser's.
    pub fn range(mut self, range: TileZoom) -> Self {
        self.range = range;
        self
    }

    /// Shows the number of presets after the slider.
    pub fn count(mut self, count: usize) -> Self {
        self.count = Some(count);
        self
    }

    /// The host's pieces and actions.
    pub fn options(mut self, options: PresetFooterOptions<'a>) -> Self {
        self.options = options;
        self
    }
}

/// Which footer button an index of the shown actions is.
#[derive(Clone, Copy)]
enum Slot {
    Custom(usize),
    NewGroup,
    New,
    Delete,
}

impl<K: Clone + Send + Sync + 'static> PresetFooter<'_, K> {
    /// Shows the footer across the available width and reports what was pressed, in order.
    pub fn show(self, ui: &mut Ui) -> PresetFooterOutput<K> {
        let PresetFooter { mut zoom, range, count, options, .. } = self;
        let PresetFooterOptions { slider, leading, can_delete, new_group, actions } = options;
        let env = Env::of(ui);
        let words = env.words::<PresetWords>();
        let mut shown = Vec::with_capacity(actions.len() + 3);
        let mut slots = Vec::with_capacity(actions.len() + 3);
        for (index, action) in actions.into_iter().enumerate() {
            shown.push(action);
            slots.push(Slot::Custom(index));
        }
        if new_group {
            shown.push(FooterAction::new(IconId::new("new_folder"), words.new_group.clone()));
            slots.push(Slot::NewGroup);
        }
        shown.push(FooterAction::new(IconId::new("plus"), words.new_preset.clone()));
        slots.push(Slot::New);
        shown.push(FooterAction::new(IconId::new("trash"), words.delete_button.clone()).enabled(can_delete));
        slots.push(Slot::Delete);

        let row = Row::new().fill().align(RowAlign::End).show(ui, |ui, _| {
            if let Some(add) = &leading {
                add(ui);
            }
            let edit = slider.then(|| ThumbnailSlider::new(words.thumb_size.clone(), zoom.reborrowed(), range).show(ui));
            if let Some(n) = count {
                Text::new(words.count.text(n as u64, &env.format().integer(n as u64))).show(ui);
            }
            (edit, FooterBar::actions(shown.clone()).show_footer(ui))
        });
        let (zoom_edit, footer) = row.inner.unzip();
        let mut events = Vec::new();
        if let Some(footer) = footer {
            if let Some(drop) = &footer.dropped {
                let keys = drop.payload::<ListDrag<K>>().map(|drag| drag.keys.clone()).unwrap_or_default();
                if let Some(Slot::Custom(index)) = slots.get(drop.index) {
                    events.push(PresetFooterEvent::CustomDrop { index: *index, keys })
                }
            } else if let Some(slot) = footer.picked.and_then(|at| slots.get(at)) {
                events.push(match *slot {
                    Slot::Custom(index) => PresetFooterEvent::Custom { index },
                    Slot::NewGroup => PresetFooterEvent::NewGroup,
                    Slot::New => PresetFooterEvent::New,
                    Slot::Delete => PresetFooterEvent::Delete,
                });
            }
        }
        PresetFooterOutput { response: row.output, events, zoom: zoom_edit.flatten() }
    }
}
