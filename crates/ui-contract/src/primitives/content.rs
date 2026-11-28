//! Content painters and document text.

use super::draw::{Draw, DrawEnv, StrokeWidth};
use super::record::PaintRecord;
use super::text::{HAlign, TextLayout, aligned_rows};
use crate::fonts::{FontRef, SEMIBOLD, stack_facts};
use crate::runtime::egui::text_layout;
use crate::runtime::env::Env;
use crate::style::{DataColor, Ink, StyleCx, UiTokens, ink_color};
use craft_i18n::{Direction, Format, WordSet};
use egui::epaint::text::{FontId, TextDirection, VariationCoords};
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, Context, FontFamily, Painter, Stroke};
use std::borrow::Cow;
use std::cell::RefCell;
use std::sync::Arc;

/// The weight of a regular run.
const REGULAR: u16 = 400;
/// The weight of a bold run.
const BOLD: u16 = 700;
/// The lightest weight that takes the semibold family.
const BOLD_FROM: u16 = 600;

/// The raw painter of an allowlisted content painter, clipped to its rect.
#[non_exhaustive]
pub struct ContentPainterCx<'a> {
    /// The painter, for theme colours. Colours from data go through [`data_shapes`](Self::data_shapes), the kit's data
    /// methods (`picture`, `checker`, `ramp`) or `with_draw`, which dim them while the component paints its Disabled
    /// state; a component that paints data with this painter takes `DisabledLook::Faded`.
    pub painter: Painter,
    pub(super) tokens: &'a UiTokens,
    pub(super) direction: Direction,
    env: &'a Env,
    ink: Ink,
    /// How the data paths and `with_draw` paint data colours, as the component's `PaintUi::draw` does.
    pub(super) data: super::DataMode,
}

impl<'a> ContentPainterCx<'a> {
    /// A raw painter for a content painter, with the env of the scope it paints for and the surface ink the
    /// content-paint kit reads.
    #[doc(hidden)]
    pub fn new(painter: Painter, env: &'a Env, ink: Ink) -> Self {
        ContentPainterCx { painter, tokens: &env.tokens, direction: env.direction, env, ink, data: super::DataMode::Full }
    }

    /// The same painter whose `with_draw` paints data colours in `data`.
    pub(crate) fn data(mut self, data: super::DataMode) -> Self {
        self.data = data;
        self
    }

    /// The style context of the scope being painted, for a content painter that resolves the style of a component
    /// it draws the face of.
    #[doc(hidden)]
    pub fn style_cx(&self) -> StyleCx<'_> {
        StyleCx::of_env(self.painter.ctx(), self.env)
    }

    /// Runs `paint` with a `Draw` over this painter, in the scope's tokens and direction.
    #[doc(hidden)]
    pub fn with_draw<R>(&self, paint: impl FnOnce(&Draw<'_>) -> R) -> R {
        let record = RefCell::new(PaintRecord::default());
        paint(&Draw::new(&self.painter, &record, DrawEnv { tokens: self.tokens, direction: self.direction, ink: self.ink }).data(self.data))
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
            data: self.data,
        };
        paint(&faded);
    }

    /// Runs `paint` with a `ContentPainterCx` whose painting is clipped to `rect` (inside the clip it already has).
    pub fn clipped(&self, rect: egui::Rect, paint: impl FnOnce(&ContentPainterCx<'_>)) {
        let clipped = ContentPainterCx {
            painter: self.painter.with_clip_rect(rect.intersect(self.painter.clip_rect())),
            tokens: self.tokens,
            direction: self.direction,
            env: self.env,
            ink: self.ink,
            data: self.data,
        };
        paint(&clipped);
    }

    /// The one sanctioned text layout for content painters: document runs with their own font, size and colour,
    /// laid out by the call `TextRun::layout` uses.
    pub fn layout(&self, text: &DocText<'_>) -> TextLayout {
        text.layout(self.painter.ctx(), self.tokens, Direction::Ltr)
    }
}

