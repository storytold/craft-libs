//! [`HueSaturationEditor`]: Master and six colour ranges picked by their swatches, Colorize, the two spectrum bars and
//! the four range thumbs that wrap at 360°.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use craft_behavior::{Metrics, Mixed, Sense};
use craft_theme::{Hsb, Srgba};
use craft_ui::choice::{Checkbox, RadioGroup, Swatch, SwatchFill, SwatchMetrics, SwatchShape};
use craft_ui::contract::{
    Bind, Common, Component, Cx, DisabledLook, EditResponse, Gap, MeasureCx, Outcome, PaintUi, Pattern, Props as _, Remember as _, Role, Show as _,
    ShowContainer as _,
};
use craft_ui::display::{Shade, Text};
use craft_ui::layout::RowAlign;
use craft_ui::layout_components::{Row as LayoutRow, Stack};
use craft_ui::primitives::PaintShape;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::style::{ComponentStyle, DataColor, StyleColor, StyleCx, TypeRole};
use craft_ui::value_drag::{Anchor, OffThumbs, SpanInk, Thumb, ThumbRow, ThumbSpan, TrackFace};
use egui::{Color32, Response, Ui, Vec2};

use super::sliders::{Run, editor_group, lifted, read, words};
use super::style::tones;
use super::{AdjustContext, AdjustField, AdjustTrack};
use crate::ui::curve::ToneRamp;

/// Hues sampled on each spectrum bar: every 10°, 0 to 360.
const SAMPLES: usize = 37;
/// The distance between two samples of a spectrum bar, in degrees.
const SAMPLE_STEP: f64 = 10.0;
/// The distance between two stops of the Hue slider's track, in degrees.
const TRACK_STEP: f64 = 30.0;
/// The stops of the Hue slider's track: red through the spectrum and back to red.
const TRACK_STOPS: usize = 13;
/// Colorize keeps Saturation at or above this (Photoshop's 25 %).
const COLORIZE_SATURATION: f64 = 25.0;
/// A turn.
const TURN: f64 = 360.0;
/// The most a range spans: the end thumb is at most this far past the start.
const MAX_SPAN: f64 = 359.0;
/// The bands under a range's thumbs: the whole range faint, its full-strength part muted.
const RANGE_SPANS: [ThumbSpan; 2] = [ThumbSpan::new(0, 3, SpanInk::Faint), ThumbSpan::new(1, 2, SpanInk::Muted)];
/// The grey Colorize's swatch shows the adjustment of: what Colorize makes of it is the colorize colour.
const MID_GREY: Srgba = Srgba::rgb(0.5, 0.5, 0.5);
/// The edited mark's dot as a share of its slot's height.
const EDITED_DOT: f32 = 1.0;

/// One of Hue/Saturation's six colour ranges (Reds, Yellows, Greens, Cyans, Blues, Magentas).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct HueRangeValue {
    /// Hue shift −180..=180°, whole degrees.
    pub hue: f64,
    /// −100..=100 %.
    pub saturation: f64,
    /// −100..=100 %.
    pub lightness: f64,
    /// Where the range starts fading in, is fully on, starts fading out and ends, in degrees
    /// (Photoshop's four range sliders); may run past 0 or 360.
    pub bounds: [f64; 4],
}

impl HueRangeValue {
    /// A range with the shifts `hue`, `saturation` and `lightness` and the four `bounds`.
    pub fn new(hue: f64, saturation: f64, lightness: f64, bounds: [f64; 4]) -> Self {
        Self { hue, saturation, lightness, bounds }
    }

    /// Range `index` (0 Reds … 5 Magentas) unchanged, with Photoshop's extent: centre `60 ×
    /// index`, fully on for 30° around it, fading over 30° each side (`[c − 45, c − 15, c + 15,
    /// c + 45]`).
    pub fn neutral(index: usize) -> Self {
        let c = 60.0 * index as f64;
        Self { hue: 0.0, saturation: 0.0, lightness: 0.0, bounds: [c - 45.0, c - 15.0, c + 15.0, c + 45.0] }
    }

