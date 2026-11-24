//! [`GradientMapEditor`]: the Gradient Map adjustment: the preset picker, the [`GradientEditor`] stop bar with colour
//! stops only, Reverse, Dither and the interpolation method.
//!
//! It lives here rather than with the other adjustment editors in craft-imaging because it picks from the palette's
//! [`GradientGroup`] presets, and a family crate depends on no other family crate but craft-color-ui.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use craft_behavior::{EditEvent, Mixed};
use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Entry, Pattern, Props as _, Remember as _, Show as _, ShowContainer as _, clicked};
use craft_ui::display::{Shade, Text};
use craft_ui::form::Field;
use craft_ui::layout_components::Stack;
use craft_ui::patterns::presets::{PickerField, PresetFns, PresetPicker, PresetRow, PresetSource};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::TypeRole;
use egui::{Response, Ui};
use std::sync::Arc;

use craft_color_ui::ui::gradient::{ColorStop, GradientEditor, GradientStops, Sampler, StopColor, StopRows, StopRules};
use serde::{Deserialize, Serialize};

use super::{GradientGroup, GradientRef};

const MAX_STOPS: usize = 64;
const MIN_STOPS: usize = 2;

/// An explicit pick wins when two presets have the same colour stops.
#[derive(Clone, Copy, Default)]
struct PickedPreset(Option<GradientRef>);

/// How a Gradient Map's colours run between its stops. The host renders them; the editor only offers the choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum GradientMethod {
    /// Even steps of perceived lightness between the stops.
    Perceptual,
    /// Straight across the stops in linear light.
    Linear,
    /// Straight across the stops' encoded values, bent by the gradient's smoothness.
    Classic,
    /// Like Perceptual, with an eased start and end of each segment. A new map's method.
    #[default]
    Smooth,
    /// Each stop's colour held flat up to the next stop: hard bands, no blend.
    Stripes,
}

impl GradientMethod {
    /// Every method, in the order the editor lists them.
    pub const ALL: [GradientMethod; 5] =
        [GradientMethod::Perceptual, GradientMethod::Linear, GradientMethod::Classic, GradientMethod::Smooth, GradientMethod::Stripes];

    /// The method's stable lowercase name, as it is serialized (`perceptual`, `linear`, `classic`, `smooth`, `stripes`).
    pub fn name(self) -> &'static str {
        match self {
            GradientMethod::Perceptual => "perceptual",
            GradientMethod::Linear => "linear",
            GradientMethod::Classic => "classic",
            GradientMethod::Smooth => "smooth",
            GradientMethod::Stripes => "stripes",
        }
    }

    /// The method [`name`](Self::name)d `name`, if any.
    pub fn from_name(name: &str) -> Option<GradientMethod> {
        GradientMethod::ALL.into_iter().find(|m| m.name() == name)
    }
}

/// The Gradient Map adjustment's value: the gradient the image's luminance maps through, its two options and the
/// interpolation method.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct GradientMap {
    /// The gradient: colour stops only.
    pub stops: GradientStops,
    /// Whether the gradient runs from its end to its start.
    pub reverse: bool,
    /// Whether the mapped colours are dithered.
    pub dither: bool,
    /// How the colours run between the stops; Smooth when a stored value has none.
    #[serde(default)]
    pub method: GradientMethod,
}

impl GradientMap {
    /// The map through `stops`, with the two options and the Smooth method.
    pub fn new(stops: GradientStops, reverse: bool, dither: bool) -> Self {
        Self { stops, reverse, dither, method: GradientMethod::default() }
    }

    /// The map with `method`.
    #[must_use]
    pub fn with_method(mut self, method: GradientMethod) -> Self {
        self.method = method;
        self
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
        /// How to add another stop on the gradient.
        add_stop_hint: "gradientMap.addStopHint" = "Click below the ramp to add a stop.";
        /// The command that imports presets.
        import_presets: "gradientMap.importPresets" = "Import .GRD";
        /// The Reverse option.
        reverse: "gradientMap.reverse" = "Reverse";
        /// The Dither option.
        dither: "gradientMap.dither" = "Dither";
        /// The interpolation method list.
        method: "gradientMap.method" = "Method";
        /// Even steps of perceived lightness.
        perceptual: "gradientMap.methodPerceptual" = "Perceptual";
        /// Straight in linear light.
        linear: "gradientMap.methodLinear" = "Linear";
        /// Straight across the encoded values.
        classic: "gradientMap.methodClassic" = "Classic";
        /// Perceptual with eased segment ends.
        smooth: "gradientMap.methodSmooth" = "Smooth";
        /// Hard bands, no blend.
        stripes: "gradientMap.methodStripes" = "Stripes";
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
/// stops, opacity dropped), Import .GRD, the stop bar of two to 64 colour stops without midpoints, the Reverse and
/// Dither options and the Method list (Perceptual, Linear, Classic, Smooth, Stripes). Each change is one edit.
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
    /// Methods the Method list shows but does not let the user pick: those the host cannot render yet.
    #[prop]
    disabled_methods: &'a [GradientMethod],
}

impl<'a> GradientMapEditor<'a> {
    /// An editor named `label` of `value`.
    pub fn new(label: impl Into<Label>, value: &'a GradientMap) -> Self {
        GradientMapEditor { label: label.into(), value, presets: &[], sampler: None, disabled_methods: &[] }
    }

