//! [`GradientMapEditor`]: the Gradient Map adjustment: the preset picker, the [`GradientEditor`] stop bar with colour
//! stops only, Reverse and Dither.
//!
//! It lives here, beside the gradient editor it builds on, rather than with the other adjustment editors in
//! craft-imaging: a family crate depends on no other family crate but craft-color.

#[cfg(feature = "screen-testing")]
mod stories;

use craft_behavior::{EditEvent, Mixed};
use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Pattern, Props as _, Show as _, ShowContainer as _, clicked};
use craft_ui::layout_components::Stack;
use craft_ui::patterns::presets::{PresetPicker, PresetRow};
use craft_ui::runtime::env::{Env, Label};
use egui::{Response, Ui};
use std::sync::Arc;

use super::{ColorStop, GradientEditor, GradientGroup, GradientRef, GradientStops, Sampler, StopColor, StopRows, StopRules};

const MAX_STOPS: usize = 64;
const MIN_STOPS: usize = 2;

/// The Gradient Map adjustment's value: the gradient the image's luminance maps through, and its two options.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct GradientMap {
    /// The gradient: colour stops only.
    pub stops: GradientStops,
    /// Whether the gradient runs from its end to its start.
    pub reverse: bool,
    /// Whether the mapped colours are dithered.
    pub dither: bool,
}

impl GradientMap {
    /// The map through `stops`, with the two options.
    pub fn new(stops: GradientStops, reverse: bool, dither: bool) -> Self {
        Self { stops, reverse, dither }
    }
}

craft_ui::words! {
    /// The words of the Gradient Map editor.
    pub struct GradientMapWords {
        /// The editor's group.
        gradient_map: "gradientMap.title" = "Gradient Map";
        /// The stop bar.
        stops: "gradientMap.stops" = "Gradient stops";
        /// The preset picker.
        preset: "gradientMap.preset" = "Preset";
        /// The picker's placeholder while the stops are no preset.
        custom: "gradientMap.custom" = "Custom";
        /// The command that imports presets.
        import_presets: "gradientMap.importPresets" = "Import .GRD";
        /// The Reverse option.
        reverse: "gradientMap.reverse" = "Reverse";
        /// The Dither option.
        dither: "gradientMap.dither" = "Dither";
    }
}

/// What the editor reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct GradientMapOutput {
    /// The editor's group.
    pub response: Response,
    /// The edit of the map made this pass, from the preset, the stop bar or an option.
    pub event: Option<EditEvent<GradientMap>>,
    /// The colour of a stop to ask the user for.
    pub color_requested: Option<StopColor>,
    /// Import .GRD was chosen: the host opens its file dialog. The value is unchanged.
    pub import: bool,
}

/// Photoshop's Gradient Map editor: top to bottom the preset picker (a pick replaces the stops with the preset's colour
/// stops, opacity dropped), Import .GRD, the stop bar of two to 64 colour stops without midpoints, and the Reverse and
/// Dither options. Each change is one edit.
#[must_use = "show the editor with `show`"]
#[derive(Pattern, craft_ui::contract::Props)]
pub struct GradientMapEditor<'a> {
    label: Label,
    value: &'a GradientMap,
    /// The presets the picker offers.
    #[prop]
    presets: &'a [GradientGroup],
    /// Samples the stop bar's ramp; the default sampler of colour-only rows by default.
    #[prop]
    sampler: Option<&'a Sampler>,
}

impl<'a> GradientMapEditor<'a> {
    /// An editor named `label` of `value`.
    pub fn new(label: impl Into<Label>, value: &'a GradientMap) -> Self {
        GradientMapEditor { label: label.into(), value, presets: &[], sampler: None }
    }

    /// The stops of preset `key`, as the map takes them: colour stops only, opaque, midpoints centred, at most 64.
    fn preset_stops(&self, key: GradientRef) -> Option<GradientStops> {
        let GradientRef::Item { group, item } = key else { return None };
        let preset = self.presets.get(group)?.items.get(item)?;
        let color: Vec<ColorStop> = preset
            .stops
            .color
            .iter()
            .take(MAX_STOPS)
            .map(|s| {
                let mut color = s.color;
                color.a = 1.0;
                ColorStop { color, midpoint: 0.5, ..*s }
            })
            .collect();
        (color.len() >= MIN_STOPS).then(|| GradientStops::new(color))
    }

    /// Shows the editor across the available width.
    pub fn show(self, ui: &mut Ui) -> GradientMapOutput {
        let words = Env::of(ui).words::<GradientMapWords>();
        let base = self.value.clone();
        let label: Arc<str> = Arc::from(words.gradient_map.as_str());
        let set = |after: GradientMap| EditEvent::set(Mixed::Same(base.clone()), after, Arc::clone(&label), None);
        let mut event = None;
        let mut color_requested = None;
        let mut import = false;
        let (preset_label, custom, import_label, stops_label) =
            (words.preset.clone(), words.custom.clone(), words.import_presets.clone(), words.stops.clone());
        let (reverse_label, dither_label) = (words.reverse.clone(), words.dither.clone());
        let shown = Stack::new().label(self.label.clone()).show(ui, |ui, _| {
            let rows: Vec<PresetRow<GradientRef>> = self
                .presets
                .iter()
                .enumerate()
                .flat_map(|(group, g)| {
                    let header = PresetRow::group(GradientRef::Item { group, item: usize::MAX }, g.name.clone(), 0);
                    let items =
                        g.items.iter().enumerate().map(move |(item, p)| PresetRow::item(GradientRef::Item { group, item }, p.name.clone(), 1));
                    std::iter::once(header).chain(items)
                })
                .collect();
            let picker =
                PresetPicker::new(preset_label.clone(), Bind::given(None::<GradientRef>), &rows, Bind::kept()).placeholder(custom.clone()).show(ui);
            if let Some(stops) = picker.picked.and_then(|key| self.preset_stops(key)) {
                event = Some(set(GradientMap { stops, ..base.clone() }));
            }
            if clicked(&Button::new(import_label.clone()).show(ui)) {
                import = true;
            }
            let rules = StopRules { rows: StopRows::ColorOnly, min: MIN_STOPS, max: MAX_STOPS, ..StopRules::default() };
            let mut editor =
                GradientEditor::new(stops_label.clone(), Bind::given(base.stops.clone())).rules(rules).midpoints(false).location_decimals(1);
            if let Some(sampler) = self.sampler {
                editor = editor.sampler(sampler);
            }
            let bar = editor.show(ui);
            if let Some(edit) = bar.edit.events.value().cloned() {
                event = Some(set(GradientMap { stops: edit, ..base.clone() }));
            }
            color_requested = bar.color_requested;
            let mut reverse = base.reverse;
            let toggled = !Checkbox::new(Bind::bound(&mut reverse)).label(reverse_label.clone()).show(ui).events.is_empty();
            if toggled {
                event = Some(set(GradientMap { reverse, ..base.clone() }));
            }
            let mut dither = base.dither;
            let toggled = !Checkbox::new(Bind::bound(&mut dither)).label(dither_label.clone()).show(ui).events.is_empty();
            if toggled {
                event = Some(set(GradientMap { dither, ..base.clone() }));
            }
        });
        GradientMapOutput { response: shown.output, event, color_requested, import }
    }
}
