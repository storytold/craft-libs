//! `TextRun`, the one description of UI text for measure, layout and paint, and `TextLayout`, its laid-out form.

use crate::fonts::StackFacts;
use crate::runtime::egui::text_layout;
use crate::style::text_measure::{paragraph_job, span_format};
use crate::style::{StyleColor, TypeRole, UiTokens};
use craft_i18n::Direction;
use egui::text::{LayoutJob, TextWrapping};
use egui::{Color32, Context, Galley};
use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;

/// How a run fits its room.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Wrap {
    /// One line at its natural width.
    #[default]
    Line,
    /// Wraps at the room's width.
    Wrap,
    /// One line, cut with an ellipsis at the end: names, titles, labels beside fixed controls.
    Elide,
    /// One line, cut with an ellipsis at the start: anything whose end says the most.
    ElideStart,
    /// One line, cut with an ellipsis in the middle, the end keeping a quarter of what stays: file names, where the
    /// extension and version suffix say as much as the start.
    ElideMiddle,
    /// One line, a folder or file path that drops leading folders first (`…/Pictures/Library`, `…\Pictures`), so the
    /// name at its end stays; a last name too long by itself is cut at its end.
    ElidePath,
    /// Wraps at the room's width and is cut with an ellipsis at the end of the given number of lines (at least 1).
    Clamp(u8),
}

impl Wrap {
    /// Whether the run is one line cut to its room.
    pub const fn cuts(self) -> bool {
        matches!(self, Wrap::Elide | Wrap::ElideStart | Wrap::ElideMiddle | Wrap::ElidePath)
    }
}

/// Where the lines of a multi-line run sit within the run's width.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HAlign {
    /// At the start edge: left, right in a right-to-left interface.
    #[default]
    Start,
    /// Centred.
    Center,
    /// At the end edge: right, left in a right-to-left interface.
    End,
}

/// One styled stretch of a run.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Span<'a> {
    /// The text.
    pub text: Cow<'a, str>,
    /// Italic.
    pub italic: bool,
    /// Tabular figures.
    pub tabular: bool,
    /// The monospace face.
    pub mono: bool,
    /// The muted text colour.
    pub muted: bool,
    /// A shortcut hint: muted.
    pub shortcut: bool,
    /// A search match highlight.
    pub highlight: bool,
    /// Underlined in the span's colour, `stroke.thin` wide, along the bottom of the line box.
    pub underline: bool,
    /// The role's semibold face, at the role's size.
    pub strong: bool,
}

impl<'a> Span<'a> {
    /// A plain span.
    pub fn new(text: impl Into<Cow<'a, str>>) -> Self {
        Span {
            text: text.into(),
            italic: false,
            tabular: false,
            mono: false,
            muted: false,
            shortcut: false,
            highlight: false,
            underline: false,
            strong: false,
        }
    }

    /// Italic.
    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    /// Tabular figures.
    pub fn tabular(mut self) -> Self {
        self.tabular = true;
        self
    }

    /// The monospace face.
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    /// The muted colour.
    pub fn muted(mut self) -> Self {
        self.muted = true;
        self
    }

    /// A shortcut hint.
    pub fn shortcut(mut self) -> Self {
        self.shortcut = true;
        self
    }

    /// A match highlight.
    pub fn highlight(mut self) -> Self {
        self.highlight = true;
        self
    }

    /// Underlined: a link, or the mnemonic letter of a menu title.
    pub fn underline(mut self) -> Self {
        self.underline = true;
        self
    }

    /// The role's semibold face: a short mark that stands out among the row's other text, such as a layer's `fx`.
    pub fn strong(mut self) -> Self {
        self.strong = true;
        self
    }
}

/// The one description of UI text.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TextRun<'a> {
    /// The styled stretches, in reading order.
    pub spans: Vec<Span<'a>>,
    /// The type role.
    pub role: TypeRole,
    /// The colour; `None` takes the surface's ink.
    pub color: Option<StyleColor>,
    /// How the run fits its room.
    pub wrap: Wrap,
    /// Laid out left to right whatever the env's direction (a chord).
    pub ltr: bool,
    /// Where the lines sit within the run's width.
    pub align: HAlign,
    /// The angle the run is painted at, in radians clockwise about the top-left corner of its layout. Layout and
    /// measure see the unrotated run.
    pub rotation: f32,
}