    /// The canonical form of four bounds: increasing, within one turn (each at most 359° past the
    /// first), centre (the mean of the inner two) moved into −30..330. Any non-finite bound gives
    /// the Reds' default `[−45, −15, 15, 45]`.
    pub fn canonical_bounds(bounds: [f64; 4]) -> [f64; 4] {
        if bounds.iter().any(|b| !b.is_finite()) {
            return [-45.0, -15.0, 15.0, 45.0];
        }
        let mut out = [bounds[0]; 4];
        for k in 1..4 {
            out[k] = (out[k - 1] + (bounds[k] - out[k - 1]).rem_euclid(360.0)).min(out[0] + 359.0);
        }
        let centre = (out[1] + out[2]) / 2.0;
        let shift = (centre + 30.0).rem_euclid(360.0) - 30.0 - centre;
        out.map(|b| b + shift)
    }

    /// The middle of the range's fully-on span (its inner two bounds, canonical), in 0..360: 0° for the Reds, 240°
    /// for the Blues.
    pub fn centre(&self) -> f64 {
        let bounds = Self::canonical_bounds(self.bounds);
        ((bounds[1] + bounds[2]) / 2.0).rem_euclid(TURN)
    }

    /// Whether the range's own hue, saturation or lightness changes anything.
    fn edited(&self) -> bool {
        self.hue != 0.0 || self.saturation != 0.0 || self.lightness != 0.0
    }

    /// `bounds` with thumb `index` (0 range start … 3 range end) dragged to `degrees` (0..=360 on
    /// the bar), PhotoCraft's rule: canonicalise first; the start moves by the shortest turn to
    /// `degrees`, at most to the second thumb and at least 180° before it; any other thumb moves
    /// to `degrees` measured from the start, between its neighbours (the end at most 359° past the
    /// start). An index past 3 changes nothing but the canonical form.
    pub fn move_bound(bounds: [f64; 4], index: usize, degrees: f64) -> [f64; 4] {
        let mut b = Self::canonical_bounds(bounds);
        if index >= b.len() {
            return b;
        }
        if index == 0 {
            let delta = (degrees - b[0] + 180.0).rem_euclid(360.0) - 180.0;
            b[0] = (b[0] + delta).min(b[1]).max(b[1] - 180.0);
        } else {
            let rel = (degrees - b[0]).rem_euclid(360.0);
            let hi = if index == 3 { b[0] + 359.0 } else { b[index + 1] };
            b[index] = (b[0] + rel).clamp(b[index - 1], hi);
        }
        b
    }
}

/// A Hue/Saturation value.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct HueSaturation {
    /// Master hue: −180..=180°, or 0..=360° with Colorize.
    pub hue: f64,
    /// Master saturation: −100..=100 %, or 0..=100 % with Colorize.
    pub saturation: f64,
    /// Master lightness: −100..=100 %.
    pub lightness: f64,
    /// Colorize: tints the whole image with the master hue.
    pub colorize: bool,
    /// Reds, Yellows, Greens, Cyans, Blues, Magentas.
    pub ranges: [HueRangeValue; 6],
}

impl HueSaturation {
    /// A value of the master shifts, Colorize and the six ranges.
    pub fn new(hue: f64, saturation: f64, lightness: f64, colorize: bool, ranges: [HueRangeValue; 6]) -> Self {
        Self { hue, saturation, lightness, colorize, ranges }
    }
}

impl Default for HueSaturation {
    fn default() -> Self {
        Self { hue: 0.0, saturation: 0.0, lightness: 0.0, colorize: false, ranges: std::array::from_fn(HueRangeValue::neutral) }
    }
}

/// Which of the three rows of a view.
#[derive(Clone, Copy)]
enum Row {
    Hue,
    Saturation,
    Lightness,
}

impl HueSaturation {
    /// `self` with a row set to `value`, whole degrees or percent.
    fn with_row(mut self, range: Option<usize>, row: Row, value: f64) -> Self {
        let value = value.round();
        match (range, row) {
            (None, Row::Hue) => self.hue = value,
            (None, Row::Saturation) => self.saturation = value,
            (None, Row::Lightness) => self.lightness = value,
            (Some(r), Row::Hue) => self.ranges[r].hue = value,
            (Some(r), Row::Saturation) => self.ranges[r].saturation = value,
            (Some(r), Row::Lightness) => self.ranges[r].lightness = value,
        }
        self
    }

