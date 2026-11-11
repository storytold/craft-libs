//! [`HueSaturationEditor`]: Master and six colour ranges, Colorize, the two spectrum bars and the four range thumbs
//! that wrap at 360°.

#[cfg(feature = "screen-testing")]
mod stories;

use craft_behavior::Mixed;
use craft_theme::{Hsb, Srgba};
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, EditResponse, Entry, Outcome, Pattern, Props as _, Remember as _, Show as _, ShowContainer as _};
use craft_ui::display::{Shade, Text};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::style::{DataColor, TypeRole};
use craft_ui::value_drag::{Anchor, Thumb, ThumbRow, TrackFace};
use egui::{Color32, Ui};

use super::sliders::{Run, lifted, read, words};
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

/// One of Hue/Saturation's six colour ranges (Reds, Yellows, Greens, Cyans, Blues, Magentas).
#[derive(Clone, Copy, Debug, PartialEq)]
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
#[derive(Clone, Copy, Debug, PartialEq)]
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

/// A fully saturated, full-brightness hue as image data.
fn hue_color(degrees: f64) -> DataColor {
    DataColor::new(Color32::from(Srgba::from_hsb(Hsb { h: degrees as f32, s: 1.0, b: 1.0 })))
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

/// Hue/Saturation.
///
/// - **Range select** (Master, Reds … Magentas): view state of the editor, Master at first. Disabled and forced to
///   Master while Colorize is on. The range shown comes back in [`HueSaturationOutput::shown_range`].
/// - **Master**: Hue (−180..180°; 0..360° with Colorize) over the hue spectrum track, Saturation (−100..100 %;
///   0..100 % with Colorize) over a track from grey to the current hue, Lightness (−100..100 %) over a fixed black,
///   grey, white track. Tracks only while dynamic tracks are on.
/// - **A range**: its Hue, Saturation and Lightness rows, then the range thumbs (below).
/// - **Colorize** (a `Set`): on, Hue wraps into 0..360 and Saturation becomes at least 25; off, a Hue above 180 wraps
///   back by −360.
/// - **Spectrum bars**: the input hues above (37 samples, every 10°), the host's
///   [`adjusted_spectrum`](Self::adjusted_spectrum) below (the same 37 hues after the adjustment; the input bar again
///   without it). Not mirrored in right-to-left layouts: they are scales of the image.
/// - **Range thumbs** (a range shown): four thumbs under the lower bar as one thumb row that wraps through 0° and keeps
///   its order going round from the first thumb. A drag moves a thumb with [`HueRangeValue::move_bound`], whole
///   degrees. Under the bars a readout "a° / b°  ·  c° \ d°", each bound within one turn.
#[must_use = "show the editor with `show`"]
#[derive(Pattern)]
pub struct HueSaturationEditor<'a> {
    label: Label,
    value: Bind<'a, HueSaturation>,
    adjusted: Option<&'a dyn Fn(f64) -> Srgba>,
    range: Option<Option<usize>>,
    context: AdjustContext,
}

impl<'a> HueSaturationEditor<'a> {
    /// The editor named `label` of `value`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, HueSaturation>) -> Self {
        Self { label: label.into(), value, adjusted: None, range: None, context: AdjustContext::dialog(true) }
    }

    /// What a fully saturated, full-brightness hue (degrees 0..=360) becomes under the shown value: the host runs its
    /// engine. Called 37 times a pass.
    pub fn adjusted_spectrum(mut self, adjusted: &'a dyn Fn(f64) -> Srgba) -> Self {
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
        let shown = FieldGroup::new(label).show(ui, |ui, cx| {
            let now = read(&mut value, cx).unwrap_or_default();
            let memory = cx.memory::<HueMemory>().clone();
            // Colorize works on the master only.
            let kept = host_range.map_or(memory.range, |range| range.map_or(0, |r| r + 1));
            let at = if now.colorize { 0 } else { kept.min(6) };
            let range = at.checked_sub(1);
            let names = [&words.reds, &words.yellows, &words.greens, &words.cyans, &words.blues, &words.magentas];
            let mut entries = vec![Entry::new(0usize, words.master.clone())];
            entries.extend(names.iter().enumerate().map(|(i, name)| Entry::new(i + 1, (*name).clone())));
            let mut picked = None;
            Field::new(words.range.clone()).id_salt("range").show(ui, |ui, _| {
                picked = Select::new(words.range.clone(), Bind::given(at), &entries).enabled(!now.colorize).show(ui).value().copied();
            });
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
                Some(adjusted) => (0..SAMPLES).map(|i| DataColor::new(Color32::from(adjusted(i as f64 * SAMPLE_STEP)))).collect(),
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
                        .track_face(TrackFace::new(ToneRamp::new(&after)))
                        .label(words.spectrum_after.clone())
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
