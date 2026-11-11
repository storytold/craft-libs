//! The Brush Preset picker ([`BrushPresetPicker`]): a closed field that opens a popover with the brush's Size and
//! Hardness, the buttons and gear menu, and the preset browser.

use std::hash::Hash;

use craft_behavior::{EditEvent, EditEvents, IconId, Mixed, Value};
use craft_ui::buttons::IconButton;
use craft_ui::contract::{Bind, BodyCx, EditResponse, Outcome, Props as _, Show as _, ShowContainer as _, Trigger, clicked};
use craft_ui::form::SliderField;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::patterns::presets::{CellShape, PickerField, PresetEvent, PresetFooterOptions, PresetPicker, PresetRow, PresetState, PresetView};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::tile_grid::TileZoom;
use egui::{Response, Ui};

use super::settings::{BrushCardPart, BrushCardParts, BrushPickerSettings};
use super::words::BrushPresetWords;
use crate::ui::brush_settings::BrushSizeField;

/// The largest diameter the picker's Size row allows until the app sets its engine's limit.
const DEFAULT_MAX_DIAMETER: f64 = 5000.0;
/// How far one key press changes the thumbnail scale.
const SCALE_STEP: f64 = 0.05;

/// The brush values the picker's head edits, given by the app each pass.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct BrushPickerValues {
    /// The diameter in pixels: the Size field and its log slider.
    pub diameter: f64,
    /// The hardness, 0 to 1, shown in percent; `None` for a tip without one (a sampled tip): then no Hardness
    /// slider shows.
    pub hardness: Option<f64>,
}

impl BrushPickerValues {
    /// A round tip: its `diameter` and `hardness`.
    pub const fn new(diameter: f64, hardness: Option<f64>) -> Self {
        Self { diameter, hardness }
    }
}

/// What the picker asks the app to do beyond the size and hardness edits.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum BrushPick<K> {
    /// A click on a preset: the binding holds it and the picker stays open.
    Select(K),
    /// A double click or Enter on a preset: the binding holds it and the picker closes.
    Activate(K),
    /// A browser event: `Rename`, `Delete` (also from the gear), `Reorder`, `MoveTo` and the footer's buttons.
    List(PresetEvent<K>),
    /// Rename Brush… in the gear: the app asks for the new name of the current preset.
    Rename(K),
    /// Brush Settings…: open the Brush Settings panel.
    OpenSettings,
    /// New Brush Preset (the button or the gear): open the app's New Brush Preset dialog.
    NewPreset,
    /// Import Brushes… (the gear, shown only with [`BrushPresetPicker::import`]).
    Import,
}

impl<K> BrushPick<K> {
    /// Whether this pick closed the picker: `Activate`, and every pick that opens another panel or dialog (Brush
    /// Settings, New Brush Preset, Import).
    pub fn closes(&self) -> bool {
        matches!(self, BrushPick::Activate(_) | BrushPick::OpenSettings | BrushPick::NewPreset | BrushPick::Import)
    }
}

/// The picker's state, kept by the host while the app runs: the browser's view (search, open groups, scale) and the
/// parts the presets show. [`settings`](Self::settings) is what persists.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct BrushPickerState<K> {
    /// The browser's view settings; its zoom is the thumbnail scale.
    pub browser: PresetState<K>,
    /// Which parts the presets show.
    pub parts: BrushCardParts,
}

impl<K> Default for BrushPickerState<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K> BrushPickerState<K> {
    /// Every part on, scale 1, an empty search.
    pub fn new() -> Self {
        Self { browser: PresetState::new(), parts: BrushCardParts::ALL }
    }
}

impl<K: Eq + Hash> PartialEq for BrushPickerState<K> {
    fn eq(&self, other: &Self) -> bool {
        self.browser == other.browser && self.parts == other.parts
    }
}

/// What [`BrushPresetPicker::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the picker's edits and picks"]
pub struct BrushPresetPickerOutput<K> {
    /// The closed field.
    pub response: Response,
    /// The field's value and its edit; `None` when the body did not run.
    pub edit: Option<EditResponse<Option<K>>>,
    /// The Size row: the new diameter, whole pixels from 1 to the maximum. A drag is one gesture; a typed value is a
    /// `Set`. The app sends it to its tool options host.
    pub diameter: EditEvents<f64>,
    /// The Hardness slider: the new hardness, 0 to 1 in whole percent.
    pub hardness: EditEvents<f64>,
    /// The picks, in the order they happened.
    pub picks: Vec<BrushPick<K>>,
    /// The persisted view when the user changed it this pass (a part box, the scale): the app saves it.
    pub settings: Option<BrushPickerSettings>,
}

