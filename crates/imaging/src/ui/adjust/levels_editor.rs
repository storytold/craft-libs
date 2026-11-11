//! [`LevelsEditor`]: a levels adjustment's histogram, input thumbs and fields, output bar, thumbs
//! and fields, editing one channel's [`Levels`].

#[cfg(feature = "screen-testing")]
mod stories;

use std::ops::RangeInclusive;

use craft_behavior::{Metrics, Mixed, Proposal, Sense, decimals_for_step};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, Outcome, PaintUi, Props as _, Role, WordSetCx as _,
};
use craft_ui::display::{Shade, Text};
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, DataColor, StyleCx, TypeRole};
use craft_ui::text::NumberField;
use craft_ui::value_drag::{Follow, Order, RowOp, RowRules, Thumb, ThumbOp, ThumbRow, TrackFace};
use egui::{Color32, Rect, Response, Vec2, pos2};

use super::style::tones;
use crate::ui::curve::ToneRamp;
use crate::ui::histogram::{ChannelColor, Histogram, HistogramData, PlotHeight, StaleMark};

/// One channel's levels: the input black and white points with the gamma between them, and the
/// output black and white points.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Levels {
    /// The input black point.
    pub in_black: f64,
    /// The midtone gamma: above 1 brightens.
    pub gamma: f64,
    /// The input white point.
    pub in_white: f64,
    /// Output black; above `out_white` it inverts.
    pub out_black: f64,
    /// The output white point.
    pub out_white: f64,
}

/// Levels are equal when every value is, an unset (NaN) value equal to another: the edit
/// machinery compares values to tell a change from none, and a value that is never equal to
/// itself reads as an outside change of every gesture.
impl PartialEq for Levels {
    fn eq(&self, other: &Self) -> bool {
        let same = |a: f64, b: f64| a == b || (a.is_nan() && b.is_nan());
        same(self.in_black, other.in_black)
            && same(self.gamma, other.gamma)
            && same(self.in_white, other.in_white)
            && same(self.out_black, other.out_black)
            && same(self.out_white, other.out_white)
    }
}

impl Levels {
    /// The levels that change nothing on `range`: the input and output ends at its ends, gamma 1.
    pub fn identity(range: RangeInclusive<f64>) -> Self {
        let (start, end) = range.into_inner();
        Self { in_black: start, gamma: 1.0, in_white: end, out_black: start, out_white: end }
    }

    /// Levels of the given values.
    pub fn new(in_black: f64, gamma: f64, in_white: f64, out_black: f64, out_white: f64) -> Self {
        Self { in_black, gamma, in_white, out_black, out_white }
    }

    fn input(&self) -> [Mixed<f64>; 3] {
        Mixed::each([self.in_black, self.gamma, self.in_white])
    }

    fn output(&self) -> [Mixed<f64>; 2] {
        Mixed::each([self.out_black, self.out_white])
    }

    /// The levels with the input values `row` gives; a part that is mixed keeps its value.
    fn with_input(mut self, row: &[Mixed<f64>; 3]) -> Self {
        for (slot, part) in [&mut self.in_black, &mut self.gamma, &mut self.in_white].into_iter().zip(row) {
            if let Some(&value) = part.same() {
                *slot = value;
            }
        }
        self
    }

    /// The levels with the output values `row` gives; a part that is mixed keeps its value.
    fn with_output(mut self, row: &[Mixed<f64>; 2]) -> Self {
        for (slot, part) in [&mut self.out_black, &mut self.out_white].into_iter().zip(row) {
            if let Some(&value) = part.same() {
                *slot = value;
            }
        }
        self
    }
}

/// Which input end an Alt (Option) drag is moving, for the host's clipping view
/// ([`LevelsEditorOutput::clipping`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LevelsClip {
    /// The input black thumb: show what clips to black.
    Shadows,
    /// The input white thumb: show what clips to white.
    Highlights,
}

craft_ui::words! {
    /// The editor's texts, whole phrases in the installed language unless set. Each thumb and the
    /// field under it share a name: one value, two ways to set it.
    #[non_exhaustive]
    pub struct LevelsWords {
        /// The histogram's accessible name.
        histogram: "levels.histogram" = "Histogram";
        /// The caption over the histogram and the input row.
        input: "levels.input" = "Input Levels";
        /// The caption over the output bar.
        output: "levels.output" = "Output Levels";
        /// The input black thumb and field.
        in_black: "levels.inBlack" = "Input black";
        /// The gamma thumb and field.
        gamma: "levels.gamma" = "Gamma";
        /// The input white thumb and field.
        in_white: "levels.inWhite" = "Input white";
        /// The output black thumb and field.
        out_black: "levels.outBlack" = "Output black";
        /// The output white thumb and field.
        out_white: "levels.outWhite" = "Output white";
    }
}

