//! The Eyedropper tool (F04.01): `SampleSize`, `SampleSource` and `EyedropperOptions` as data, `EyedropperBar` for the
//! options bar and `EyedropperTool` for the canvas.
//!
//! ```ignore
//! Bar::new("Eyedropper options", BarKind::Options).show(ui, |ui, _| {
//!     EyedropperBar::new(&mut options).show(ui);
//! });
//! let mut tool = EyedropperTool::new(options, |doc, size, source| composite_color(doc, size, source)).foreground(foreground);
//! tool_host.run(ui, &canvas, &mut tool);
//! if let Some(pick) = tool.take() {
//!     *(if pick.background { &mut background } else { &mut foreground }) = pick.color;
//! }
//! ```

use std::f32::consts::{PI, TAU};

use craft_behavior::kurbo::Point;
use craft_behavior::{CancelReason, Cursor, EditEvents, IconId, PaintedCursor};
use craft_host::target::Os;
use craft_theme::Srgba;
use craft_ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolPath, ToolPointer};
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, EditResponse, Entry, Outcome, Pattern, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::form::Field;
use craft_ui::pickers::Select;
use craft_ui::primitives::{Draw, PaintShape, StrokeWidth};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::DataColor;
use egui::{Color32, Pos2, Response, Ui, Vec2, vec2};

#[cfg(feature = "screen-testing")]
mod stories;

craft_ui::words! {
    /// The Eyedropper options bar's words, English by default.
    #[non_exhaustive]
    pub struct EyedropperWords {
        sample_size: "eyedropper.sampleSize" = "Sample Size:";
        size: "eyedropper.size" = "Size";
        point_sample: "eyedropper.pointSample" = "Point Sample";
        /// `{n}` is the side of the averaged square in pixels.
        average: template "eyedropper.average" = "{n} by {n} Average";
        sample: "eyedropper.sample" = "Sample:";
        source: "eyedropper.source" = "Source";
        current_layer: "eyedropper.currentLayer" = "Current Layer";
        current_and_below: "eyedropper.currentAndBelow" = "Current & Below";
        all_layers: "eyedropper.allLayers" = "All Layers";
        all_layers_no_adjustments: "eyedropper.allLayersNoAdjustments" = "All Layers No Adjustments";
        current_and_below_no_adjustments: "eyedropper.currentAndBelowNoAdjustments" = "Current & Below No Adjustments";
        show_ring: "eyedropper.showRing" = "Show Sampling Ring";
        /// `{key}` is the modifier that picks the background colour.
        hint: template "eyedropper.hint" = "Click to sample the foreground color  ·  {key}-click for background";
    }
}

/// How many pixels around the pointer the eyedropper averages: Photoshop's Sample Size list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum SampleSize {
    /// The pixel under the pointer.
    #[default]
    Point,
    /// 3 by 3 pixels.
    Avg3,
    /// 5 by 5 pixels.
    Avg5,
    /// 11 by 11 pixels.
    Avg11,
    /// 31 by 31 pixels.
    Avg31,
    /// 51 by 51 pixels.
    Avg51,
    /// 101 by 101 pixels.
    Avg101,
}

impl SampleSize {
    /// Every size in menu order.
    pub const ALL: [SampleSize; 7] =
        [SampleSize::Point, SampleSize::Avg3, SampleSize::Avg5, SampleSize::Avg11, SampleSize::Avg31, SampleSize::Avg51, SampleSize::Avg101];

    /// The side of the sampled square in pixels.
    pub fn pixels(self) -> u32 {
        match self {
            SampleSize::Point => 1,
            SampleSize::Avg3 => 3,
            SampleSize::Avg5 => 5,
            SampleSize::Avg11 => 11,
            SampleSize::Avg31 => 31,
            SampleSize::Avg51 => 51,
            SampleSize::Avg101 => 101,
        }
    }

    /// The menu text: "Point Sample" or "11 by 11 Average".
    pub fn label(self, words: &EyedropperWords) -> Label {
        match self {
            SampleSize::Point => words.point_sample.clone(),
            average => Label::from(words.average.fill(&[("n", &average.pixels().to_string())])),
        }
    }
}

/// Which layers the eyedropper reads: Photoshop's Sample list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum SampleSource {
    /// The current layer.
    Current,
    /// The current layer and the ones below.
    CurrentAndBelow,
    /// Every layer.
    #[default]
    All,
    /// Every layer but adjustments.
    AllNoAdjustments,
    /// The current layer and the ones below, but no adjustments.
    CurrentAndBelowNoAdjustments,
}