    /// `self` with Colorize switched: Hue wraps into 0..360 and Saturation reaches at least 25; off, a Hue above 180
    /// wraps back by 360.
    fn colorized(mut self, on: bool) -> Self {
        self.colorize = on;
        if on {
            self.hue = self.hue.rem_euclid(TURN);
            self.saturation = self.saturation.max(COLORIZE_SATURATION);
        } else if self.hue > 180.0 {
            self.hue -= TURN;
        }
        self
    }

    /// A value holding only range `r` of `self`: the master unchanged, Colorize off and the other ranges neutral, so
    /// an adjustment of it shows what that range alone does.
    fn only_range(&self, r: usize) -> HueSaturation {
        let mut only = HueSaturation::default();
        only.ranges[r] = self.ranges[r];
        only
    }

    /// The four thumbs of range `r` as the thumb row shows them: each bound within one turn.
    fn thumbs(&self, r: usize) -> [f64; 4] {
        HueRangeValue::canonical_bounds(self.ranges[r].bounds).map(|degrees| degrees.rem_euclid(TURN))
    }

    /// `self` with the thumbs of range `r` where the thumb row put them: each thumb that moved goes through
    /// [`HueRangeValue::move_bound`], whole degrees.
    fn with_thumbs(mut self, r: usize, row: &[Mixed<f64>; 4]) -> Self {
        let shown = self.thumbs(r);
        let mut bounds = self.ranges[r].bounds;
        for (k, (put, was)) in row.iter().zip(shown).enumerate() {
            if let Some(&degrees) = put.same().filter(|&&degrees| degrees != was) {
                bounds = HueRangeValue::move_bound(bounds, k, degrees);
            }
        }
        self.ranges[r].bounds = bounds.map(f64::round);
        self
    }
}

/// A fully saturated, full-brightness hue.
fn hue_srgba(degrees: f64) -> Srgba {
    Srgba::from_hsb(Hsb { h: degrees.rem_euclid(TURN) as f32, s: 1.0, b: 1.0 })
}

/// A fully saturated, full-brightness hue as image data.
fn hue_color(degrees: f64) -> DataColor {
    DataColor::new(Color32::from(hue_srgba(degrees)))
}

/// The hue spectrum for a colour track: red, yellow, green, cyan, blue, magenta, red.
fn hue_stops() -> Vec<DataColor> {
    (0..TRACK_STOPS).map(|i| hue_color(i as f64 * TRACK_STEP)).collect()
}

/// What a Hue/Saturation editor keeps between passes: the range shown (0 Master, 1 Reds … 6 Magentas) and the value
/// its running gesture began with.
#[derive(Clone, Default)]
struct HueMemory {
    range: usize,
    began: Option<HueSaturation>,
}

/// What a [`HueSaturationEditor`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct HueSaturationOutput {
    /// The edit of the value.
    pub edit: EditResponse<HueSaturation>,
    /// The range shown after the pass: `None` for Master, else 0 Reds … 5 Magentas.
    pub shown_range: Option<usize>,
}

impl Outcome for HueSaturationOutput {
    type Value = HueSaturation;

    fn response(&self) -> &egui::Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

impl std::ops::Deref for HueSaturationOutput {
    type Target = EditResponse<HueSaturation>;