/// Gamma's step, on its thumb and in its field.
const GAMMA_STEP: f64 = 0.01;

/// How far inside the black and white thumbs a gamma drag keeps its thumb, as a fraction of the
/// span between them: it never passes them, and the gamma stays finite.
const GAMMA_T: f64 = 1e-6;

/// The input level the gamma thumb sits at: the level that maps to 50 % grey. `values` are the
/// input row's (black, gamma, white).
pub(crate) fn gamma_at(values: &[f64], gamma: f64) -> f64 {
    let (black, white) = (values[0], values[2]);
    black + (white - black) * 0.5f64.powf(gamma)
}

/// The gamma that puts its thumb at input level `x`.
pub(crate) fn gamma_from(values: &[f64], x: f64) -> f64 {
    let (black, white) = (values[0], values[2]);
    let t = ((x - black) / (white - black).max(f64::MIN_POSITIVE)).clamp(GAMMA_T, 1.0 - GAMMA_T);
    t.ln() / 0.5f64.ln()
}

/// The style of a levels editor: the heights of its rows, the width it asks for and the light end of the
/// output bar.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct LevelsEditorStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The height of a caption.
    caption: f32,
    /// The height of the input thumb row: its thumbs, so the gap under them is the one gap of the editor.
    row: f32,
    /// The height of the output row: its thumbs and the bar they hang from.
    bar_row: f32,
    /// The height of a row of fields.
    fields: f32,
    /// The width of a levels field: three digits.
    field: f32,
    /// The width of the gamma field: a value like 9.99.
    gamma_field: f32,
    /// The width the editor asks for when the parent offers none.
    width: f32,
    /// The light end of the output bar: the channel's bar colour.
    bar: Color32,
}

/// What a levels editor reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct LevelsEditorOutput {
    /// The editor's edit: a thumb's drag, a field's typing or scrub, a key, a reset.
    pub edit: EditResponse<Levels>,
    /// Which input end an Alt drag moves now, for the host's clipping view (Photoshop's Alt-drag on
    /// the black or white input thumb); `None` otherwise.
    pub clipping: Option<LevelsClip>,
}

impl Outcome for LevelsEditorOutput {
    type Value = Levels;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// Edits one channel's [`Levels`] through the edit lifecycle, as PhotoCraft's Levels:
/// `LevelsEditor::new("Levels", Bind::bound(&mut levels)).show(ui)`.
///
/// Top to bottom: the "Input Levels" caption; the channel's histogram with the input row under it
/// (black, gamma and white triangles); the three input fields (black at the start, gamma centred,
/// white at the end); the "Output Levels" caption; a black-to-channel bar with the output row (two
/// triangles) on it; the two output fields. The plot, the two thumb rows and the fields keep this
/// geometry left to right under a right-to-left interface; the captions follow the reading order. The access order is
/// the caption, the input thumb row, the histogram, the input fields, then the output rows: the plot spans the input
/// row's travel, so the row is declared first.
///
/// - The gamma thumb sits at `black + (white − black) · 0.5^gamma` and follows the black and white
///   thumbs; a gamma drag sets `ln t / ln 0.5` of where it is between them, in steps of 0.01 within
///   the [gamma range](Self::gamma_range). The arrows move it the way they point (Right lowers the
///   gamma).
/// - The input ends stay [`min_gap`](Self::min_gap) steps apart; the output ends may cross (output
///   black above white inverts).
/// - Each thumb is a Tab stop. A press grabs a thumb where it is, a press off the thumbs moves the
///   nearest one there. A double click or Cmd/Ctrl+Backspace sets a thumb's default; Esc cancels a
///   drag.
/// - Each field shows its thumb's value and takes values within the thumb's bounds.
/// - While Alt is held during an input black or white drag, [`LevelsEditorOutput::clipping`] says
///   which end, for the host's clipping view; Alt is not the fine scale on those two thumbs.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct LevelsEditor<'a> {
    #[common]
    common: Common,
    levels: Bind<'a, Levels>,
    /// The levels' range, ascending.
    #[prop(quantity = "levels")]
    range: RangeInclusive<f64>,
    /// The step levels snap to and the fields step by.
    #[prop(quantity = "step")]
    step: f64,
    /// The gamma's range.
    #[prop(quantity = "gamma")]
    gamma_range: RangeInclusive<f64>,
    /// How many steps apart the input ends stay.
    #[prop(quantity = "steps")]
    min_gap: f64,
    /// The edited channel's colour: the light end of the output bar.
    #[style_key]
    #[prop]
    channel: ChannelColor,
    /// What a screen reader reads after the label: the channel ("Red channel").
    #[prop(into)]
    value: Option<Label>,
    /// The channel's histogram; without it the plot is empty.
    #[prop]
    histogram: Option<&'a HistogramData>,
    /// The histogram is older than the image.
    #[prop]
    stale: bool,
    /// Whether the "Input Levels" and "Output Levels" captions show.
    #[prop]
    captions: bool,
    clip: Option<LevelsClip>,
}