    /// A preset as Gradient Map uses it: opaque colour stops with centred midpoints.
    fn preset_colors(&self, key: GradientRef) -> Option<impl Iterator<Item = ColorStop> + '_> {
        let GradientRef::Item { group, item } = key else { return None };
        let colors = &self.presets.get(group)?.items.get(item)?.stops.color;
        (colors.len() >= MIN_STOPS).then(|| {
            colors.iter().take(MAX_STOPS).map(|s| {
                let mut stop = s.with_midpoint(0.5);
                stop.color.a = 1.0;
                stop
            })
        })
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
        let (reverse_label, dither_label, method_label) = (words.reverse.clone(), words.dither.clone(), words.method.clone());
        let method_entries: Vec<Entry<GradientMethod>> = GradientMethod::ALL
            .into_iter()
            .map(|m| {
                let name = match m {
                    GradientMethod::Perceptual => &words.perceptual,
                    GradientMethod::Linear => &words.linear,
                    GradientMethod::Classic => &words.classic,
                    GradientMethod::Smooth => &words.smooth,
                    GradientMethod::Stripes => &words.stripes,
                };
                Entry::new(m, name.clone()).enabled(!self.disabled_methods.contains(&m))
            })
            .collect();
        let shown = Stack::new().label(self.label.clone()).show(ui, |ui, cx| {
            let source = PresetSource::new(self.presets, PresetFns::new(preset_rows));
            // Stop tags identify host data, so they do not determine which preset's colours match.
            let matches = |&key: &GradientRef| {
                self.preset_colors(key).is_some_and(|colors| colors.map(|s| s.with_tag(0)).eq(base.stops.color.iter().map(|s| s.with_tag(0))))
            };
            let preferred = cx.keep::<PickedPreset>().0.filter(matches);
            let mut items =
                self.presets.iter().enumerate().flat_map(|(group, g)| (0..g.items.len()).map(move |item| GradientRef::Item { group, item }));
            let current = preferred.or_else(|| items.find(matches));
            let picked = Field::new(preset_label.clone())
                .show(ui, |ui, _| {
                    PresetPicker::new(preset_label.clone(), Bind::given(current), &source, Bind::kept())
                        .field(PickerField::Name)
                        .placeholder(custom.clone())
                        .show(ui)
                        .picked
                })
                .inner
                .flatten();
            if let Some(key) = picked
                && let Some(colors) = self.preset_colors(key)
            {
                cx.keep::<PickedPreset>().0 = Some(key);
                event = Some(set(GradientMap { stops: GradientStops::new(colors.collect()), ..base.clone() }));
            }
            if clicked(&Button::new(import_label.clone()).show(ui)) {
                import = true;
            }
            if base.stops.color.len() < MAX_STOPS {
                Text::new(words.add_stop_hint.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
            }
            let rules = StopRules::new(StopRows::ColorOnly).stops(MIN_STOPS, MAX_STOPS);
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
            let picked = Field::new(method_label.clone())
                .id_salt("method")
                .show(ui, |ui, _| Select::new(method_label.clone(), Bind::given(base.method), &method_entries).show(ui).value().copied())
                .inner
                .flatten();
            if let Some(method) = picked.filter(|m| *m != base.method && !self.disabled_methods.contains(m)) {
                event = Some(set(GradientMap { method, ..base.clone() }));
            }
        });
        GradientMapOutput { response: shown.output, event, color_requested, import }
    }
}

/// The picker's rows: each group's header, then its presets. A header's key names no preset (`item == usize::MAX`).
fn preset_rows(presets: &&[GradientGroup], _: &Env) -> Vec<PresetRow<GradientRef>> {
    presets
        .iter()
        .enumerate()
        .flat_map(|(group, g)| {
            let header = PresetRow::group(GradientRef::Item { group, item: usize::MAX }, g.name.clone(), 0);
            let items = g.items.iter().enumerate().map(move |(item, p)| PresetRow::item(GradientRef::Item { group, item }, p.name.clone(), 1));
            std::iter::once(header).chain(items)
        })
        .collect()
}