impl<'a> TextRun<'a> {
    /// The characters a run that elides keeps before its ellipsis at its least, its readable least: a row narrows
    /// such a run no further, and scrolls or drops it instead. A run of no more than one character beyond them (5 or
    /// fewer) never elides. White space before the ellipsis is cut with it.
    pub const READABLE_CHARACTERS: usize = 4;

    /// A one-span run in the label role.
    pub fn new(text: impl Into<Cow<'a, str>>) -> Self {
        TextRun {
            spans: vec![Span::new(text)],
            role: TypeRole::Label,
            color: None,
            wrap: Wrap::Line,
            ltr: false,
            align: HAlign::Start,
            rotation: 0.0,
        }
    }

    /// A run of `spans`.
    pub fn spans(spans: Vec<Span<'a>>) -> Self {
        TextRun { spans, role: TypeRole::Label, color: None, wrap: Wrap::Line, ltr: false, align: HAlign::Start, rotation: 0.0 }
    }

    /// `text` with its character `at` underlined, as a mnemonic shows while it is in use; all of `text` plain when
    /// `at` is `None` or past its end.
    pub fn mnemonic(text: &'a str, at: Option<usize>) -> Self {
        let Some((start, letter)) = at.and_then(|at| text.char_indices().nth(at)) else { return TextRun::new(text) };
        let end = start + letter.len_utf8();
        let spans = [Span::new(&text[..start]), Span::new(&text[start..end]).underline(), Span::new(&text[end..])];
        TextRun::spans(spans.into_iter().filter(|span| !span.text.is_empty()).collect())
    }

    /// Sets the type role.
    pub fn role(mut self, role: TypeRole) -> Self {
        self.role = role;
        self
    }

    /// Sets the colour.
    pub fn color(mut self, color: StyleColor) -> Self {
        self.color = Some(color);
        self
    }

    /// Sets how the run fits its room.
    pub fn wrap(mut self, wrap: Wrap) -> Self {
        self.wrap = wrap;
        self
    }

    /// Sets where the lines sit within the run's width, and where `Draw::text` places the run in its rect.
    pub fn align(mut self, align: HAlign) -> Self {
        self.align = align;
        self
    }

    /// Sets the angle the run is painted at, in radians clockwise about the top-left corner of its layout.
    pub fn rotated(mut self, radians: f32) -> Self {
        self.rotation = radians;
        self
    }

    /// Cuts the run with an ellipsis at the end.
    pub fn elide(self) -> Self {
        self.wrap(Wrap::Elide)
    }

    /// The text of every span, joined.
    pub fn full_text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_ref()).collect()
    }
}

/// The galley moved so its rect starts at x = 0: egui anchors a centred or right-aligned galley at its centre or
/// right edge, while every caller places a layout by its top-left corner.
pub(super) fn anchored_at_start(mut galley: Arc<Galley>) -> Arc<Galley> {
    let shift = -galley.rect.min.x;
    if shift == 0.0 {
        return galley;
    }
    let galley_mut = Arc::make_mut(&mut galley);
    let by = egui::vec2(shift, 0.0);
    for placed in &mut galley_mut.rows {
        placed.pos.x += shift;
    }
    galley_mut.rect = galley_mut.rect.translate(by);
    galley_mut.mesh_bounds = galley_mut.mesh_bounds.translate(by);
    galley
}

/// The galley with each row placed within the widest one by `align`: at the left edge or the right, the centre, with
/// Start and End swapping sides in a right-to-left paragraph. One rule for both directions, so a run measures the same
/// by direction. The rect of the galley already spans its widest row, which stays where it is.
pub(super) fn aligned_rows(mut galley: Arc<Galley>, align: HAlign, rtl: bool, pixels_per_point: f32) -> Arc<Galley> {
    let share = free_room_before(align, rtl);
    let width = galley.rect.width();
    if share == 0.0 || galley.rows.iter().all(|placed| placed.size.x >= width) {
        return galley;
    }
    let galley_mut = Arc::make_mut(&mut galley);
    for placed in &mut galley_mut.rows {
        let free = (width - placed.size.x).max(0.0);
        placed.pos.x = (free * share * pixels_per_point).round() / pixels_per_point;
    }
    galley
}

/// The share of the free room that sits before text aligned by `align`: 0 at the left edge, 1 at the right. The one
/// rule for rows within a run and for a laid-out block within its rect.
pub(super) fn free_room_before(align: HAlign, rtl: bool) -> f32 {
    match (align, rtl) {
        (HAlign::Center, _) => 0.5,
        (HAlign::Start, true) | (HAlign::End, false) => 1.0,
        _ => 0.0,
    }
}