    fn deref(&self) -> &Self::Target {
        &self.edit
    }
}

/// What a colour of the image becomes under a Hue/Saturation value; see [`HueSaturationEditor::adjusted`].
pub type AdjustedFn<'a> = dyn Fn(&HueSaturation, Srgba) -> Srgba + 'a;

/// Hue/Saturation.
///
/// - **Range swatches** (Master, Reds … Magentas): a radio group of round swatches, view state of the editor, Master
///   (the hue wheel) at first. A range swatch shows the colour at the range's centre; once the range's own Hue,
///   Saturation or Lightness is not zero it shows that colour before and after the range alone (start and end halves,
///   through [`adjusted`](Self::adjusted)) with a dot under it. With a range shown, its centre stands above the Hue row,
///   at the end edge: "240°", or "0° / 30°" once its Hue moves it. While Colorize is on the row is one read-only
///   swatch of what the value makes of a mid grey, and the editor shows Master. The range shown comes back in
///   [`HueSaturationOutput::shown_range`].
/// - **Master**: Hue (−180..180°; 0..360° with Colorize) over the hue spectrum track, Saturation (−100..100 %;
///   0..100 % with Colorize) over a track from grey to the current hue, Lightness (−100..100 %) over a fixed black,
///   grey, white track. Tracks only while dynamic tracks are on.
/// - **A range**: its Hue, Saturation and Lightness rows, then the range thumbs (below).
/// - **Colorize** (a `Set`): on, Hue wraps into 0..360 and Saturation becomes at least 25; off, a Hue above 180 wraps
///   back by −360.
/// - **Spectrum bars**: the input hues above (37 samples, every 10°), the host's [`adjusted`](Self::adjusted) below
///   (the same 37 hues after the adjustment; the input bar again without it). Not mirrored in right-to-left layouts: they are scales of the image.
/// - **Range thumbs** (a range shown): four thumbs under the lower bar as one thumb row that wraps through 0° and keeps
///   its order going round from the first thumb, over a faint band from the first thumb to the last and a muted one
///   between the middle two. A drag on a thumb moves it with [`HueRangeValue::move_bound`], whole degrees; a press
///   off the thumbs does nothing. The row is not mirrored either. Under the bars a readout "a° / b°  ·  c° \ d°", each bound within one turn.
#[must_use = "show the editor with `show`"]
#[derive(Pattern)]
pub struct HueSaturationEditor<'a> {
    label: Label,
    value: Bind<'a, HueSaturation>,
    adjusted: Option<&'a AdjustedFn<'a>>,
    range: Option<Option<usize>>,
    context: AdjustContext,
}