impl<K: Value> Outcome for BrushPresetPickerOutput<K> {
    type Value = Option<K>;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        match &self.edit {
            Some(edit) => edit.story_facts(),
            None => craft_ui::testing::StoryOutput::default(),
        }
    }
}

/// Photoshop's Brush Preset picker: a closed field showing the current preset's tip and the brush size, which opens
/// a popover over [`PresetPicker`].
///
/// - **Head.** [`BrushSizeField`] (Size, 1 to [`max_diameter`](Self::max_diameter), log slider) and, while
///   [`BrushPickerValues::hardness`] is set, a Hardness slider field (0 to 100 %). They edit the brush live and report
///   [`BrushPresetPickerOutput::diameter`] and [`hardness`](BrushPresetPickerOutput::hardness). Under them the Brush
///   Settings and New Brush Preset buttons and the gear.
/// - **Gear menu.** New Brush Preset…; Rename Brush… and Delete Brush (enabled with a current preset); the Brush
///   Name, Brush Stroke and Brush Tip boxes (at least one stays on); Import Brushes… with
///   [`import`](Self::import).
/// - **Presets.** The browser with its search field (any letter case, groups open while filtering) and the scale
///   slider (0.15 to 2.0) in its footer. The tip alone is a grid with the size as each caption; the name or the stroke
///   is a list, with the stroke band while the stroke shows. A click picks and keeps the picker open; a double click
///   or Enter picks and closes it.
/// - **Closing.** The popover's own: a press outside, Escape, and the picks that open something else
///   ([`BrushPick::closes`]).
#[must_use = "show the picker with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
#[pattern(probe(BrushPresetPicker<'static, u32>))]
pub struct BrushPresetPicker<'a, K: Value + Eq + Hash> {
    label: Label,
    current: Bind<'a, Option<K>>,
    rows: &'a [PresetRow<K>],
    state: Bind<'a, BrushPickerState<K>>,
    values: BrushPickerValues,
    /// The largest diameter the app's engine paints; 5000 until set.
    #[prop(quantity = "pixels")]
    max_diameter: f64,
    /// Shows Import Brushes… in the gear menu.
    #[prop(flag)]
    import: bool,
}

impl<'a, K: Value + Eq + Hash> BrushPresetPicker<'a, K> {
    /// A picker named `label` over the presets in `rows` (build each with [`brush_row`](super::brush_row)), holding the
    /// preset the brush was picked from in `current`, its view in `state` and the brush's `values`.
    pub fn new(
        label: impl Into<Label>,
        current: Bind<'a, Option<K>>,
        rows: &'a [PresetRow<K>],
        state: Bind<'a, BrushPickerState<K>>,
        values: BrushPickerValues,
    ) -> Self {
        Self { label: label.into(), current, rows, state, values, max_diameter: DEFAULT_MAX_DIAMETER, import: false }
    }