/// What a layout reads from the env: the tokens, the direction and the ink colour.
#[derive(Clone, Copy)]
pub(crate) struct TextEnv<'a> {
    pub tokens: &'a UiTokens,
    pub direction: Direction,
    pub ink: Color32,
    /// The font stack of the context, read once for the layout.
    pub fonts: StackFacts,
}

impl TextRun<'_> {
    /// The one text layout behind `MeasureCx::layout`, `PartsCx::layout`, `TextParts::layout` and `Draw::text`: the
    /// paragraph direction comes from the caller's env, so measure and paint agree. `max_width` is the room; a
    /// `Line` run ignores it.
    pub(crate) fn layout(&self, ctx: &Context, env: &TextEnv<'_>, max_width: f32) -> TextLayout {
        let full = self.full_text();
        // Every cut goes through the one cut that `least_width` measures, so a run shown at its least keeps its
        // readable characters.
        let galley = match self.wrap {
            Wrap::Elide | Wrap::ElideStart | Wrap::ElideMiddle | Wrap::ElidePath => self.layout_cut(ctx, env, &full, max_width),
            _ => text_layout::layout(ctx, self.job(env, self.spans.iter().map(|s| (s, s.text.as_ref())), max_width)),
        };
        let elided = galley.elided || self.wrap.cuts() && galley.job.text != full;
        let mut layout = TextLayout::new(aligned_rows(galley, self.align, self.is_rtl(env), ctx.pixels_per_point()), elided, full);
        layout.rotation = self.rotation;
        layout
    }

    /// The width of the run at its readable least: cut where its wrap says, keeping
    /// [`READABLE_CHARACTERS`](Self::READABLE_CHARACTERS) and the ellipsis, or whole when it has no more than one
    /// character beyond them.
    pub(crate) fn least_width(&self, ctx: &Context, env: &TextEnv<'_>) -> f32 {
        let full = self.full_text();
        let whole = text_layout::layout(ctx, self.job(env, self.spans.iter().map(|s| (s, s.text.as_ref())), f32::INFINITY)).size().x;
        let ends: Vec<usize> = full.char_indices().map(|(at, _)| at).chain([full.len()]).collect();
        if ends.len() - 1 <= Self::READABLE_CHARACTERS + 1 {
            return whole;
        }
        let rtl_text = !self.is_rtl(env) && craft_i18n::base_direction(&full) == Some(Direction::Rtl);
        let width = |pieces: &[Piece]| text_layout::layout(ctx, self.job_cut(env, &full, pieces, rtl_text)).size().x;
        // The least is measured on the cut `layout_cut` shows at that width: for a path, its last name cut, or the
        // name whole after the ellipsis when the name is too short to cut.
        let least = if self.wrap == Wrap::ElidePath {
            let (name, separator) = path_name(&full);
            let name_ends: Vec<usize> = full[name.clone()].char_indices().map(|(at, _)| name.start + at).chain([name.end]).collect();
            match separator {
                _ if name_ends.len() - 1 > Self::READABLE_CHARACTERS + 1 => {
                    width(&cut_pieces(&full, &name_ends, name, Wrap::Elide, Self::READABLE_CHARACTERS))
                }
                Some(at) => width(&[Piece::Ellipsis, Piece::Keep(at..full.len())]),
                None => whole,
            }
        } else {
            width(&cut_pieces(&full, &ends, 0..full.len(), self.wrap, Self::READABLE_CHARACTERS))
        };
        least.min(whole)
    }

    /// Whether the paragraph runs right to left: the env's direction, unless the run is fixed left to right.
    fn is_rtl(&self, env: &TextEnv<'_>) -> bool {
        !self.ltr && env.direction == Direction::Rtl
    }

    fn job<'s>(&self, env: &TextEnv<'_>, parts: impl Iterator<Item = (&'s Span<'s>, &'s str)>, max_width: f32) -> LayoutJob {
        let elide = self.wrap.cuts();
        let wrap = TextWrapping {
            max_width: if self.wrap == Wrap::Line { f32::INFINITY } else { max_width },
            max_rows: match self.wrap {
                Wrap::Clamp(lines) => usize::from(lines.max(1)),
                _ if elide => 1,
                _ => usize::MAX,
            },
            break_anywhere: elide,
            ..TextWrapping::default()
        };
        let mut job = paragraph_job(env.direction, self.ltr, wrap);
        let base = self.color.map_or(env.ink, StyleColor::egui);
        let text = &env.tokens.color.text;
        let to_color = |c: craft_theme::Rgba| Color32::from_rgba_unmultiplied(c.0[0], c.0[1], c.0[2], c.0[3]);
        for (span, text_part) in parts {
            let color = if span.muted {
                to_color(text.muted)
            } else if span.shortcut {
                to_color(text.faint)
            } else {
                base
            };
            let background = if span.highlight { to_color(env.tokens.color.state.r#match) } else { Color32::TRANSPARENT };
            let mut format = span_format(env.tokens, env.fonts, self.role, span, color, background);
            if span.underline {
                format.underline = egui::Stroke::new(env.tokens.stroke.thin, color);
            }
            job.append(text_part, 0.0, format);
        }
        job
    }

    /// The run on one row, cut where its wrap says (the end for `Elide`) with one ellipsis, keeping the most characters
    /// that fit `max_width`; whole when it fits.
    fn layout_cut(&self, ctx: &Context, env: &TextEnv<'_>, full: &str, max_width: f32) -> Arc<Galley> {
        let whole = text_layout::layout(ctx, self.job(env, self.spans.iter().map(|s| (s, s.text.as_ref())), f32::INFINITY));
        if whole.size().x <= max_width {
            return whole;
        }
        // Right-to-left text in a left-to-right paragraph keeps its ellipsis at its own end, the left.
        let rtl_text = !self.is_rtl(env) && craft_i18n::base_direction(full) == Some(Direction::Rtl);
        let layout = |pieces: &[Piece]| text_layout::layout(ctx, self.job_cut(env, full, pieces, rtl_text));
        if self.wrap == Wrap::ElidePath {
            // Leading folders go first, up to the separator before the last name, then the name itself is cut.
            let (name, separator) = path_name(full);
            for (at, _) in full.match_indices(['/', '\\']).filter(|(at, _)| *at > 0 && separator.is_some_and(|last| *at <= last)) {
                let galley = layout(&[Piece::Ellipsis, Piece::Keep(at..full.len())]);
                if galley.size().x <= max_width {
                    return galley;
                }
            }
            return Self::fit(&layout, full, name, Wrap::Elide, max_width);
        }
        Self::fit(&layout, full, 0..full.len(), self.wrap, max_width)
    }

    /// The cut of `kept` (bytes of `full`) at `wrap`'s place that keeps the most characters within `max_width`, found
    /// by bisection over the character count; at least the ellipsis alone.
    fn fit(layout: &dyn Fn(&[Piece]) -> Arc<Galley>, full: &str, kept: Range<usize>, wrap: Wrap, max_width: f32) -> Arc<Galley> {
        let text = full.get(kept.clone()).unwrap_or_default();
        let ends: Vec<usize> = text.char_indices().map(|(at, _)| kept.start + at).chain([kept.end]).collect();
        let total = ends.len() - 1;
        let cut = |count: usize| cut_pieces(full, &ends, kept.clone(), wrap, count);
        let (mut lo, mut hi) = (0, total.saturating_sub(1));
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if layout(&cut(mid)).size().x <= max_width { lo = mid } else { hi = mid - 1 }
        }
        layout(&cut(lo))
    }

    /// The job of `pieces` of `full`: each kept range in the style of the spans it covers, each ellipsis in the style
    /// of the span the text resumes in (the last span at the end). With `rtl_text` each ellipsis is followed by a
    /// right-to-left mark, so it takes the text's direction instead of the paragraph's.
    fn job_cut(&self, env: &TextEnv<'_>, full: &str, pieces: &[Piece], rtl_text: bool) -> LayoutJob {
        let mut starts = Vec::with_capacity(self.spans.len());
        let mut at = 0;
        for span in &self.spans {
            starts.push(at..at + span.text.len());
            at += span.text.len();
        }
        let span_at = |byte: usize| starts.iter().position(|range| range.contains(&byte)).unwrap_or(starts.len().saturating_sub(1));
        let mut parts: Vec<(&Span<'_>, Cow<'_, str>)> = Vec::new();
        for (index, piece) in pieces.iter().enumerate() {
            match piece {
                Piece::Keep(range) => {
                    for (span, bytes) in self.spans.iter().zip(&starts) {
                        let (from, to) = (range.start.max(bytes.start), range.end.min(bytes.end));
                        if from < to {
                            parts.push((span, Cow::Borrowed(full.get(from..to).unwrap_or_default())));
                        }
                    }
                }
                Piece::Ellipsis => {
                    let next = pieces.get(index + 1).and_then(|piece| match piece {
                        Piece::Keep(range) => Some(range.start),
                        Piece::Ellipsis => None,
                    });
                    if let Some(span) = self.spans.get(span_at(next.unwrap_or(full.len().saturating_sub(1)))) {
                        parts.push((span, Cow::Borrowed(if rtl_text { RTL_ELLIPSIS } else { ELLIPSIS })));
                    }
                }
            }
        }
        self.job(env, parts.iter().map(|(span, text)| (*span, text.as_ref())), f32::INFINITY)
    }
}