impl<'a> HueSaturationEditor<'a> {
    /// The editor named `label` of `value`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, HueSaturation>) -> Self {
        Self { label: label.into(), value, adjusted: None, range: None, context: AdjustContext::dialog(true) }
    }

    /// What a colour of the image becomes under a value: the host runs its engine. The lower spectrum bar asks for 37
    /// full hues under the shown value, each edited range's swatch for its centre colour under a value holding only
    /// that range, and Colorize's swatch for a mid grey under the shown value. Without it every colour stays as it is.
    pub fn adjusted(mut self, adjusted: &'a AdjustedFn<'a>) -> Self {
        self.adjusted = Some(adjusted);
        self
    }

    /// The range shown, kept by the host: `None` for Master, else 0 Reds … 5 Magentas. The editor reports the user's
    /// choice in [`HueSaturationOutput::shown_range`].
    pub fn range(mut self, range: Option<usize>) -> Self {
        self.range = Some(range);
        self
    }

    /// Dialog or Properties panel; a dialog by default.
    pub fn context(mut self, context: AdjustContext) -> Self {
        self.context = context;
        self
    }

    /// Shows the editor and returns this pass's edit of the `HueSaturation` and the range shown.
    pub fn show(self, ui: &mut Ui) -> HueSaturationOutput {
        let HueSaturationEditor { label, mut value, adjusted, range: host_range, context } = self;
        let words = words(ui);
        let units = Env::of(ui).units();
        let mut out = None;
        let shown = editor_group(ui, label, context, |ui, cx| {
            let now = read(&mut value, cx).unwrap_or_default();
            let memory = cx.memory::<HueMemory>().clone();
            // Colorize works on the master only.
            let kept = host_range.map_or(memory.range, |range| range.map_or(0, |r| r + 1));
            let at = if now.colorize { 0 } else { kept.min(6) };
            let range = at.checked_sub(1);
            let names = [&words.reds, &words.yellows, &words.greens, &words.cyans, &words.blues, &words.magentas];
            let after = |value: &HueSaturation, color: Srgba| adjusted.map_or(color, |adjusted| adjusted(value, color));
            let mut picked = None;
            if now.colorize {
                Swatch::new(words.colorize_color.clone(), after(&now, MID_GREY)).shape(SwatchShape::Round).read_only(true).show(ui);
            } else {
                picked = RadioGroup::new(words.range.clone(), Bind::given(at))
                    .show(ui, |ui, _| {
                        LayoutRow::new().show(ui, |ui, _| {
                            for shown in 0..=names.len() {
                                let (label, fill, edited) = match shown.checked_sub(1) {
                                    None => (words.master.clone(), SwatchFill::Spectrum, false),
                                    Some(r) => {
                                        let range = now.ranges[r];
                                        let before = hue_srgba(range.centre());
                                        let fill = if range.edited() {
                                            SwatchFill::Split(before, after(&now.only_range(r), before))
                                        } else {
                                            SwatchFill::Color(Mixed::Same(before))
                                        };
                                        (names[r].clone(), fill, range.edited())
                                    }
                                };
                                Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
                                    Swatch::with_fill(label.clone(), fill.clone()).shape(SwatchShape::Round).member(shown).show(ui);
                                    EditedMark::new(edited).show(ui);
                                });
                            }
                        });
                    })
                    .value()
                    .copied();
            }
            if let Some(r) = at.checked_sub(1) {
                let range = now.ranges[r];
                let degrees = |at: f64| units.text(UnitKind::Degrees, at.round().rem_euclid(TURN) + 0.0, 0..=0);
                let centre = range.centre();
                let text = if range.hue.round() == 0.0 {
                    degrees(centre)
                } else {
                    words.range_centre_moved.fill(&[("from", &degrees(centre)), ("to", &degrees(centre + range.hue))])
                };
                LayoutRow::new().fill_width().align(RowAlign::End).show(ui, |ui, _| {
                    Text::new(text.clone()).shade(Shade::Faint).show(ui);
                });
            }
            let mut run = Run::start(&mut value, context, now, memory.began);
            let hue_track = AdjustTrack::Colors(hue_stops());
            let degrees =
                |range: std::ops::RangeInclusive<f64>| AdjustField::new("hue", words.hue.clone(), range, 0.0).unit_kind(UnitKind::Degrees).step(1.0);
            let percent = |key, name: &Label, range: std::ops::RangeInclusive<f64>, default| {
                AdjustField::new(key, name.clone(), range, default).unit_kind(UnitKind::Percent).step(1.0)
            };
            let rows: [(Row, AdjustField, f64); 3] = match range {
                None => {
                    let (hue_range, saturation_low) = if now.colorize { (0.0..=TURN, 0.0) } else { (-180.0..=180.0, -100.0) };
                    let saturation_default = if now.colorize { COLORIZE_SATURATION } else { 0.0 };
                    [
                        (Row::Hue, degrees(hue_range).track(hue_track), now.hue),
                        (
                            Row::Saturation,
                            percent("saturation", &words.saturation, saturation_low..=100.0, saturation_default)
                                .track(AdjustTrack::Colors(vec![tones::GRAY, hue_color(now.hue)])),
                            now.saturation,
                        ),
                        (
                            Row::Lightness,
                            percent("lightness", &words.lightness, -100.0..=100.0, 0.0).track(AdjustTrack::Colors(vec![
                                tones::BLACK,
                                tones::GRAY,
                                tones::WHITE,
                            ])),
                            now.lightness,
                        ),
                    ]
                }
                Some(r) => {
                    let current = now.ranges[r];
                    [
                        (Row::Hue, degrees(-180.0..=180.0).track(hue_track), current.hue),
                        (Row::Saturation, percent("saturation", &words.saturation, -100.0..=100.0, 0.0), current.saturation),
                        (Row::Lightness, percent("lightness", &words.lightness, -100.0..=100.0, 0.0), current.lightness),
                    ]
                }
            };
            for (row, field, shown) in &rows {
                let row = *row;
                run.slider_row(ui, cx, field, *shown, move |now: &HueSaturation, x| now.with_row(range, row, x));
            }
            let colorize = Checkbox::new(Bind::given(now.colorize)).label(words.colorize.clone()).show(ui);
            run.push(cx, colorize, |now, on| now.colorized(*on));

            let input: Vec<DataColor> = (0..SAMPLES).map(|i| hue_color(i as f64 * SAMPLE_STEP)).collect();
            let after: Vec<DataColor> = match adjusted {
                Some(adjusted) => (0..SAMPLES).map(|i| DataColor::new(Color32::from(adjusted(&now, hue_srgba(i as f64 * SAMPLE_STEP))))).collect(),
                None => input.clone(),
            };
            ToneRamp::new(&input).label(words.spectrum_before.clone()).show(ui);
            match range {
                None => {
                    ToneRamp::new(&after).label(words.spectrum_after.clone()).show(ui);
                }
                Some(r) => {
                    let thumbs = [
                        Thumb::new(words.range_start.clone()).face(tones::GRAY),
                        Thumb::new(words.full_start.clone()).face(tones::WHITE),
                        Thumb::new(words.full_end.clone()).face(tones::WHITE),
                        Thumb::new(words.range_end.clone()).face(tones::GRAY),
                    ];
                    let bounds = now.thumbs(r);
                    let row = ThumbRow::new(Bind::given(Mixed::each(bounds)), 0.0..=TURN)
                        .step(1.0)
                        .anchor(Anchor::new(0, MAX_SPAN))
                        .thumbs(thumbs)
                        .off_thumbs(OffThumbs::Ignore)
                        .spans(&RANGE_SPANS)
                        .track_face(TrackFace::new(ToneRamp::new(&after)))
                        .label(words.spectrum_after.clone())
                        .ltr_axis()
                        .show(ui)
                        .edit;
                    run.push(cx, row, move |now: &HueSaturation, row: &[Mixed<f64>; 4]| now.with_thumbs(r, row));
                    let [a, b, c, d] = bounds.map(|degrees| units.text(UnitKind::Degrees, degrees, 0..=0));
                    let readout = words.range_readout.fill(&[("a", &a), ("b", &b), ("c", &c), ("d", &d)]);
                    Text::new(readout).role(TypeRole::Mono).shade(Shade::Faint).show(ui);
                }
            }
            let memory = cx.memory::<HueMemory>();
            memory.began = run.began();
            memory.range = if now.colorize { 0 } else { picked.unwrap_or(at) };
            out = Some((run.finish(), if now.colorize { None } else { picked.unwrap_or(at).checked_sub(1) }));
        });
        let (edit, shown_range) = out.map_or((None, None), |(edit, range)| (Some(edit), range));
        HueSaturationOutput { edit: lifted(edit, shown.output), shown_range }
    }
}