impl<'a> LevelsEditor<'a> {
    /// An editor named `label` ("Levels") of the host's `levels`.
    pub fn new(label: impl Into<Label>, levels: Bind<'a, Levels>) -> Self {
        LevelsEditor {
            common: Common::new(),
            levels,
            range: 0.0..=255.0,
            step: 1.0,
            gamma_range: 0.01..=9.99,
            min_gap: 2.0,
            channel: ChannelColor::Neutral,
            value: None,
            histogram: None,
            stale: false,
            captions: true,
            clip: None,
        }
        .label(label)
    }

    fn ends(&self) -> (f64, f64) {
        (self.range.start().min(*self.range.end()), self.range.start().max(*self.range.end()))
    }

    /// The input row showing `shown`: black and white ordered a gap apart, the gamma thumb placed by them.
    fn input_row<'c>(&self, words: &LevelsWords, shown: Option<Levels>) -> ThumbRow<'c, 3> {
        let (start, end) = self.ends();
        let gap = self.min_gap * self.step;
        let ordered = |below, above| Order::between(below, above, gap);
        let thumbs = [
            Thumb::new(words.in_black.clone()).face(tones::BLACK).order(ordered(None, Some(2))).reset(start).alt_reports(),
            Thumb::new(words.gamma.clone())
                .face(tones::GRAY)
                .follows(&self.gamma_range, GAMMA_STEP, Follow::new(gamma_at, gamma_from))
                .reset(1.0)
                .unset_at(1.0)
                .falling(),
            Thumb::new(words.in_white.clone()).face(tones::WHITE).order(ordered(Some(0), None)).reset(end).alt_reports(),
        ];
        let bind = match shown {
            Some(levels) => Bind::given(levels.input()),
            None => Bind::given(Mixed::Mixed),
        };
        ThumbRow::new(bind, self.range.clone()).step(self.step).thumbs(thumbs).label(words.input.clone()).ltr_axis()
    }

    /// The output row showing `shown`, hanging from a black to channel bar.
    fn output_row<'c>(&self, words: &LevelsWords, shown: Option<Levels>, bar: &'c [DataColor; 2]) -> ThumbRow<'c, 2> {
        let (start, end) = self.ends();
        let thumbs =
            [Thumb::new(words.out_black.clone()).face(tones::BLACK).reset(start), Thumb::new(words.out_white.clone()).face(tones::WHITE).reset(end)];
        let bind = match shown {
            Some(levels) => Bind::given(levels.output()),
            None => Bind::given(Mixed::Mixed),
        };
        ThumbRow::new(bind, self.range.clone())
            .step(self.step)
            .thumbs(thumbs)
            .track_face(TrackFace::new(ToneRamp::new(bar)))
            .label(words.output.clone())
            .ltr_axis()
    }

    /// The histogram plot over `data`.
    fn plot<'c>(&self, words: &LevelsWords, data: &'c HistogramData) -> Histogram<'c> {
        let plot = Histogram::new(words.histogram.clone(), data).height(PlotHeight::Standard).bordered();
        if self.stale { plot.stale(StaleMark::Mark(Label::Empty)) } else { plot }
    }

    /// A field named `label` showing `value`, within `range`.
    fn field<'c>(&self, label: Label, value: f64, range: RangeInclusive<f64>, step: f64) -> NumberField<'c> {
        let shown = Some(value).filter(|value| value.is_finite());
        NumberField::new(label, Bind::given(Mixed::Same(shown)))
            .range(range)
            .step(step)
            .snap_typed(true)
            .commit_decimals(decimals_for_step(step))
            .ltr_axis()
    }
}

