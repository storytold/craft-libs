//! [`CurvesPanel`]: the Curves adjustment around the [`CurveEditor`]: the Channel select, the mode
//! icons (points, pencil), Smooth, the point pickers, the other channels overlaid on the composite
//! and the hint line; and the ⌘/Ctrl-click-on-the-image helper
//! ([`CurvesPanel::add_point_from_sample`]).

#[cfg(feature = "screen-testing")]
mod stories;

use std::sync::Arc;

use craft_behavior::{EditEvent, EditEvents, IconId, Mixed};
use craft_keymap::{Chord, Key, Mods};
use craft_theme::Srgba;
use craft_ui::buttons::{Button, ToggleButton};
use craft_ui::contract::{Bind, Entry, NoValue, Outcome, Pattern, Props, Remember as _, Sampler as _, Show as _, ShowContainer as _, clicked};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::TypeRole;
use egui::{Id, Response, Ui};

use super::{AdjustContext, AdjustWords, ToneChannel, ToneChannels, TonePicker, TonePickerRow};
use crate::ui::curve::{Curve, CurveChannel, CurveEditor, CurveRules};
use crate::ui::histogram::{ChannelColor, HistogramData};

/// The levels of a Curves value (8-bit tones) and the most points Photoshop allows.
const RULES: CurveRules = CurveRules::new(255.0, 16);
/// Smooth's moving-average radius (PhotoCraft's `smooth_curve_map`).
const SMOOTH_RADIUS: usize = 3;

/// The map `outputs` smoothed by a moving average over `2 * SMOOTH_RADIUS + 1` levels, the window
/// shrunk at the ends so the first and last levels keep their values.
fn smoothed(outputs: &[f32]) -> Vec<f32> {
    let n = outputs.len();
    (0..n)
        .map(|k| {
            let r = SMOOTH_RADIUS.min(k).min(n - k - 1);
            match outputs.get(k - r..=k + r) {
                Some(window) if r > 0 => window.iter().sum::<f32>() / window.len() as f32,
                _ => outputs[k],
            }
        })
        .collect()
}

/// The curves the editor overlays on the shown channel: every channel's on a composite
/// (Photoshop), none on a single channel or an isolated one.
pub(super) fn overlays_for<'a>(channels: &ToneChannels, shown: usize, curves: &'a [Curve]) -> &'a [Curve] {
    if channels.is_composite(shown) && !channels.is_isolated() { curves } else { &[] }
}

/// The modifier words of `mods` in the platform's notation (`⌘`, `Ctrl`, `Shift`).
fn modifier_words(os: craft_host::target::Os, mods: Mods) -> String {
    let with = Chord::new(mods, Key::A).display(os);
    let base = Chord::plain(Key::A).display(os);
    with.strip_suffix(base.as_str()).map_or(with.clone(), str::to_owned)
}

/// How the shown curve is edited: Photoshop's two buttons above the graph.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CurvesMode {
    /// Edit points to modify the curve; every channel is a [`Curve::Points`].
    #[default]
    Points,
    /// Draw to modify the curve; every channel is a [`Curve::Map`].
    Pencil,
}

/// A Curves value: one curve per channel of the panel's [`ToneChannels`] (same index), all in
/// one [`CurvesMode`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ToneCurves {
    /// The curves, one per channel.
    pub curves: Vec<Curve>,
    /// The mode all the curves share.
    pub mode: CurvesMode,
}

impl ToneCurves {
    /// The `curves`, one per channel, all in `mode`.
    pub fn new(curves: Vec<Curve>, mode: CurvesMode) -> Self {
        Self { curves, mode }
    }

    /// Identity point curves for `channels`, 0..=255.
    pub fn identity(channels: &ToneChannels) -> Self {
        Self { curves: vec![Curve::identity(255.0); channels.len()], mode: CurvesMode::Points }
    }
}

/// What [`CurvesPanel::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct CurvesPanelOutput {
    /// The panel's response.
    pub response: Response,
    /// The edit of this pass on the curves: the editor's gestures for the shown channel, and Smooth as one `Set` of
    /// the smoothed map. A binding that writes through has it applied already.
    pub edit: EditEvents<ToneCurves>,
    /// The channel the edit is for, and the one shown (an index into the panel's [`ToneChannels`]); the panel keeps
    /// it between passes.
    pub channel: usize,
    /// The user clicked the other mode's icon. The host converts every channel with its engine (draws the point
    /// curves into maps, or fits points to the drawn maps, PhotoCraft `set_pencil`) and passes the new value back;
    /// the panel changes nothing itself.
    pub mode: Option<CurvesMode>,
    /// The point picker the user armed (`Some(Some(p))`) or disarmed (`Some(None)`) this pass.
    pub picker: Option<Option<TonePicker>>,
    /// The selected point of the shown curve after the pass.
    pub selected: Option<usize>,
}

