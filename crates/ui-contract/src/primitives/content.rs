//! Content painters and document text.

use super::draw::{Draw, DrawEnv, StrokeWidth};
use super::record::PaintRecord;
use super::text::{HAlign, TextLayout, aligned_rows};
use crate::fonts::{FontRef, SEMIBOLD, stack_facts};
use crate::runtime::egui::text_layout;
use crate::runtime::env::Env;
use crate::style::{DataColor, Ink, StyleCx, UiTokens, ink_color};
use craft_i18n::Direction;
use craft_theme::Variant;
use egui::epaint::text::{FontId, TextDirection};
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, Context, FontFamily, Painter, Stroke};
use std::borrow::Cow;
use std::cell::RefCell;

/// The weight of a regular run.
const REGULAR: u16 = 400;
/// The weight of a bold run.
const BOLD: u16 = 700;
/// The lightest weight that takes the semibold family.
const BOLD_FROM: u16 = 600;

/// The raw painter of an allowlisted content painter, clipped to its rect.
#[non_exhaustive]
pub struct ContentPainterCx<'a> {
    /// The painter.
    pub painter: Painter,
    pub(super) tokens: &'a UiTokens,
    pub(super) direction: Direction,
    env: &'a Env,
    ink: Ink,
}

impl<'a> ContentPainterCx<'a> {
    /// A raw painter for a content painter, with the env of the scope it paints for and the surface ink the
    /// content-paint kit reads.
    #[doc(hidden)]
    pub fn new(painter: Painter, env: &'a Env, ink: Ink) -> Self {
        ContentPainterCx { painter, tokens: &env.tokens, direction: env.direction, env, ink }
    }

    /// The style context of the scope being painted, for a content painter that resolves the style of a component
    /// it draws the face of.
    #[doc(hidden)]
    pub fn style_cx(&self) -> StyleCx<'_> {
        let env = self.env;
        StyleCx::new(
            self.painter.ctx(),
            &env.tokens,
            &env.values,
            env.size,
            Variant::SECONDARY,
            env.direction,
            env.scale,
            env.os,
            env.active,
            env.key,
            env.line_ratio,
        )
    }

    /// Runs `paint` with a `Draw` over this painter, in the scope's tokens and direction.
    #[doc(hidden)]
    pub fn with_draw<R>(&self, paint: impl FnOnce(&Draw<'_>) -> R) -> R {
        let record = RefCell::new(PaintRecord::default());
        paint(&Draw::new(&self.painter, &record, DrawEnv { tokens: self.tokens, direction: self.direction, ink: self.ink }))
    }

    /// The colour of text and glyphs on the surface being painted: the resolved content-text colour `Draw` paints
    /// text with.
    pub fn ink(&self) -> Color32 {
        ink_color(self.tokens, self.ink)
    }

    /// The width in points of a named stroke width on this display, as `Draw::stroke` strokes it: a content painter
    /// that strokes with the raw painter takes its widths here, so a hairline stays one physical pixel.
    pub fn stroke_width(&self, width: StrokeWidth) -> f32 {
        width.points(self.tokens, self.painter.ctx().pixels_per_point())
    }

    /// Runs `paint` with a `ContentPainterCx` whose painting is `share` as opaque as it would be: the fade
    /// `Draw::faded` applies (fades multiply; a value outside 0 to 1 is clamped and a non-finite one leaves the
    /// opacity as it was).
    pub fn faded(&self, share: f32, paint: impl FnOnce(&ContentPainterCx<'_>)) {
        let faded = ContentPainterCx {
            painter: super::draw::faded_painter(&self.painter, share),
            tokens: self.tokens,
            direction: self.direction,
            env: self.env,
            ink: self.ink,
        };
        paint(&faded);
    }

    /// The one sanctioned text layout for content painters: document runs with their own font, size and colour,
    /// laid out by the call `TextRun::layout` uses.
    pub fn layout(&self, text: &DocText<'_>) -> TextLayout {
        text.layout(self.painter.ctx(), self.tokens, Direction::Ltr)
    }
}

/// Document text: runs with their own font, size and colour. On `TextRun` these fields would let any component
/// bypass tokens for UI text, so only `ContentPainterCx::layout` takes them.
#[derive(Clone, Copy, Debug)]
pub struct DocText<'a> {
    /// The runs.
    pub runs: &'a [DocRun<'a>],
    /// The wrap width.
    pub wrap: Option<f32>,
    /// The horizontal alignment.
    pub align: HAlign,
    /// One row, cut with an ellipsis at the end where `wrap` does not fit it.
    pub elide: bool,
}