impl ContentPainterCx<'_> {
    /// The word set `L` in the language of the scope the painter paints for.
    pub fn words<L: WordSet + Send + Sync + 'static>(&self) -> Arc<L> {
        self.env.words()
    }

    /// How the scope the painter paints for writes numbers.
    pub fn format(&self) -> &Format {
        self.env.format()
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
                    coords: VariationCoords::new(run.coords.iter().copied()),
                    ..TextFormat::default()
                },
            );
        }
        let galley = text_layout::layout(ctx, job);
        let elided = galley.elided;
        TextLayout::from_job(aligned_rows(galley, self.align, direction == Direction::Rtl, ctx.pixels_per_point()), elided, None)
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
    /// The variation coordinates the run is drawn at (axis tag, value), over those of its face
    /// ([`FaceBytes::coords`](crate::fonts::FaceBytes::coords)): one variable face registered once draws every style a
    /// document or an axis slider asks for. Empty draws the face as it is registered.
    pub coords: Cow<'a, [([u8; 4], f32)]>,
}

/// A run of text in one face of a document's fonts, which [`Draw::face_run`](super::Draw::face_run) draws on a
/// baseline: an input method's composition in the face and size of the text it goes into, as Word draws it.
///
/// - **Face.** `key` names the face among the context's document faces, one key per font file, face index and
///   variation setting (a font database's face id). The caller gives the face's bytes and index, which it holds,
///   through the `load` of `face_run`; it runs only the first time the key is drawn, so a face is installed once, not
///   every frame, and the run is drawn in its face in that same pass.
/// - **Fallback.** Characters the face lacks are drawn by the interface fonts, in their order, CJK included. A face
///   that does not load is drawn in the interface face.
/// - **Size.** `size` points on screen (the document size times the view's zoom), at least 1 and at most
///   [`largest_size`](Self::largest_size): egui draws each glyph into its font atlas and fails on one wider than the
///   atlas, so a larger run is drawn smaller, at the cap.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct FaceRun<'a> {
    /// The text.
    pub text: &'a str,
    /// The face's key among the context's document faces.
    pub key: &'a str,
    /// The size in points on screen.
    pub size: f32,
    /// The colour, from data.
    pub color: DataColor,
}

impl<'a> FaceRun<'a> {
    /// `text` in the face `key` at `size` points in `color`.
    pub const fn new(text: &'a str, key: &'a str, size: f32, color: DataColor) -> Self {
        FaceRun { text, key, size, color }
    }

    /// The largest size, in points, a face run is drawn at on `ctx`'s display: a glyph stays within a quarter of the
    /// font atlas's side and at most 1,024 pixels.
    pub fn largest_size(ctx: &Context) -> f32 {
        let side = ctx.input(|input| input.max_texture_side) as f32;
        (side / 4.0).min(1024.0) / ctx.pixels_per_point().max(f32::EPSILON)
    }

    /// The size the run is drawn at on `ctx`'s display: its own, within 1 point and [`largest_size`](Self::largest_size).
    pub fn drawn_size(&self, ctx: &Context) -> f32 {
        // `max` takes 1 for a NaN size; `min` takes the cap for an infinite one.
        self.size.max(1.0).min(Self::largest_size(ctx))
    }
}

impl<'a> DocRun<'a> {
    /// A regular-weight upright run without lines or indent.
    pub fn new(text: impl Into<Cow<'a, str>>, font: FontRef, size: f32, color: DataColor) -> Self {
        DocRun {
            text: text.into(),
            font,
            size,
            weight: REGULAR,
            italic: false,
            color,
            underline: false,
            strikethrough: false,
            indent: 0.0,
            coords: Cow::Borrowed(&[]),
        }
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

    /// The run drawn at the variation coordinates `coords` (axis tag, value) of its face.
    #[must_use]
    pub fn coords(mut self, coords: impl Into<Cow<'a, [([u8; 4], f32)]>>) -> Self {
        self.coords = coords.into();
        self
    }

    /// The run owning its text and coordinates.
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
            coords: Cow::Owned(self.coords.into_owned()),
        }
    }
}