/// The style of the edited mark: its slot and the dot's colour.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct EditedMarkStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The slot: a swatch wide, `space.gap.tight` tall.
    slot: Vec2,
    /// The dot.
    dot: StyleColor,
}

/// The mark under a range swatch: a dot while the range is edited. Its slot is always there, so the row keeps its
/// height when a range is first edited.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(sample = EditedMark::sample)]
struct EditedMark {
    #[common]
    common: Common,
    edited: bool,
}

impl EditedMark {
    fn new(edited: bool) -> Self {
        EditedMark { common: Common::new(), edited }
    }

    fn sample() -> Self {
        EditedMark::new(true)
    }
}

impl Component for EditedMark {
    const DISABLED: DisabledLook = DisabledLook::Faded;
    const ROLE: Role = Role::None;
    const OVERLAYS: bool = false;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = EditedMarkStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: EditedMarkStyleKey) -> EditedMarkStyle {
        let slot = Vec2::new(SwatchMetrics::of(cx).side, cx.tokens.space.gap.tight);
        EditedMarkStyle { metrics: Metrics::new(), slot, dot: cx.color(cx.tokens.color.text.muted) }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        cx.style.slot
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if self.edited {
            let radius = cx.rect.height() * EDITED_DOT / 2.0;
            ui.draw().fill(&PaintShape::Circle(cx.rect.center(), radius), cx.style.dot);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
