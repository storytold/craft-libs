//! `TextFrameMarks`: InDesign's View > Show Hidden Characters and Extras over text frames, a canvas control that only
//! paints (it has no parts, so a press never lands on it). Hidden characters: · spaces, » tabs, ¬ forced line breaks,
//! ° no-break spaces, ¶ paragraph ends, # the end of a story. Extras: the overset mark (a red + in the out port) on a
//! frame whose story does not fit, and frame outlines with their in and out ports.

#[cfg(feature = "screen-testing")]
mod stories;

use craft_behavior::kurbo::{Point, Rect};
use craft_behavior::{CanvasView, Input, Metrics, Sense};
use craft_theme::variant::Tone;
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PaintUi, Props, Role};
use craft_ui::fonts::FontRef;
use craft_ui::primitives::{DocRun, DocText, PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, DataColor, StyleColor, StyleCx};
use egui::{CornerRadius, Pos2, Response, Vec2};

/// A hidden-character mark's size as a share of the line's ascent (DesignCraft).
pub const MARK_SHARE: f32 = 0.75;

/// The smallest and largest size a hidden-character mark takes at any zoom: limits of document text drawn at a zoom,
/// in screen points, not interface sizes.
const MARK_SIZES: std::ops::RangeInclusive<f32> = 6.0..=40.0;

/// The threaded triangle's corners as shares of the port's side from its centre: the two back corners and the tip.
const TRIANGLE: [(f32, f32); 3] = [(-0.235, -0.295), (0.295, 0.0), (-0.235, 0.295)];

/// A hidden-character mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HiddenMark {
    /// · (centred on the space).
    Space,
    /// ».
    Tab,
    /// ¬.
    LineBreak,
    /// °.
    NoBreakSpace,
    /// ¶.
    ParagraphEnd,
    /// #.
    StoryEnd,
}

impl HiddenMark {
    /// The character drawn for the mark.
    pub fn glyph(self) -> &'static str {
        match self {
            HiddenMark::Space => "·",
            HiddenMark::Tab => "»",
            HiddenMark::LineBreak => "¬",
            HiddenMark::NoBreakSpace => "°",
            HiddenMark::ParagraphEnd => "¶",
            HiddenMark::StoryEnd => "#",
        }
    }

    /// The mark for a character, if it has one (the paragraph and story ends come from the layout, not a character).
    pub fn of(c: char) -> Option<HiddenMark> {
        match c {
            ' ' => Some(HiddenMark::Space),
            '\t' => Some(HiddenMark::Tab),
            '\u{2028}' => Some(HiddenMark::LineBreak),
            '\u{A0}' => Some(HiddenMark::NoBreakSpace),
            _ => None,
        }
    }
}

/// One mark to draw, in frame points: where it sits on the baseline, whether it is centred on that x (spaces) or starts
/// at it, and the line's ascent (its size follows the text's).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PlacedMark {
    /// The kind of mark.
    pub mark: HiddenMark,
    /// Where the mark sits on the baseline, in frame points.
    pub at: Point,
    /// Whether the mark is centred on `at.x` (spaces) or starts at it.
    pub centred: bool,
    /// The ascent of the line the mark is on, points.
    pub ascent: f64,
}

impl PlacedMark {
    /// A `mark` on the baseline at `at`, centred on it or starting at it, on a line of `ascent`.
    pub fn new(mark: HiddenMark, at: Point, centred: bool, ascent: f64) -> Self {
        PlacedMark { mark, at, centred, ascent }
    }
}

/// One text frame's extras: its outline, ports and whether its story is overset there.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct FrameExtras {
    /// The frame's inner bounds on the document (points).
    pub bounds: Rect,
    /// The frame continues a story from another frame (in port filled with a triangle).
    pub threaded_in: bool,
    /// The story continues into another frame (out port filled with a triangle).
    pub threaded_out: bool,
    /// The frame ends a story that does not fit: the out port shows the overset +.
    pub overset: bool,
}

impl FrameExtras {
    /// A frame with `bounds`, threaded nowhere and not overset.
    pub fn new(bounds: Rect) -> Self {
        FrameExtras { bounds, threaded_in: false, threaded_out: false, overset: false }
    }

    /// The frame continues a story from another frame.
    #[must_use]
    pub fn threaded_in(mut self, on: bool) -> Self {
        self.threaded_in = on;
        self
    }

    /// The story continues into another frame.
    #[must_use]
    pub fn threaded_out(mut self, on: bool) -> Self {
        self.threaded_out = on;
        self
    }

    /// The frame ends a story that does not fit.
    #[must_use]
    pub fn overset(mut self, on: bool) -> Self {
        self.overset = on;
        self
    }
}