impl Outcome for CurvesPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The panel's own state: the selected point of the shown channel and the channel it belongs to.
#[derive(Clone, Debug, Default)]
struct Shown {
    channel: usize,
    selected: Option<usize>,
}

type SamplerFn<'a> = Box<dyn Fn(&[[f32; 2]]) -> Vec<f32> + 'a>;

/// The Curves adjustment: Image › Adjustments › Curves and a Curves layer's Properties panel.
///
/// Top to bottom:
/// 1. The Channel select ([`ToneChannels`]); an isolated channel is the only entry, its name shown exactly as the
///    host gave it.
/// 2. The mode row: two icon-only toggle buttons ("Edit points to modify the curve", "Draw to modify the curve", the
///    current one pressed) and Smooth (enabled in pencil mode only).
/// 3. The [`CurveEditor`] for the shown channel: in its colour, over its histogram, in pencil mode for a drawn curve,
///    and, on a composite only, the other channels' curves as overlays (Photoshop; none for an isolated channel).
/// 4. The hint line, with the key names from the platform's shortcut words.
/// 5. With [`pickers`](Self::pickers), the point picker row.
///
/// - **Smooth** replaces the shown map with its moving average over 7 levels (radius 3, shrunk at the ends so the
///   first and last levels keep their values: PhotoCraft's `smooth_curve_map`), as one `Set`.
/// - **Mode icons** report [`CurvesPanelOutput::mode`]; the current mode's icon reports nothing.
/// - **Selection**: the selected point is the panel's own state ([`CurvesPanelOutput::selected`]); the host sets
///   it with [`select`](Self::select), after [`add_point_from_sample`](Self::add_point_from_sample).
/// - **Sampling under the dialog**: with a [`canvas`](Self::canvas) the panel arms the canvas's sampler in every
///   pass, so a ⌘/Ctrl-click on the image reaches the host's tool, which calls `add_point_from_sample`.
#[must_use = "show the panel with `show`"]
#[derive(Pattern, Props)]
pub struct CurvesPanel<'a> {
    label: Label,
    curves: Bind<'a, ToneCurves>,
    channels: ToneChannels,
    /// The host's histograms, one per channel (same index), 256 bins each.
    #[prop]
    histograms: &'a [[u32; 256]],
    /// The host's interpolation of point curves ([`CurveEditor::sampler`]).
    #[prop(boxed = "Fn(&[[f32; 2]]) -> Vec<f32> + 'a")]
    sampler: Option<SamplerFn<'a>>,
    /// The armed point picker; the picker row shows when it is given.
    #[prop]
    pickers: Option<Bind<'a, Option<TonePicker>>>,
    /// The document canvas the sampler is armed for.
    #[prop]
    canvas: Option<Id>,
    picker_enabled: [bool; 3],
    /// The document's mode has no point pickers: all disabled, with the note.
    #[prop(flag)]
    pickers_unavailable: bool,
    /// Dialog or Properties panel.
    #[prop]
    context: AdjustContext,
    /// Selects point `index` of the shown curve in this pass, as a click on it would.
    #[prop]
    select: Option<usize>,
    /// Whether the whole panel takes input.
    #[prop]
    enabled: bool,
}

impl<'a> CurvesPanel<'a> {
    /// The panel named `label` over `curves`, one curve per channel of `channels`.
    pub fn new(label: impl Into<Label>, curves: Bind<'a, ToneCurves>, channels: ToneChannels) -> Self {
        Self {
            label: label.into(),
            curves,
            channels,
            histograms: &[],
            sampler: None,
            pickers: None,
            canvas: None,
            picker_enabled: [true; 3],
            pickers_unavailable: false,
            context: AdjustContext::dialog(true),
            select: None,
            enabled: true,
        }
    }

    /// Enables or disables one picker.
    pub fn picker_enabled(mut self, picker: TonePicker, enabled: bool) -> Self {
        self.picker_enabled[picker as usize] = enabled;
        self
    }

