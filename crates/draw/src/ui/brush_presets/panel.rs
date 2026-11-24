//! `BrushesPanel`: the Brushes panel, a Size field over a preset browser.

use std::hash::Hash;

use craft_behavior::{EditEvents, IconId, Selection};
use craft_ui::contract::{Bind, EditResponse, MemoProps, Outcome, Props as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::patterns::presets::{PresetBrowser, PresetEvent, PresetFooterEvent, PresetFooterOptions, PresetReorder, PresetSource, PresetState};
use craft_ui::runtime::drag::DropZone;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::sections::FooterAction;
use craft_ui::tile_grid::TileZoom;
use egui::{Response, Ui};

use super::picker::BrushPick;
use super::settings::BrushPickerSettings;
use super::words::BrushPresetWords;
use crate::ui::brush_settings::BrushSizeField;

/// How far one key press changes the thumbnail scale.
const SCALE_STEP: f64 = 0.05;

/// The thumbnail zoom range of the brush lists: the picker and the panel share it.
pub(super) const BRUSH_THUMBS: TileZoom =
    TileZoom::bounded(*BrushPickerSettings::SCALE_RANGE.start() as f64, *BrushPickerSettings::SCALE_RANGE.end() as f64, SCALE_STEP, 1.0);

/// The Brushes panel: a Size field over a [`PresetBrowser`] of the brush presets, with Import as the footer's one
/// action.
#[must_use = "show the panel with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
#[pattern(probe(BrushesPanel<'static, u32>))]
pub struct BrushesPanel<'a, K: Clone + Eq + Hash + Send + Sync + MemoProps + 'static> {
    label: Label,
    source: &'a PresetSource<'a, K>,
    state: Bind<'a, PresetState<K>>,
    diameter: Bind<'a, f64>,
    max_size: f64,
    /// The selected preset, which the host keeps.
    #[prop]
    selected: Option<K>,
    /// Where files dropped on the panel go.
    #[prop]
    drop_zone: Option<DropZone>,
    /// Shows the footer's Import action.
    #[prop]
    import: bool,
}

/// What [`BrushesPanel::show`] returns.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the panel's edits and picks"]
pub struct BrushesPanelOutput<K> {
    /// The panel.
    pub response: Response,
    /// The Size field's edit.
    pub diameter: EditResponse<f64>,
    /// The picks, in the order they happened.
    pub picks: Vec<BrushPick<K>>,
}

impl<K> Outcome for BrushesPanelOutput<K> {
    type Value = craft_ui::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl<'a, K: Clone + Eq + Hash + Send + Sync + MemoProps + 'static> BrushesPanel<'a, K> {
    /// A panel named `label` over the presets of `source`, with its view in `state` and the brush
    /// size in `diameter` up to `max_size`.
    pub fn new(
        label: impl Into<Label>,
        source: &'a PresetSource<'a, K>,
        state: Bind<'a, PresetState<K>>,
        diameter: Bind<'a, f64>,
        max_size: f64,
    ) -> Self {
        BrushesPanel { label: label.into(), source, state, diameter, max_size, selected: None, drop_zone: None, import: false }
    }

    /// Shows the panel.
    pub fn show(self, ui: &mut Ui) -> BrushesPanelOutput<K> {
        let BrushesPanel { label, source, mut state, mut diameter, max_size, mut selected, mut drop_zone, import } = self;
        let words = Env::of(ui).words::<BrushPresetWords>();
        let shown = Stack::new().label(label.clone()).fill().show(ui, |ui, _| {
            let size = BrushSizeField::new(words.size.clone(), diameter.reborrowed(), max_size).show(ui);
            let mut footer = PresetFooterOptions::new().new_group(false);
            if import {
                footer = footer.action(FooterAction::new(IconId::new("folder_open"), words.menu_import.clone()));
            }
            let mut browser = PresetBrowser::new(label.clone(), state.reborrowed(), source)
                .toolbar(true)
                .reorder(PresetReorder::items_and_groups())
                .thumb_range(BRUSH_THUMBS)
                .footer(footer);
            if let Some(key) = selected.take() {
                browser = browser.selection(Bind::given(Selection::single(key)));
            }
            if let Some(zone) = drop_zone.take() {
                browser = browser.drop_zone(zone);
            }
            let listed = browser.show(ui);
            (size.edit, listed.events)
        });
        let response = shown.output;
        let (diameter, events) = shown.inner.unwrap_or_else(|| (EditResponse::lifted(response.clone(), EditEvents::new(), None), Vec::new()));
        BrushesPanelOutput { response, diameter, picks: picks_of(events) }
    }
}

/// Maps the browser's events to picks: `Select` and `Activate` to their picks, the footer's New to
/// `NewPreset`, the footer's one custom action to `Import`, every other event to `List`.
pub(super) fn picks_of<K>(events: Vec<PresetEvent<K>>) -> Vec<BrushPick<K>> {
    events
        .into_iter()
        .map(|event| match event {
            PresetEvent::Select { key, .. } => BrushPick::Select(key),
            PresetEvent::Activate { key, .. } => BrushPick::Activate(key),
            PresetEvent::Footer(PresetFooterEvent::New) => BrushPick::NewPreset,
            PresetEvent::Footer(PresetFooterEvent::Custom { index: 0, .. }) => BrushPick::Import,
            other => BrushPick::List(other),
        })
        .collect()
}