impl SampleSource {
    /// Every source in menu order.
    pub const ALL: [SampleSource; 5] = [
        SampleSource::Current,
        SampleSource::CurrentAndBelow,
        SampleSource::All,
        SampleSource::AllNoAdjustments,
        SampleSource::CurrentAndBelowNoAdjustments,
    ];

    /// The stable id of the source, for settings and scripts.
    pub fn id(self) -> &'static str {
        match self {
            SampleSource::Current => "current",
            SampleSource::CurrentAndBelow => "currentAndBelow",
            SampleSource::All => "all",
            SampleSource::AllNoAdjustments => "allNoAdjustments",
            SampleSource::CurrentAndBelowNoAdjustments => "currentAndBelowNoAdjustments",
        }
    }

    /// The menu text.
    pub fn label(self, words: &EyedropperWords) -> Label {
        match self {
            SampleSource::Current => &words.current_layer,
            SampleSource::CurrentAndBelow => &words.current_and_below,
            SampleSource::All => &words.all_layers,
            SampleSource::AllNoAdjustments => &words.all_layers_no_adjustments,
            SampleSource::CurrentAndBelowNoAdjustments => &words.current_and_below_no_adjustments,
        }
        .clone()
    }
}

/// The Eyedropper's options bar values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct EyedropperOptions {
    /// How many pixels are averaged.
    pub size: SampleSize,
    /// Which layers are read.
    pub source: SampleSource,
    /// Whether the sampling ring shows while the button is down.
    pub show_ring: bool,
}

impl Default for EyedropperOptions {
    /// Point Sample, All Layers, ring on.
    fn default() -> Self {
        EyedropperOptions { size: SampleSize::Point, source: SampleSource::All, show_ring: true }
    }
}

/// The Eyedropper's members of an options bar: Sample Size and Sample as selects, Show Sampling Ring as a check box,
/// and the hint. Show it inside the options `Bar`; the members write into `options`.
#[must_use = "show the bar with `show`"]
#[derive(Pattern)]
pub struct EyedropperBar<'a> {
    options: &'a mut EyedropperOptions,
}

/// What [`EyedropperBar::show`] reports: the edit of each member.
#[derive(Debug)]
#[non_exhaustive]
pub struct EyedropperBarOutput {
    /// Sample Size.
    pub size: EditResponse<SampleSize>,
    /// Sample.
    pub source: EditResponse<SampleSource>,
    /// Show Sampling Ring.
    pub ring: EditResponse<bool>,
}

impl Outcome for EyedropperBarOutput {
    type Value = EyedropperOptions;

    fn response(&self) -> &Response {
        &self.size.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.size.story_facts()
    }
}

impl<'a> EyedropperBar<'a> {
    /// The members over `options`.
    pub fn new(options: &'a mut EyedropperOptions) -> Self {
        EyedropperBar { options }
    }

    /// Shows the members.
    pub fn show(self, ui: &mut Ui) -> EyedropperBarOutput {
        let EyedropperBar { options } = self;
        let env = Env::of(ui);
        let words = env.words::<EyedropperWords>();
        let sizes: Vec<Entry<SampleSize>> = SampleSize::ALL.iter().map(|size| Entry::new(*size, size.label(&words))).collect();
        let sources: Vec<Entry<SampleSource>> = SampleSource::ALL.iter().map(|source| Entry::new(*source, source.label(&words))).collect();
        let size =
            Field::new(words.sample_size.clone()).show(ui, |ui, _| Select::new(words.size.clone(), Bind::bound(&mut options.size), &sizes).show(ui));
        let source =
            Field::new(words.sample.clone()).show(ui, |ui, _| Select::new(words.source.clone(), Bind::bound(&mut options.source), &sources).show(ui));
        let ring = Checkbox::new(Bind::bound(&mut options.show_ring)).label(words.show_ring.clone()).show(ui);
        let key = if env.view().shell().os == Os::Mac { "Option" } else { "Alt" };
        Text::new(Label::from(words.hint.fill(&[("key", key)]))).show(ui);
        // A parent's sizing pass places a field without running its body: that run reports no edit.
        let (size_response, source_response) = (size.response().clone(), source.response().clone());
        EyedropperBarOutput {
            size: size.inner.unwrap_or_else(|| EditResponse::lifted(size_response, EditEvents::new(), None)),
            source: source.inner.unwrap_or_else(|| EditResponse::lifted(source_response, EditEvents::new(), None)),
            ring,
        }
    }
}

/// The picked colour, and whether it is for the background.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct EyedropperPick {
    /// The sampled colour.
    pub color: Srgba,
    /// Alt was held: the colour is for the background.
    pub background: bool,
}