/// The ellipsis every cut shows.
const ELLIPSIS: &str = "\u{2026}";
/// The ellipsis followed by a right-to-left mark.
const RTL_ELLIPSIS: &str = "\u{2026}\u{200f}";

/// A piece of a cut run: a byte range of its text kept, or the ellipsis.
enum Piece {
    Keep(Range<usize>),
    Ellipsis,
}

/// The last name of the path `full` (bytes, trailing separators excluded, since a trailing separator starts no name)
/// and the separator before it, if any: where `Wrap::ElidePath` cuts last.
fn path_name(full: &str) -> (Range<usize>, Option<usize>) {
    let end = full.trim_end_matches(['/', '\\']).len();
    let separator = full.get(..end).and_then(|text| text.rfind(['/', '\\']));
    (separator.map_or(0, |at| at + 1)..end, separator)
}

/// The pieces of `kept` (bytes of `full`; `ends` holds where each of its characters starts, then its end) cut at
/// `wrap`'s place to keep `count` characters and one ellipsis, less the white space before the ellipsis.
fn cut_pieces(full: &str, ends: &[usize], kept: Range<usize>, wrap: Wrap, count: usize) -> Vec<Piece> {
    let total = ends.len() - 1;
    let (head, tail) = match wrap {
        Wrap::ElideStart => (0, count),
        Wrap::ElideMiddle => (count - count / 4, count / 4),
        _ => (count, 0),
    };
    let head_end = kept.start + full.get(kept.start..ends[head]).unwrap_or_default().trim_end().len();
    let mut pieces = vec![Piece::Keep(kept.start..head_end), Piece::Ellipsis, Piece::Keep(ends[total - tail]..kept.end)];
    pieces.retain(|piece| !matches!(piece, Piece::Keep(range) if range.is_empty()));
    pieces
}