/// The style of the frame marks: the port's side and inset, and the overset colour.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TextFrameMarksStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The side of a port.
    port: f32,
    /// How far along its edge a port's centre lies.
    port_inset: f32,
    overset: StyleColor,
}

/// The marks and extras of the frames on screen, painted over the page after its content.
///
/// - Marks are drawn in the frame's layer colour, at `ascent` x [`MARK_SHARE`] x zoom, kept between 6 and 40 points.
/// - Ports are squares of the style's `port` side filled with the paper colour and outlined in the layer colour: the
///   in port `port_inset` right of the top-left corner on the top edge, the out port `port_inset` left of the
///   bottom-right corner on the bottom edge (DesignCraft's `port_pos`). A threaded port holds a small triangle in the
///   layer colour; the overset port a +, drawn on every frame that ends an overset story, not only selected ones.
/// - Nothing here takes input or focus.
#[derive(Component, Props)]
#[component(canvas_control)]
pub struct TextFrameMarks<'a> {
    #[common]
    common: Common,
    /// The view that maps frame points to the screen.
    view: CanvasView,
    frames: &'a [FrameExtras],
    /// The hidden-character marks (View > Show Hidden Characters); none show without them.
    #[prop]
    hidden_characters: &'a [PlacedMark],
    /// The layer colour the marks and outlines take.
    color: DataColor,
    /// The colour the ports are filled with: the document's paper.
    paper: DataColor,
}

impl<'a> TextFrameMarks<'a> {
    /// The extras of `frames` seen through `view`, named `label`, in the layer colour `color`, with ports filled with
    /// the document's `paper`.
    pub fn new(label: impl Into<Label>, view: CanvasView, frames: &'a [FrameExtras], color: DataColor, paper: DataColor) -> Self {
        TextFrameMarks { common: Common::new(), view, frames, hidden_characters: &[], color, paper }.label(label)
    }
}

impl Component for TextFrameMarks<'_> {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const INPUT: Input = Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = TextFrameMarksStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: TextFrameMarksStyleKey) -> TextFrameMarksStyle {
        TextFrameMarksStyle {
            metrics: Metrics::new(),
            port: cx.tokens.size.handle.md,
            port_inset: cx.tokens.space.pad.control,
            overset: cx.tone(Tone::Bad),
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = &cx.style;
        let draw = ui.draw();
        let zoom = self.view.zoom as f32;
        for placed in self.hidden_characters {
            let size = (placed.ascent as f32 * MARK_SHARE * zoom).clamp(*MARK_SIZES.start(), *MARK_SIZES.end());
            let run = [DocRun::new(placed.mark.glyph(), FontRef::proportional(), size, self.color)];
            let at = self.view.to_screen(placed.at);
            let left = if placed.centred { at.x - size / 2.0 } else { at.x };
            let room = egui::Rect::from_min_size(Pos2::new(left, at.y - size), Vec2::splat(size));
            draw.doc(room, &DocText::new(&run));
        }
        for frame in self.frames {
            let corners = self.view.quad(frame.bounds);
            draw.stroke(&PaintShape::Polygon(corners.to_vec()), StrokeWidth::Hairline, self.color);
            let in_port = port_centre(corners[0], corners[1], style.port_inset);
            let out_port = port_centre(corners[2], corners[3], style.port_inset);
            for (centre, threaded, is_out) in [(in_port, frame.threaded_in, false), (out_port, frame.threaded_out, true)] {
                let square = PaintShape::Rect(egui::Rect::from_center_size(centre, Vec2::splat(style.port)), CornerRadius::ZERO);
                draw.data_fill(&square, self.paper);
                draw.stroke(&square, StrokeWidth::Hairline, self.color);
                if is_out && frame.overset {
                    let arm = style.port / 2.0;
                    for along in [Vec2::new(arm, 0.0), Vec2::new(0.0, arm)] {
                        draw.stroke(&PaintShape::Path(vec![centre - along, centre + along]), StrokeWidth::Medium, style.overset);
                    }
                } else if threaded {
                    let points = TRIANGLE.iter().map(|&(x, y)| centre + Vec2::new(x, y) * style.port).collect();
                    draw.data_fill(&PaintShape::Polygon(points), self.color);
                }
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// The port's centre on the edge from `corner` toward `other`: `inset` along it, never past the edge's middle on a
/// small frame.
fn port_centre(corner: Pos2, other: Pos2, inset: f32) -> Pos2 {
    let edge = other - corner;
    let length = edge.length();
    if length > f32::EPSILON { corner + edge * (inset.min(length / 2.0) / length) } else { corner }
}