/// The pipette icon's tip, in the icon's own 24-unit box.
const HOTSPOT: [f32; 2] = [2.0, 22.0];
/// The ring's outer radius in points.
const RING_OUTER_RADIUS: f32 = 105.0;
/// The width of each of the ring's two colour bands in points.
const RING_BAND: f32 = 17.0;
/// How many segments approximate a ring.
const RING_SEGMENTS: usize = 48;

/// The Eyedropper tool: a press samples the foreground (Alt: the background) and a drag resamples. The sampling ring
/// (the new colour over the current one) paints while the button is held.
pub struct EyedropperTool<'a> {
    options: EyedropperOptions,
    sample: Box<dyn FnMut([f64; 2], SampleSize, SampleSource) -> Option<Srgba> + 'a>,
    held: Option<(Pos2, Srgba)>,
    picked: Option<EyedropperPick>,
    foreground: Srgba,
}

impl<'a> EyedropperTool<'a> {
    /// A tool sampling through `sample` with the document point, the sample size and the source (`None` outside the
    /// document).
    pub fn new(options: EyedropperOptions, sample: impl FnMut([f64; 2], SampleSize, SampleSource) -> Option<Srgba> + 'a) -> Self {
        EyedropperTool { options, sample: Box::new(sample), held: None, picked: None, foreground: Srgba::rgb(0.0, 0.0, 0.0) }
    }

    /// The current foreground colour: the lower half of the sampling ring.
    #[must_use]
    pub fn foreground(mut self, color: Srgba) -> Self {
        self.foreground = color;
        self
    }

    /// The newest pick, once.
    pub fn take(&mut self) -> Option<EyedropperPick> {
        self.picked.take()
    }

    fn sample_at(&mut self, pointer: &ToolPointer, cx: &ToolContext<'_>) {
        let Some(space) = cx.space() else { return };
        let Point { x, y } = space.view.to_doc(pointer.pos);
        match (self.sample)([x, y], self.options.size, self.options.source) {
            Some(color) => {
                self.held = Some((pointer.pos, color));
                self.picked = Some(EyedropperPick { color, background: pointer.mods.alt });
            }
            None => self.held = None,
        }
    }
}

/// The points of the arc of `radius` from angle `from` to `to` around `center`.
fn arc(center: Pos2, radius: f32, from: f32, to: f32) -> Vec<Pos2> {
    (0..=RING_SEGMENTS)
        .map(|i| {
            let angle = from + (to - from) * i as f32 / RING_SEGMENTS as f32;
            center + vec2(angle.cos(), angle.sin()) * radius
        })
        .collect()
}

/// A band of `color` between the radii `inner` and `outer` over the angles `from` to `to`, as convex quads.
fn band(draw: &Draw<'_>, center: Pos2, (inner, outer): (f32, f32), (from, to): (f32, f32), color: Srgba) {
    let (near, far) = (arc(center, inner, from, to), arc(center, outer, from, to));
    for at in 0..RING_SEGMENTS {
        let quad = vec![near[at], far[at], far[at + 1], near[at + 1]];
        draw.data_fill(&PaintShape::Polygon(quad), DataColor::new(Color32::from(color)));
    }
}

impl CanvasTool for EyedropperTool<'_> {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        self.picked = None;
        self.sample_at(pointer, cx);
        cx.request_repaint();
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.sample_at(pointer, cx);
        cx.request_repaint();
    }

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {
        self.held = None;
    }

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {
        self.held = None;
        self.picked = None;
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        let (true, Some((center, new)), Some(style)) = (self.options.show_ring, self.held, cx.style()) else { return };
        let tokens = style.tokens;
        let inner = RING_OUTER_RADIUS - 2.0 * RING_BAND;
        band(draw, center, (inner, inner + RING_BAND), (PI, TAU), new);
        band(draw, center, (inner, inner + RING_BAND), (0.0, PI), self.foreground);
        let grey = style.color(tokens.color.overlay.grid);
        let ring = |radius: f32| {
            let mut points = arc(center, radius, 0.0, TAU);
            points.truncate(RING_SEGMENTS);
            points.push(points[0]);
            PaintShape::Path(points)
        };
        draw.stroke(&ring(inner + RING_BAND * 1.5), StrokeWidth::Thick, grey);
        draw.stroke(&ring(RING_OUTER_RADIUS), StrokeWidth::Hairline, style.color(tokens.color.overlay.halo));
        draw.stroke(&ring(inner), StrokeWidth::Hairline, style.color(tokens.color.overlay.halo));
    }

    /// The pipette with its tip on the sampled pixel (the icon's lower left, in its 24-unit box).
    fn cursor(&self, _: &ToolContext<'_>) -> Cursor {
        PaintedCursor::tool(IconId::new("eyedropper"), Vec2::from(HOTSPOT), None).into()
    }

    fn autoscrolls(&self) -> bool {
        false
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Color Picker command")
    }
}