/// The rects of the editor's rows in a room of `size`, from its top left corner.
struct Rows {
    caption_in: Rect,
    plot: Rect,
    input: Rect,
    fields_in: Rect,
    caption_out: Rect,
    output: Rect,
    fields_out: Rect,
}

/// The height of everything but the plot, with the gaps between all rows but the plot and the input row, which touch.
fn fixed_height(captions: bool, style: &LevelsEditorStyle) -> f32 {
    let caption = if captions { style.caption } else { 0.0 };
    let rows = if captions { 6.0 } else { 4.0 };
    2.0 * caption + style.row + style.bar_row + 2.0 * style.fields + style.metrics.gap * (rows - 1.0)
}

/// The rows in a room of `size`: the plot takes what the other rows leave.
fn arrange(size: Vec2, captions: bool, style: &LevelsEditorStyle) -> Rows {
    let gap = style.metrics.gap;
    let plot = (size.y - fixed_height(captions, style)).max(0.0);
    let mut y = 0.0;
    let mut row = |height: f32, shown: bool, gap: f32| {
        let rect = Rect::from_min_size(pos2(0.0, y), Vec2::new(size.x, height));
        if shown {
            y += height + gap;
        }
        rect
    };
    let caption_in = row(style.caption, captions, gap);
    let plot = row(plot, true, 0.0);
    let input = row(style.row, true, gap);
    let fields_in = row(style.fields, true, gap);
    let caption_out = row(style.caption, captions, gap);
    let output = row(style.bar_row, true, gap);
    let fields_out = row(style.fields, true, gap);
    Rows { caption_in, plot, input, fields_in, caption_out, output, fields_out }
}

/// The rects of `count` fields in a row: the first at the start, the last at the end, a middle one centred.
fn field_rects(row: Rect, width: f32, middle: f32, count: usize) -> Vec<Rect> {
    let width = width.min(row.width() / count as f32);
    let rect = |x: f32, width: f32| Rect::from_min_size(pos2(x, row.top()), Vec2::new(width, row.height()));
    match count {
        3 => {
            let middle = middle.min(row.width() / 3.0);
            vec![rect(row.left(), width), rect(row.center().x - middle / 2.0, middle), rect(row.right() - width, width)]
        }
        _ => vec![rect(row.left(), width), rect(row.right() - width, width)],
    }
}

/// The range the field of input thumb `thumb` takes with the thumbs at `placed`: an input end's stops the gap short of
/// the other end.
fn input_range(editor: &LevelsEditor<'_>, thumb: usize, placed: [f64; 3]) -> RangeInclusive<f64> {
    let (start, end) = editor.ends();
    let gap = editor.min_gap * editor.step;
    match thumb {
        0 => start..=(placed[2] - gap).clamp(start, end),
        1 => editor.gamma_range.clone(),
        _ => (placed[0] + gap).clamp(start, end)..=end,
    }
}

/// `levels` with the input thumb `thumb` set to `value` as a drag of it would stop, so the input ends keep their gap.
fn set_input(rules: &RowRules<'_, 3>, levels: &Levels, thumb: usize, value: f64) -> Levels {
    levels.with_input(&rules.apply(&RowOp::new(thumb, ThumbOp::Set(value)), &levels.input()))
}

/// `levels` with the output thumb `thumb` set to `value`.
fn set_output(rules: &RowRules<'_, 2>, levels: &Levels, thumb: usize, value: f64) -> Levels {
    levels.with_output(&rules.apply(&RowOp::new(thumb, ThumbOp::Set(value)), &levels.output()))
}