/// One direction run of a single-row layout, owned.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct LayoutRun {
    /// The run's bytes in the text.
    pub bytes: Range<usize>,
    /// Whether the run reads right to left.
    pub rtl: bool,
    /// Each character's start offset in the row.
    pub positions: Vec<f32>,
    /// Each character's advance.
    pub widths: Vec<f32>,
}

/// Records that the galley of `layout` was painted, and its full text when the layout cut text.
pub(crate) fn note_painted(ctx: &Context, layout: &TextLayout) {
    if layout.elided {
        crate::runtime::egui::painted::note_elided(ctx, &layout.galley, &layout.full);
    }
}

/// A laid-out run.
#[derive(Clone)]
#[non_exhaustive]
pub struct TextLayout {
    /// The galley.
    pub galley: Arc<Galley>,
    /// The layout cut text.
    pub elided: bool,
    /// The full text, as the tooltip and accessible name of an elided run.
    pub full: String,
    /// The first row's baseline, from the layout's top.
    pub first_baseline: f32,
    /// The bidi runs of a single-row layout; empty for a wrapped one.
    pub runs: Vec<LayoutRun>,
    /// The run's paint angle, in radians clockwise about the layout's top-left corner.
    pub rotation: f32,
}

impl TextLayout {
    pub(crate) fn new(galley: Arc<Galley>, elided: bool, full: String) -> TextLayout {
        let first_baseline = galley.rows.first().map_or(0.0, |placed| match placed.row.glyphs.first() {
            Some(glyph) => placed.pos.y + glyph.pos.y,
            None => placed.pos.y + placed.row.size.y,
        });
        let runs = if galley.rows.len() <= 1 { Self::build_row_runs(&galley, 0) } else { Vec::new() };
        TextLayout { galley, elided, full, first_baseline, runs, rotation: 0.0 }
    }