    /// ⌘/Ctrl-click on the image while Curves is open: adds a point to the shown channel's curve
    /// at the sampled tone, on the curve (its output there), and returns its index for
    /// [`select`](Self::select). With `every_channel` (Shift) it adds one to each of
    /// Red, Green and Blue instead, at that channel's value, and returns the shown channel's
    /// point if the shown channel is one of them.
    ///
    /// - The tone, 0..=255 rounded to a whole level: Red, Green, Blue read their channel; the
    ///   composite reads Rec. 709 luma (0.2126 R + 0.7152 G + 0.0722 B); a grey document the mean
    ///   of R, G, B.
    /// - The output is the curve's at that input (through `sampler`, the host's interpolation, run by run between
    ///   corner points), rounded; the point goes in through [`CurveRules::insert`], so a point already at that input
    ///   is reused and the 16-point limit holds.
    /// - Does nothing and returns `None` in pencil mode, for Lab and CMYK channels, for an
    ///   isolated mask or saved channel ([`ToneChannel::Named`]: it holds its own grey levels;
    ///   an isolated Red, Green or Blue channel takes the point) and when `sample` is off the image
    ///   (the host doesn't call it then). `every_channel` is ignored for a grey document.
    pub fn add_point_from_sample(
        curves: &mut ToneCurves,
        channels: &ToneChannels,
        shown: usize,
        sample: Srgba,
        every_channel: bool,
        sampler: &dyn Fn(&[[f32; 2]]) -> Vec<f32>,
    ) -> Option<usize> {
        if curves.mode == CurvesMode::Pencil {
            return None;
        }
        let list = channels.as_slice();
        let shown_channel = list.get(shown)?;
        let gray = channels.is_gray();
        let level = |channel: &ToneChannel| -> Option<f32> {
            Some(match channel {
                ToneChannel::Red => sample.r,
                ToneChannel::Green => sample.g,
                ToneChannel::Blue => sample.b,
                ToneChannel::Composite(super::ToneSpace::Rgb) => sample.r * 0.2126 + sample.g * 0.7152 + sample.b * 0.0722,
                ToneChannel::Gray => (sample.r + sample.g + sample.b) / 3.0,
                _ => return None,
            })
        };
        level(shown_channel)?;
        let targets: Vec<usize> = if every_channel && !gray {
            list.iter().enumerate().filter(|(_, c)| matches!(c, ToneChannel::Red | ToneChannel::Green | ToneChannel::Blue)).map(|(i, _)| i).collect()
        } else {
            vec![shown]
        };
        let mut selected = None;
        for index in targets {
            let Some(tone) = list.get(index).and_then(level) else { continue };
            let input = (tone.clamp(0.0, 1.0) * 255.0).round();
            let Some(Curve::Points { points, corners }) = curves.curves.get_mut(index) else { continue };
            corners.resize(points.len(), false);
            let output = Curve::through(points, corners, sampler).get(input as usize).copied().unwrap_or(input).round();
            let before = points.len();
            let Some(at) = RULES.insert(points, [input, output]) else { continue };
            if points.len() > before {
                corners.insert(at, false);
            }
            if index == shown {
                selected = Some(at);
            }
        }
        selected
    }