impl Component for LevelsEditor<'_> {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = LevelsEditorStyle;
    type Output = LevelsEditorOutput;

    fn style(cx: &StyleCx<'_>, key: LevelsEditorStyleKey) -> LevelsEditorStyle {
        let control = cx.control();
        let row = control.height;
        LevelsEditorStyle {
            metrics: Metrics::new().gap(control.gap),
            caption: row,
            row: control.icon,
            bar_row: control.icon + cx.tokens.size.ramp.bar,
            fields: row,
            field: cx.text_width(TypeRole::Body, "000") + 2.0 * control.pad_x,
            gamma_field: cx.text_width(TypeRole::Body, "0.00") + 2.0 * control.pad_x,
            width: cx.tokens.size.plot.width,
            bar: key.channel.bar(cx),
        }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let words = cx.words::<LevelsWords>();
        let width = cx.room.unwrap_or(cx.style.width);
        let empty = HistogramData::tone(&[0u32; 256], self.channel);
        let mut plot = self.plot(&words, self.histogram.unwrap_or(&empty));
        let plot = cx.child("plot", &mut plot, Proposal::Width(width)).y;
        Vec2::new(width, fixed_height(self.captions, cx.style) + plot)
    }

    fn bind(&mut self) -> Option<BindMut<'_, Levels>> {
        Some(self.levels.reborrow())
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if let Some(value) = &self.value {
            access.value(cx.env.resolve(value).as_str());
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let words = cx.words::<LevelsWords>();
        let shown = cx.value().copied();
        let rows = arrange(cx.rect.size(), self.captions, cx.style);
        let at = |rect: Rect| rect.translate(cx.rect.min.to_vec2());
        let base = shown.unwrap_or_else(|| Levels::identity(self.range.clone()));
        let in_row = self.input_row(&words, shown);
        let in_rules = in_row.rules();
        let bar = [tones::BLACK, DataColor::new(cx.style.bar)];
        let out_row = self.output_row(&words, shown, &bar);
        let out_rules = out_row.rules();
        let placed = in_rules.placed(&base.input());

        if self.captions {
            let caption = |label: &Label| Text::new(label.clone()).role(TypeRole::Caption).shade(Shade::Muted);
            ui.child("caption-in", at(rows.caption_in), caption(&words.input));
            ui.child("caption-out", at(rows.caption_out), caption(&words.output));
        }
        let input = ui.child("input", at(rows.input), self.input_row(&words, shown));
        // The plot spans the travel of the input thumbs, so bin 0 and the last bin lie under the thumbs at the range ends.
        let empty = HistogramData::tone(&[0u32; 256], self.channel);
        let plot = Rect::from_x_y_ranges(input.travel, at(rows.plot).y_range());
        ui.child("plot", plot, self.plot(&words, self.histogram.unwrap_or(&empty)));
        let fields = field_rects(rows.fields_in, cx.style.field, cx.style.gamma_field, 3);
        let labels = [words.in_black.clone(), words.gamma.clone(), words.in_white.clone()];
        let values = [base.in_black, base.gamma, base.in_white];
        let typed: Vec<_> = (0..3)
            .map(|i| {
                let step = if i == 1 { GAMMA_STEP } else { self.step };
                let field = self.field(labels[i].clone(), values[i], input_range(self, i, placed), step);
                let field = if i == 1 { field.fixed_decimals(decimals_for_step(GAMMA_STEP)) } else { field };
                ui.child(("in-field", i), at(fields[i]), field)
            })
            .collect();

        let output = ui.child("output", at(rows.output), self.output_row(&words, shown, &bar));
        let fields = field_rects(rows.fields_out, cx.style.field, 0.0, 2);
        let (low, high) = self.ends();
        let out_labels = [words.out_black.clone(), words.out_white.clone()];
        let out_values = [base.out_black, base.out_white];
        let typed_out: Vec<_> = (0..2)
            .map(|i| {
                let field = self.field(out_labels[i].clone(), out_values[i], low..=high, self.step);
                ui.child(("out-field", i), at(fields[i]), field)
            })
            .collect();

        cx.adopt(&input.edit, &base, |value, mine| *mine = mine.with_input(value));
        cx.adopt(&output.edit, &base, |value, mine| *mine = mine.with_output(value));
        for (i, edit) in typed.iter().enumerate() {
            cx.adopt(edit, &base, |value, mine| {
                if let Some(value) = value {
                    *mine = set_input(&in_rules, mine, i, *value);
                }
            });
        }
        for (i, edit) in typed_out.iter().enumerate() {
            cx.adopt(edit, &base, |value, mine| {
                if let Some(value) = value {
                    *mine = set_output(&out_rules, mine, i, *value);
                }
            });
        }
        self.clip = input.held.and_then(|held| match held.thumb {
            0 if held.mods.alt => Some(LevelsClip::Shadows),
            2 if held.mods.alt => Some(LevelsClip::Highlights),
            _ => None,
        });
    }

    fn finish(self, cx: Cx<'_, Self>) -> LevelsEditorOutput {
        LevelsEditorOutput { edit: cx.edit_response(), clipping: self.clip }
    }
}