    /// Shows the picker and returns this pass's edits and picks, in order.
    pub fn show(self, ui: &mut Ui) -> BrushPresetPickerOutput<K> {
        let BrushPresetPicker { label, mut current, rows, mut state, values, max_diameter, import } = self;
        let env = Env::of(ui);
        let words = env.words::<BrushPresetWords>();
        let name: std::sync::Arc<str> = std::sync::Arc::from(label.as_str());
        let shown = Stack::new().label(label.clone()).show(ui, |ui, cx| {
            let before = match state.reborrow().read(cx) {
                Some(Mixed::Same(held)) => held,
                _ => BrushPickerState::new(),
            };
            let parts = before.parts.normalised();
            let held = match current.reborrow().read(cx) {
                Some(Mixed::Same(held)) => held,
                _ => None,
            };
            let has_current = held.as_ref().is_some_and(|key| rows.iter().any(|row| !row.is_group() && row.key == *key));
            let mut browser = before.browser.clone();
            let mut next_parts = parts;
            let mut head_picks: Vec<BrushPick<K>> = Vec::new();
            let (mut diameter, mut hardness) = (EditEvents::new(), EditEvents::new());
            let head = |ui: &mut Ui, head_cx: &mut BodyCx<'_>| {
                head_picks.clear();
                next_parts = parts;
                let size = BrushSizeField::new(words.size.clone(), Bind::given(values.diameter), max_diameter).show(ui);
                diameter = size.edit.map(|edit| edit.events).unwrap_or_default();
                hardness = EditEvents::new();
                if let Some(value) = values.hardness {
                    let field = SliderField::new(words.hardness.clone(), Bind::given(value * 100.0), 0.0..=100.0)
                        .step(1.0)
                        .unit_kind(UnitKind::Percent)
                        .show(ui);
                    hardness = field.events.map(|percent| (percent / 100.0).clamp(0.0, 1.0));
                }
                Row::new().show(ui, |ui, _| {
                    let icon = |name: &'static str| IconSpec::svg(IconId::new(name));
                    let settings = IconButton::new(icon("adjustments")).label(words.settings.clone()).tooltip(words.settings.clone()).show(ui);
                    if clicked(&settings) {
                        head_picks.push(BrushPick::OpenSettings);
                    }
                    let new_preset = IconButton::new(icon("plus")).label(words.new_preset.clone()).tooltip(words.new_preset.clone()).show(ui);
                    if clicked(&new_preset) {
                        head_picks.push(BrushPick::NewPreset);
                    }
                    let gear = IconButton::new(icon("settings")).label(words.options.clone()).tooltip(words.options.clone()).show(ui);
                    let _ = Menu::new(words.options.clone()).opened_by(&gear, Trigger::Click).show(ui, |ui, _| {
                        if MenuItem::new(words.menu_new.clone()).show(ui).ran {
                            head_picks.push(BrushPick::NewPreset);
                        }
                        if MenuItem::new(words.menu_rename.clone()).enabled(has_current).show(ui).ran
                            && let Some(key) = held.clone()
                        {
                            head_picks.push(BrushPick::Rename(key));
                        }
                        if MenuItem::new(words.menu_delete.clone()).enabled(has_current).show(ui).ran
                            && let Some(key) = held.clone()
                        {
                            head_picks.push(BrushPick::List(PresetEvent::Delete(key)));
                        }
                        for (part, label) in [
                            (BrushCardPart::Name, &words.part_name),
                            (BrushCardPart::Stroke, &words.part_stroke),
                            (BrushCardPart::Tip, &words.part_tip),
                        ] {
                            let on = next_parts.get(part);
                            if MenuItem::new(label.clone()).check(on).keep_open(true).show(ui).ran {
                                next_parts = next_parts.set(part, !on);
                            }
                        }
                        if import && MenuItem::new(words.menu_import.clone()).show(ui).ran {
                            head_picks.push(BrushPick::Import);
                        }
                    });
                });
                if head_picks.iter().any(BrushPick::closes) {
                    head_cx.request_close();
                }
            };
            let picker = PresetPicker::new(words.picker.clone(), current.reborrowed(), rows, Bind::bound(&mut browser))
                .field(PickerField::ThumbnailAndName)
                .display_name(env.units().pixels(values.diameter.round()))
                .cell_shape(CellShape::Square)
                .view(if parts.tip_only() { PresetView::Grid } else { PresetView::List })
                .toolbar(true)
                .keep_open()
                .thumb_range(TileZoom::bounded(
                    f64::from(*BrushPickerSettings::SCALE_RANGE.start()),
                    f64::from(*BrushPickerSettings::SCALE_RANGE.end()),
                    SCALE_STEP,
                    1.0,
                ))
                .footer(PresetFooterOptions::new().new_group(false))
                .head(head);
            let out = picker.show(ui);
            let mut picks = std::mem::take(&mut head_picks);
            if let Some(key) = out.picked {
                picks.insert(0, if out.closed { BrushPick::Activate(key) } else { BrushPick::Select(key) });
            }
            picks.extend(out.events.into_iter().map(BrushPick::List));
            let after = BrushPickerState { browser, parts: next_parts };
            let mut settings = None;
            if after != before {
                let event = EditEvent::set(Mixed::Same(before.clone()), after.clone(), name.clone(), None);
                state.reborrow().write(cx, &event);
                settings = (after.settings() != before.settings()).then(|| after.settings());
            }
            (out.edit, diameter, hardness, picks, settings)
        });
        match shown.inner {
            Some((edit, diameter, hardness, picks, settings)) => {
                BrushPresetPickerOutput { response: shown.output, edit: Some(edit), diameter, hardness, picks, settings }
            }
            None => BrushPresetPickerOutput {
                response: shown.output,
                edit: None,
                diameter: EditEvents::new(),
                hardness: EditEvents::new(),
                picks: Vec::new(),
                settings: None,
            },
        }
    }
}