    /// One run per maximal stretch of glyphs with one direction; an empty row is one empty left-to-right run.
    fn build_row_runs(galley: &Galley, row: usize) -> Vec<LayoutRun> {
        let glyphs = galley.rows.get(row).map_or(&[][..], |placed| &placed.row.glyphs[..]);
        let mut runs: Vec<LayoutRun> = Vec::new();
        let mut at = 0;
        for glyph in glyphs {
            let len = glyph.chr.len_utf8();
            match runs.last_mut() {
                Some(run) if run.rtl == glyph.is_rtl() => {
                    run.bytes.end += len;
                    run.positions.push(glyph.pos.x);
                    run.widths.push(glyph.advance_width);
                }
                _ => {
                    runs.push(LayoutRun { bytes: at..at + len, rtl: glyph.is_rtl(), positions: vec![glyph.pos.x], widths: vec![glyph.advance_width] })
                }
            }
            at += len;
        }
        if runs.is_empty() {
            runs.push(LayoutRun { bytes: 0..0, rtl: false, positions: Vec::new(), widths: Vec::new() });
        }
        runs
    }

    /// Every row as its text, its rect from the galley's origin and its bidi runs, for the AccessKit run nodes.
    pub(crate) fn row_lines(&self) -> Vec<(String, egui::Rect, Vec<LayoutRun>)> {
        (0..self.galley.rows.len())
            .map(|at| {
                let placed = &self.galley.rows[at];
                let text = placed.row.glyphs.iter().map(|glyph| glyph.chr).collect();
                (text, egui::Rect::from_min_size(placed.pos, placed.row.size), Self::build_row_runs(&self.galley, at))
            })
            .collect()
    }

    /// The galley size.
    pub fn size(&self) -> egui::Vec2 {
        self.galley.size()
    }
}

/// A chord formatted for the OS, as one text run laid out as its own left-to-right run.
#[derive(Clone, Debug, PartialEq)]
pub struct ChordText {
    text: String,
}

impl ChordText {
    /// A chord already formatted by craft-keymap's pure function over `craft_keymap::layout`.
    pub fn new(formatted: impl Into<String>) -> Self {
        ChordText { text: formatted.into() }
    }

    /// The chord as a run laid out left to right in any direction.
    pub fn run(&self) -> TextRun<'_> {
        let mut run = TextRun::new(self.text.as_str());
        run.ltr = true;
        run
    }

    /// The chord isolated as a left-to-right run (LRI ... PDI) for use inside a longer label.
    pub fn isolated(&self) -> String {
        format!("\u{2066}{}\u{2069}", self.text)
    }
}

/// The width of a reading column, from the text it holds: a count of characters of the role's average character
/// width, as a print measure is counted, so the column follows the text size.
///
/// - **Counts.** [`TARGET_CHARACTERS`](Self::TARGET_CHARACTERS) is the measure a reading column aims for and
///   [`LEAST_CHARACTERS`](Self::LEAST_CHARACTERS) the shortest it allows. Both are Bringhurst's (*The Elements of
///   Typographic Style*, 2.1.2): 66 characters is the ideal for a single-column page, and 45 to 75 is the range
///   that reads well.
/// - **Average character width.** The width of the lowercase Latin alphabet, a to z, set on one line in the role's
///   face and size, divided by 26. That is the typographer's usual way to count characters per line. It is measured
///   from the font that draws the text (the same layout `TextRun::layout` does), not a constant.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ReadingMeasure {
    /// The width a reading column aims for, in points.
    pub target: f32,
    /// The least width a reading column keeps, in points.
    pub least: f32,
}

impl ReadingMeasure {
    /// The characters in the measure a reading column aims for.
    pub const TARGET_CHARACTERS: f32 = 66.0;
    /// The characters in the shortest measure a reading column allows.
    pub const LEAST_CHARACTERS: f32 = 45.0;

    const SAMPLE: &'static str = "abcdefghijklmnopqrstuvwxyz";
    const SAMPLE_CHARACTERS: f32 = 26.0;

    /// The widths for text of `role`. `layout` lays a run out the way the component lays out its text
    /// (`|run| cx.layout(run)` in a measure, `|run| draw.layout(run, f32::INFINITY)` in a painter).
    pub fn of(role: TypeRole, layout: impl FnOnce(&TextRun<'static>) -> TextLayout) -> Self {
        let sample = TextRun::new(Self::SAMPLE).role(role);
        let average = layout(&sample).size().x / Self::SAMPLE_CHARACTERS;
        ReadingMeasure { target: average * Self::TARGET_CHARACTERS, least: average * Self::LEAST_CHARACTERS }
    }
}