impl<'a> DocText<'a> {
    /// Unwrapped document text of `runs`.
    pub const fn new(runs: &'a [DocRun<'a>]) -> Self {
        DocText { runs, wrap: None, align: HAlign::Start, elide: false }
    }

    /// Cuts the text to one row with an ellipsis at the end.
    #[must_use]
    pub const fn elide(mut self) -> Self {
        self.elide = true;
        self
    }

    /// The text of every run, joined.
    pub fn full_text(&self) -> String {
        self.runs.iter().map(|run| run.text.as_ref()).collect()
    }

    /// Lays the runs out with the context's fonts: the call behind `ContentPainterCx::layout`, `Draw::doc` and `Text`
    /// with spans, so measure and paint agree. Each run's underline and strikethrough are drawn in the run's colour at
    /// `stroke.thin`; a bold run takes the semibold family when its font is the interface face and the app installed
    /// one.
    #[doc(hidden)]
    pub fn layout(&self, ctx: &Context, tokens: &UiTokens, direction: Direction) -> TextLayout {
        let mut job = LayoutJob::default();
        job.wrap.max_width = self.wrap.unwrap_or(f32::INFINITY);
        if self.elide {
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
        }
        if direction == Direction::Rtl {
            job.direction = TextDirection::RightToLeft;
        }
        let semibold = FontFamily::Name(SEMIBOLD.into());
        let has_semibold = stack_facts(ctx).semibold;
        for run in self.runs {
            let family = match run.font.family() {
                FontFamily::Proportional if run.weight >= BOLD_FROM && has_semibold => semibold.clone(),
                family => family,
            };
            let line = |on: bool| if on { Stroke::new(tokens.stroke.thin, run.color.0) } else { Stroke::NONE };
            job.append(
                run.text.as_ref(),
                run.indent,
                TextFormat {
                    font_id: FontId::new(run.size, family),
                    color: run.color.0,
                    italics: run.italic,
                    underline: line(run.underline),
                    strikethrough: line(run.strikethrough),
                    ..TextFormat::default()
                },
            );
        }
        let galley = text_layout::layout(ctx, job);
        let elided = galley.elided;
        TextLayout::new(aligned_rows(galley, self.align, direction == Direction::Rtl, ctx.pixels_per_point()), elided, self.full_text())
    }
}

/// One run of document text.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct DocRun<'a> {
    /// The text.
    pub text: Cow<'a, str>,
    /// The font.
    pub font: FontRef,
    /// The size in points.
    pub size: f32,
    /// The weight. egui's fonts have one weight per family: from 600 up, a run in the interface face takes the
    /// semibold family when the app installed one, and a run in any other family keeps its face.
    pub weight: u16,
    /// Italic.
    pub italic: bool,
    /// The colour, from data.
    pub color: DataColor,
    /// Underlined.
    pub underline: bool,
    /// Struck through.
    pub strikethrough: bool,
    /// Space in points before the run. On the first run of a paragraph it is the first-line indent: a wrapped row
    /// starts at the edge.
    pub indent: f32,
}

impl<'a> DocRun<'a> {
    /// A regular-weight upright run without lines or indent.
    pub fn new(text: impl Into<Cow<'a, str>>, font: FontRef, size: f32, color: DataColor) -> Self {
        DocRun { text: text.into(), font, size, weight: REGULAR, italic: false, color, underline: false, strikethrough: false, indent: 0.0 }
    }

    /// Italic.
    #[must_use]
    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    /// Bold weight.
    #[must_use]
    pub fn bold(mut self) -> Self {
        self.weight = BOLD;
        self
    }

    /// Underlined.
    #[must_use]
    pub fn underline(mut self) -> Self {
        self.underline = true;
        self
    }

    /// Struck through.
    #[must_use]
    pub fn strikethrough(mut self) -> Self {
        self.strikethrough = true;
        self
    }

    /// `points` of space before the run.
    #[must_use]
    pub fn indent(mut self, points: f32) -> Self {
        self.indent = points;
        self
    }

    /// The run owning its text.
    pub fn into_owned(self) -> DocRun<'static> {
        DocRun {
            text: Cow::Owned(self.text.into_owned()),
            font: self.font,
            size: self.size,
            weight: self.weight,
            italic: self.italic,
            color: self.color,
            underline: self.underline,
            strikethrough: self.strikethrough,
            indent: self.indent,
        }
    }
}