    /// Draws the panel and reports the shown channel's edit.
    pub fn show(self, ui: &mut Ui) -> CurvesPanelOutput {
        let Self {
            label,
            mut curves,
            channels,
            histograms,
            sampler,
            mut pickers,
            canvas,
            picker_enabled,
            pickers_unavailable,
            context,
            select,
            enabled,
        } = self;
        let env = Env::of(ui);
        let words = env.words::<AdjustWords>();
        let view = env.view();
        let os = view.shell().os;
        let entries: Vec<Entry<usize>> = channels.as_slice().iter().enumerate().map(|(at, channel)| Entry::new(at, channel.label(&words))).collect();
        let curve_channels: Vec<CurveChannel> = channels.as_slice().iter().map(|c| CurveChannel::new(c.label(&words), c.color())).collect();
        let (mut edit, mut mode, mut picker) = (EditEvents::new(), None, None);
        let mut selected = None;
        let mut shown_channel = 0;
        let group = Stack::new().label(label).enabled(enabled).show(ui, |ui, cx| {
            let value = match curves.reborrow().read(cx) {
                Some(Mixed::Same(value)) => value,
                _ => ToneCurves::identity(&channels),
            };
            let state = cx.keep::<Shown>().clone();
            let channel = state.channel.min(channels.len().saturating_sub(1));
            let pencil = value.mode == CurvesMode::Pencil;
            let fallback = Curve::identity(255.0);
            let curve = value.curves.get(channel).unwrap_or(&fallback);
            let tone = channels.as_slice().get(channel);
            let color = tone.map_or(ChannelColor::Neutral, ToneChannel::color);
            let name = tone.map_or(Label::Empty, |c| c.label(&words));
            let data = histograms.get(channel).map(|counts| HistogramData::tone(counts, color));
            let overlays = overlays_for(&channels, channel, &value.curves);
            let key = modifier_words(os, if pencil { Mods::SHIFT } else { Mods::CMD });
            let template = if pencil { &words.pencil_hint } else { &words.points_hint };
            let hint = template.fill(&[("key", &key)]);
            // A Curves panel in a dialog keeps the image under the dialog usable for the ⌘/Ctrl-click point.
            if let Some(canvas) = canvas {
                cx.arm_sampler(canvas);
            }

            let mut picked = channel;
            Row::new().show(ui, |ui, _| {
                Text::new(words.channel.clone()).shade(Shade::Muted).role(TypeRole::Label).show(ui);
                if let Some(&at) = Select::new(words.channel.clone(), Bind::given(channel), &entries).show(ui).value() {
                    picked = at;
                }
            });
            let mut smooth = false;
            Row::new().show(ui, |ui, _| {
                for (target, icon, name) in
                    [(CurvesMode::Points, "spline", words.edit_points.clone()), (CurvesMode::Pencil, "pencil", words.draw_curve.clone())]
                {
                    let current = value.mode == target;
                    let toggle = ToggleButton::new(name, Bind::given(current)).icon(IconId::new(icon)).icon_only().id_salt(target as u8).show(ui);
                    if toggle.value() == Some(&true) && !current {
                        mode = Some(target);
                    }
                }
                let can_smooth = pencil && matches!(curve, Curve::Map(_));
                smooth = clicked(&Button::new(words.smooth.clone()).enabled(can_smooth).tooltip(words.smooth_tip.clone()).show(ui));
            });

            let selection = if picked == channel { select.or(state.selected) } else { None };
            let sample = sampler.as_deref();
            let mut editor = CurveEditor::new(name, Bind::given(curve.clone()))
                .channels(&curve_channels)
                .channel(channel)
                .overlays(overlays)
                .hint(hint)
                .levels(255)
                .max_points(RULES.max_points)
                .selection(selection);
            if pencil {
                editor = editor.pencil();
            }
            if let Some(data) = &data {
                editor = editor.histogram(data);
            }
            if let Some(sample) = sample {
                editor = editor.sampler(|points: &[[f32; 2]]| sample(points));
            }
            let shown = editor.show(ui);
            selected = shown.selected;
            let mapped = |one: Curve| {
                let mut next = value.clone();
                if let Some(slot) = next.curves.get_mut(channel) {
                    *slot = one;
                }
                next
            };
            let mut events = shown.edit.events.clone().map(mapped);
            if let (true, Curve::Map(map)) = (smooth, curve) {
                events = EditEvent::set(Mixed::Same(value.clone()), mapped(Curve::Map(smoothed(map))), Arc::from("Smooth"), None).into();
            }
            for event in &events {
                curves.reborrow().write(cx, event);
            }
            edit = events;

            if let Some(armed) = pickers.as_mut() {
                let mut row = TonePickerRow::new(armed.reborrowed());
                if pickers_unavailable {
                    row = row.unavailable();
                }
                if let Some(canvas) = canvas {
                    row = row.canvas(canvas);
                }
                for each in TonePicker::ALL {
                    let usable = picker_enabled[each as usize] && !(each == TonePicker::Gray && (channels.is_gray() || context.is_gray()));
                    row = row.enabled_picker(each, usable);
                }
                picker = row.show(ui).change;
            }
            // An edit belongs to the channel it was drawn for; a switch shows from the next pass, with no selection.
            shown_channel = if picked == channel || !edit.is_empty() { channel } else { picked };
            *cx.keep::<Shown>() = Shown { channel: shown_channel, selected: if shown_channel == channel { selected } else { None } };
        });
        CurvesPanelOutput { response: group.output, edit, channel: shown_channel, mode, picker, selected }
    }
}
