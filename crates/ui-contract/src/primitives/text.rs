//! `TextRun`, the one description of UI text for measure, layout and paint, and `TextLayout`, its laid-out form.

use crate::fonts::StackFacts;
use crate::runtime::egui::memory;
use crate::runtime::egui::text_layout;
pub use crate::style::Span;
use crate::style::text_measure::{paragraph_job, span_format};
use crate::style::{StyleColor, TypeRole, UiTokens};
use craft_i18n::Direction;
use egui::text::{LayoutJob, TextWrapping};
use egui::{Color32, Context, Galley};
use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::ops::{Deref, DerefMut, Range};
use std::sync::{Arc, OnceLock};

/// How a run fits its room.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Wrap {
    /// One line at its natural width.
    #[default]
    Line,
    /// Wraps at the room's width.
    Wrap,
    /// One line, cut at the room's edge with no ellipsis, as many characters as fit: tabs that shrink to their first
    /// letters. The full text is the tooltip and accessible name, as for an elided run.
    Clip,
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

    /// Borrows the common single-span case; only a multi-span run needs joining.
    fn full_text_cow(&self) -> Cow<'_, str> {
        match self.spans.as_slice() {
            [span] => Cow::Borrowed(&span.text),
            _ => Cow::Owned(self.full_text()),
        }
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

/// The baseline of a control's line of `role` whose centre line lies at `centre`: where a one-line text of `role` in
/// its own face sits with its row centred on the centre line, as `Draw::text` centres a line in a taller rect. Every
/// text in the line sits on it, whatever its face or size: a select's face, a field's mono digits and the smaller unit
/// after them.
pub(crate) fn line_baseline(ctx: &Context, env: &TextEnv<'_>, role: TypeRole, centre: f32) -> f32 {
    let line = TextRun::new("x").role(role).layout(ctx, env, f32::INFINITY);
    centre - line.size().y / 2.0 + line.first_baseline
}

/// A bounded per-context cache. Equality compares the complete input, rather than trusting a hash collision.
#[derive(Clone, PartialEq)]
struct LayoutKey {
    run: TextRun<'static>,
    tokens: u64,
    direction: Direction,
    ink: Color32,
    fonts: StackFacts,
    width: u32,
    scale: u32,
}

/// A lookup borrows the spans; only a new cache entry copies their text.
struct LayoutQuery<'a> {
    run: &'a TextRun<'a>,
    tokens: u64,
    direction: Direction,
    ink: Color32,
    fonts: StackFacts,
    width: u32,
    scale: u32,
}

struct CachedText {
    key: LayoutKey,
    layout: Option<Arc<TextLayout>>,
    least: Option<f32>,
}

#[derive(Default)]
struct TextMemo {
    entries: HashMap<u64, Vec<CachedText>>,
    order: VecDeque<u64>,
    generation: u64,
    sentinel: Option<Arc<Galley>>,
}

fn with_memo<R>(ctx: &Context, f: impl FnOnce(&mut TextMemo) -> R) -> R {
    // Egui clears its galley cache whenever the atlas or any layout input owned by fonts changes.
    // Keeping this cached empty galley lets us observe that reset, including external providers and rasterizers.
    let sentinel = ctx.fonts_mut(|fonts| fonts.layout_job(LayoutJob::default()));
    memory::with::<TextMemo, _>(ctx, |memo| {
        if memo.sentinel.as_ref().is_none_or(|old| !Arc::ptr_eq(old, &sentinel)) {
            memo.entries.clear();
            memo.order.clear();
            memo.generation = memo.generation.wrapping_add(1);
            memo.sentinel = Some(sentinel);
        }
        f(memo)
    })
}

impl LayoutQuery<'_> {
    // Hash the text and the common discriminators. Equality checks the remaining fields and resolves collisions.
    fn hash(&self) -> u64 {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        for span in &self.run.spans {
            span.text.hash(&mut hash);
        }
        self.run.role.hash(&mut hash);
        self.tokens.hash(&mut hash);
        self.direction.hash(&mut hash);
        self.width.hash(&mut hash);
        self.scale.hash(&mut hash);
        hash.finish()
    }

    fn matches(&self, key: &LayoutKey) -> bool {
        key.run == *self.run
            && key.tokens == self.tokens
            && key.direction == self.direction
            && key.ink == self.ink
            && key.fonts == self.fonts
            && key.width == self.width
            && key.scale == self.scale
    }

    fn owned(&self) -> LayoutKey {
        let spans = self
            .run
            .spans
            .iter()
            .map(|span| Span {
                text: Cow::Owned(span.text.to_string()),
                italic: span.italic,
                tabular: span.tabular,
                mono: span.mono,
                muted: span.muted,
                shortcut: span.shortcut,
                highlight: span.highlight,
                underline: span.underline,
                strong: span.strong,
            })
            .collect();
        LayoutKey {
            run: TextRun {
                spans,
                role: self.run.role,
                color: self.run.color,
                wrap: self.run.wrap,
                ltr: self.run.ltr,
                align: self.run.align,
                rotation: self.run.rotation,
            },
            tokens: self.tokens,
            direction: self.direction,
            ink: self.ink,
            fonts: self.fonts,
            width: self.width,
            scale: self.scale,
        }
    }
}

impl TextMemo {
    fn get(&self, key: &LayoutQuery<'_>) -> Option<&CachedText> {
        self.entries.get(&key.hash())?.iter().find(|entry| key.matches(&entry.key))
    }

    fn put(&mut self, key: &LayoutQuery<'_>, layout: Option<Arc<TextLayout>>, least: Option<f32>) {
        // Large document text should not pin galleys in a UI-label cache.
        if key.run.spans.len() > 1024 || key.run.spans.iter().fold(0usize, |len, span| len.saturating_add(span.text.len())) > 16 * 1024 {
            return;
        }
        let hash = key.hash();
        if let Some(entry) = self.entries.get_mut(&hash).and_then(|bucket| bucket.iter_mut().find(|entry| key.matches(&entry.key))) {
            if layout.is_some() {
                entry.layout = layout;
            }
            if least.is_some() {
                entry.least = least;
            }
            return;
        }
        // Bound retained text and galleys, including colliding keys.
        if self.order.len() >= 4096
            && let Some(old) = self.order.pop_front()
            && let Some(bucket) = self.entries.get_mut(&old)
        {
            bucket.remove(0);
            if bucket.is_empty() {
                self.entries.remove(&old);
            }
        }
        self.entries.entry(hash).or_default().push(CachedText { key: key.owned(), layout, least });
        self.order.push_back(hash);
    }
}

impl TextRun<'_> {
    /// All inputs read by `job`, the cutting and row-alignment steps.
    fn layout_key<'a>(&'a self, ctx: &Context, env: &TextEnv<'_>, max_width: f32) -> LayoutQuery<'a> {
        LayoutQuery {
            run: self,
            tokens: env.tokens.key(),
            direction: env.direction,
            ink: env.ink,
            fonts: env.fonts,
            width: max_width.to_bits(),
            scale: ctx.pixels_per_point().to_bits(),
        }
    }

    /// The one text layout behind `MeasureCx::layout`, `PartsCx::layout`, `TextParts::layout` and `Draw::text`: the
    /// paragraph direction comes from the caller's env, so measure and paint agree. `max_width` is the room; a
    /// `Line` run ignores it.
    pub(crate) fn layout(&self, ctx: &Context, env: &TextEnv<'_>, max_width: f32) -> TextLayout {
        let key = self.layout_key(ctx, env, max_width);
        let (cached, generation) = with_memo(ctx, |memo| (memo.get(&key).and_then(|entry| entry.layout.clone()), memo.generation));
        if let Some(layout) = cached {
            return (*layout).clone();
        }
        // Only a custom cut needs the joined text: other jobs already own the full text.
        let full = self.wrap.cuts().then(|| self.full_text_cow());
        // Every cut goes through the one cut that `least_width` measures, so a run shown at its least keeps its
        // readable characters.
        let galley = match &full {
            Some(full) => self.layout_cut(ctx, env, full, max_width),
            None => text_layout::layout(ctx, self.job(env, self.spans.iter().map(|s| (s, s.text.as_ref())), max_width)),
        };
        let changed = full.as_ref().is_some_and(|full| galley.job.text != *full);
        let elided = galley.elided || changed;
        let original = full.filter(|_| changed).map(|full| Arc::<str>::from(full.as_ref()));
        let mut layout = TextLayout::from_job(aligned_rows(galley, self.align, self.is_rtl(env), ctx.pixels_per_point()), elided, original);
        layout.rotation = self.rotation;
        with_memo(ctx, |memo| {
            if memo.generation == generation {
                memo.put(&key, Some(Arc::new(layout.clone())), None);
            }
        });
        layout
    }

    /// The width of the run at its readable least: cut where its wrap says, keeping
    /// [`READABLE_CHARACTERS`](Self::READABLE_CHARACTERS) and the ellipsis, or whole when it has no more than one
    /// character beyond them.
    pub(crate) fn least_width(&self, ctx: &Context, env: &TextEnv<'_>) -> f32 {
        let key = self.layout_key(ctx, env, f32::INFINITY);
        let (cached, generation) = with_memo(ctx, |memo| (memo.get(&key).and_then(|entry| entry.least), memo.generation));
        if let Some(least) = cached {
            return least;
        }
        let least = self.compute_least_width(ctx, env);
        with_memo(ctx, |memo| {
            if memo.generation == generation {
                memo.put(&key, None, Some(least));
            }
        });
        least
    }

    fn compute_least_width(&self, ctx: &Context, env: &TextEnv<'_>) -> f32 {
        let full = self.full_text_cow();
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
        let clip = self.wrap == Wrap::Clip;
        let wrap = TextWrapping {
            max_width: if self.wrap == Wrap::Line { f32::INFINITY } else { max_width },
            max_rows: match self.wrap {
                Wrap::Clamp(lines) => usize::from(lines.max(1)),
                _ if elide || clip => 1,
                _ => usize::MAX,
            },
            break_anywhere: elide || clip,
            overflow_character: if clip { None } else { TextWrapping::default().overflow_character },
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
        crate::runtime::egui::painted::note_elided(ctx, &layout.galley, layout.full());
    }
}

/// The full text of a layout, materialized only when read.
///
/// Dereferences to `String` so existing field reads continue to work.
pub struct LayoutText {
    galley: Arc<Galley>,
    original: Option<Arc<str>>,
    value: Arc<OnceLock<String>>,
    changed: Option<String>,
}

impl LayoutText {
    fn shared(&self) -> Self {
        Self { galley: self.galley.clone(), original: self.original.clone(), value: self.value.clone(), changed: self.changed.clone() }
    }
}

impl std::fmt::Debug for LayoutText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.deref(), f)
    }
}

impl std::fmt::Display for LayoutText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self.deref(), f)
    }
}

impl AsRef<str> for LayoutText {
    fn as_ref(&self) -> &str {
        self.deref()
    }
}

impl PartialEq for LayoutText {
    fn eq(&self, other: &Self) -> bool {
        self.deref() == other.deref()
    }
}

impl PartialEq<&str> for LayoutText {
    fn eq(&self, other: &&str) -> bool {
        self.deref() == other
    }
}

impl PartialEq<String> for LayoutText {
    fn eq(&self, other: &String) -> bool {
        self.deref() == other
    }
}

impl Deref for LayoutText {
    type Target = String;

    fn deref(&self) -> &String {
        self.changed.as_ref().unwrap_or_else(|| self.value.get_or_init(|| self.original.as_deref().unwrap_or(&self.galley.job.text).to_owned()))
    }
}

impl DerefMut for LayoutText {
    fn deref_mut(&mut self) -> &mut String {
        let (value, original, galley) = (&self.value, &self.original, &self.galley);
        self.changed.get_or_insert_with(|| value.get_or_init(|| original.as_deref().unwrap_or(&galley.job.text).to_owned()).clone())
    }
}

/// A layout's single-row direction runs, built only when read.
///
/// Dereferences to `Vec<LayoutRun>` so existing field reads continue to work.
pub struct LayoutRuns {
    galley: Arc<Galley>,
    value: Arc<OnceLock<Vec<LayoutRun>>>,
    changed: Option<Vec<LayoutRun>>,
}

impl LayoutRuns {
    fn shared(&self) -> Self {
        Self { galley: self.galley.clone(), value: self.value.clone(), changed: self.changed.clone() }
    }
}

impl std::fmt::Debug for LayoutRuns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.deref(), f)
    }
}

impl PartialEq for LayoutRuns {
    fn eq(&self, other: &Self) -> bool {
        self.deref() == other.deref()
    }
}

impl PartialEq<Vec<LayoutRun>> for LayoutRuns {
    fn eq(&self, other: &Vec<LayoutRun>) -> bool {
        self.deref() == other
    }
}

impl Deref for LayoutRuns {
    type Target = Vec<LayoutRun>;

    fn deref(&self) -> &Vec<LayoutRun> {
        self.changed.as_ref().unwrap_or_else(|| {
            self.value.get_or_init(|| if self.galley.rows.len() <= 1 { TextLayout::build_row_runs(&self.galley, 0) } else { Vec::new() })
        })
    }
}

impl DerefMut for LayoutRuns {
    fn deref_mut(&mut self) -> &mut Vec<LayoutRun> {
        let (value, galley) = (&self.value, &self.galley);
        self.changed.get_or_insert_with(|| {
            value.get_or_init(|| if galley.rows.len() <= 1 { TextLayout::build_row_runs(galley, 0) } else { Vec::new() }).clone()
        })
    }
}

/// A laid-out run.
#[non_exhaustive]
pub struct TextLayout {
    /// The galley.
    pub galley: Arc<Galley>,
    /// The layout cut text.
    pub elided: bool,
    /// The full text, as the tooltip and accessible name of an elided run.
    pub full: LayoutText,
    /// The first row's baseline, from the layout's top.
    pub first_baseline: f32,
    /// The bidi runs of a single-row layout; empty for a wrapped one.
    pub runs: LayoutRuns,
    /// The run's paint angle, in radians clockwise about the layout's top-left corner.
    pub rotation: f32,
}

impl Clone for TextLayout {
    fn clone(&self) -> Self {
        Self {
            galley: self.galley.clone(),
            elided: self.elided,
            full: self.full.shared(),
            first_baseline: self.first_baseline,
            runs: self.runs.shared(),
            rotation: self.rotation,
        }
    }
}

impl TextLayout {
    pub(crate) fn new(galley: Arc<Galley>, elided: bool, full: String) -> TextLayout {
        let original = (galley.job.text != full).then(|| Arc::<str>::from(full));
        Self::from_job(galley, elided, original)
    }

    pub(super) fn from_job(galley: Arc<Galley>, elided: bool, full: Option<Arc<str>>) -> TextLayout {
        let first_baseline = galley.rows.first().map_or(0.0, |placed| match placed.row.glyphs.first() {
            Some(glyph) => placed.pos.y + glyph.pos.y,
            None => placed.pos.y + placed.row.size.y,
        });
        let full = LayoutText { galley: galley.clone(), original: full, value: Arc::new(OnceLock::new()), changed: None };
        let runs = LayoutRuns { galley: galley.clone(), value: Arc::new(OnceLock::new()), changed: None };
        TextLayout { galley, elided, full, first_baseline, runs, rotation: 0.0 }
    }

    /// The full text, as the tooltip and accessible name of an elided run.
    pub fn full(&self) -> &str {
        &self.full
    }

    /// The bidi runs of a single-row layout; empty for a wrapped one.
    pub fn runs(&self) -> &[LayoutRun] {
        &self.runs
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
                let runs = if self.galley.rows.len() == 1 && self.runs.changed.is_none() {
                    self.runs().to_vec()
                } else {
                    Self::build_row_runs(&self.galley, at)
                };
                (text, egui::Rect::from_min_size(placed.pos, placed.row.size), runs)
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

#[cfg(feature = "testing")]
mod memo_tests {
    use super::{
        Arc, Context, Cow, Direction, HAlign, LayoutRun, Span, StackFacts, StyleColor, TextEnv, TextRun, TypeRole, UiTokens, Wrap, with_memo,
    };
    use crate::testing::trial;
    use craft_theme::{Appearance, Contrast, Mode, Size, TextSize, builtins, resolve};

    #[trial]
    fn layouts_reuse_equal_inputs_and_font_updates_invalidate_them() {
        let appearance =
            Appearance { mode: Mode::Dark, contrast: Contrast::Standard, size: Size::Compact, text_size: TextSize::Small, reduced_motion: false };
        let tokens = UiTokens::new(Arc::new(resolve(&builtins()[0], appearance, None).tokens), Contrast::Standard, Size::Compact);
        let ctx = Context::default();
        let mut output = ctx.run_pass(Default::default(), |ctx| {
            let env = TextEnv {
                tokens: &tokens,
                direction: Direction::Ltr,
                ink: crate::style::rgba_color(tokens.tokens().color.text.normal),
                fonts: StackFacts::default(),
            };
            let run = TextRun::new("a label long enough to elide").elide();
            let first = run.layout(ctx, &env, 80.0);
            let second = run.layout(ctx, &env, 80.0);
            assert!(Arc::ptr_eq(&first.galley, &second.galley));
            assert!(first.runs.value.get().is_none());
            assert!(first.full.value.get().is_none());
            let mut cold = first.clone();
            cold.full.push('?');
            cold.runs.clear();
            assert!(cold.runs().is_empty());
            assert_eq!(first.full(), "a label long enough to elide");
            assert_ne!(cold.full(), first.full());
            assert!(second.full.value.get().is_some());
            let _ = first.runs();
            assert!(second.runs.value.get().is_some());
            let _: String = first.full.clone();
            let _: Vec<LayoutRun> = first.runs.clone();
            let mut changed = first.clone();
            changed.full.push('!');
            changed.runs.clear();
            assert_eq!(first.full(), "a label long enough to elide");
            assert_ne!(changed.full(), first.full());
            assert!(!first.runs().is_empty());
            assert!(changed.runs().is_empty());
            let again = run.layout(ctx, &env, 80.0);
            assert_eq!(again.full(), first.full());
            assert_eq!(again.runs(), first.runs());
            let _ = run.layout(ctx, &env, 120.0);
            let _ = run.least_width(ctx, &env);
            let least_key = run.layout_key(ctx, &env, f32::INFINITY);
            with_memo(ctx, |memo| {
                assert_eq!(memo.order.len(), 3);
                assert!(memo.get(&least_key).is_some_and(|entry| entry.least.is_some()));
            });
            let mut variants = Vec::new();
            let mut changed = run.clone();
            changed.spans[0].text = Cow::Borrowed("a different label");
            variants.push(changed);
            let mut changed = run.clone();
            changed.spans = vec![Span::new("a label "), Span::new("long enough to elide")];
            variants.push(changed);
            let mut changed = run.clone();
            changed.spans[0].italic = true;
            variants.push(changed);
            let mut changed = run.clone();
            changed.spans[0].tabular = true;
            variants.push(changed);
            let mut changed = run.clone();
            changed.spans[0].mono = true;
            variants.push(changed);
            let mut changed = run.clone();
            changed.spans[0].muted = true;
            variants.push(changed);
            let mut changed = run.clone();
            changed.spans[0].shortcut = true;
            variants.push(changed);
            let mut changed = run.clone();
            changed.spans[0].highlight = true;
            variants.push(changed);
            let mut changed = run.clone();
            changed.spans[0].underline = true;
            variants.push(changed);
            let mut changed = run.clone();
            changed.spans[0].strong = true;
            variants.push(changed);
            let mut changed = run.clone();
            changed.color = Some(StyleColor(crate::style::rgba_color(tokens.tokens().color.text.muted)));
            variants.push(changed);
            let mut changed = run.clone();
            changed.role = TypeRole::Body;
            variants.push(changed);
            let mut changed = run.clone();
            changed.wrap = Wrap::Clip;
            variants.push(changed);
            let mut changed = run.clone();
            changed.align = HAlign::Center;
            variants.push(changed);
            let mut changed = run.clone();
            changed.ltr = true;
            variants.push(changed);
            let mut changed = run.clone();
            changed.rotation = 0.5;
            variants.push(changed);
            for changed in variants {
                let before = with_memo(ctx, |memo| memo.order.len());
                let _ = changed.layout(ctx, &env, 80.0);
                let _ = changed.layout(ctx, &env, 80.0);
                assert_eq!(with_memo(ctx, |memo| memo.order.len()), before + 1);
            }
            let rtl = TextEnv { direction: Direction::Rtl, ..env };
            let _ = run.layout(ctx, &rtl, 80.0);
            let ink = TextEnv { ink: crate::style::rgba_color(tokens.tokens().color.text.muted), ..env };
            assert_ne!(ink.ink, env.ink);
            let _ = run.layout(ctx, &ink, 80.0);
            let fonts = TextEnv { fonts: StackFacts { semibold: true, ..env.fonts }, ..env };
            let _ = run.layout(ctx, &fonts, 80.0);
            let fonts = TextEnv { fonts: StackFacts { system_ui: true, ..env.fonts }, ..env };
            let _ = run.layout(ctx, &fonts, 80.0);
            let fonts = TextEnv { fonts: StackFacts { line_ratio: 1.25, ..env.fonts }, ..env };
            let _ = run.layout(ctx, &fonts, 80.0);
            let mut changed_tokens = (**tokens.tokens()).clone();
            changed_tokens.color.text.muted = craft_theme::Rgba([12, 34, 56, 255]);
            let changed_tokens = UiTokens::new(Arc::new(changed_tokens), Contrast::Standard, Size::Compact);
            let styled = TextEnv { tokens: &changed_tokens, ..env };
            let _ = run.layout(ctx, &styled, 80.0);
            let before = with_memo(ctx, |memo| memo.order.len());
            assert_eq!(before, 25);
        });
        output.textures_delta.clear();
        // A direct egui font replacement bypasses framework hooks; the sentinel must detect it.
        let mut definitions = ctx.fonts(|fonts| fonts.definitions().clone());
        definitions.families.insert(egui::FontFamily::Name("memo trial".into()), Vec::new());
        ctx.set_fonts(definitions);
        let mut output = ctx.run_pass(Default::default(), |ctx| {
            with_memo(ctx, |memo| assert!(memo.order.is_empty()));
        });
        output.textures_delta.clear();
    }

    #[trial]
    fn idle_text_reuses_galleys_and_changed_text_lays_out_once_per_width() {
        let appearance =
            Appearance { mode: Mode::Dark, contrast: Contrast::Standard, size: Size::Compact, text_size: TextSize::Small, reduced_motion: false };
        let tokens = UiTokens::new(Arc::new(resolve(&builtins()[0], appearance, None).tokens), Contrast::Standard, Size::Compact);
        let ctx = Context::default();
        let run = TextRun::new("the initial label for the cache trial").wrap(Wrap::Wrap);
        let env = TextEnv {
            tokens: &tokens,
            direction: Direction::Ltr,
            ink: crate::style::rgba_color(tokens.tokens().color.text.normal),
            fonts: StackFacts::default(),
        };
        let mut output = ctx.run_pass(Default::default(), |ctx| {
            let _ = run.layout(ctx, &env, 80.0);
            let _ = run.layout(ctx, &env, 120.0);
        });
        output.textures_delta.clear();
        let mut output = ctx.run_pass(Default::default(), |ctx| {
            let before = ctx.fonts(|fonts| fonts.num_galleys_in_cache());
            let _ = run.layout(ctx, &env, 80.0);
            let _ = run.layout(ctx, &env, 120.0);
            assert_eq!(ctx.fonts(|fonts| fonts.num_galleys_in_cache()), before);
            let changed = TextRun::new("a different label for the cache trial").wrap(Wrap::Wrap);
            let _ = changed.layout(ctx, &env, 80.0);
            let _ = changed.layout(ctx, &env, 120.0);
            assert_eq!(ctx.fonts(|fonts| fonts.num_galleys_in_cache()), before + 2);
        });
        output.textures_delta.clear();
        let scale = 2.0;
        ctx.set_pixels_per_point(scale);
        let mut output = ctx.run_pass(Default::default(), |ctx| {
            assert_eq!(ctx.pixels_per_point(), scale);
            let _ = run.layout(ctx, &env, 80.0);
            let after = with_memo(ctx, |memo| memo.order.len());
            let _ = run.layout(ctx, &env, 80.0);
            assert_eq!(with_memo(ctx, |memo| memo.order.len()), after);
            assert_eq!(after, 1);
        });
        output.textures_delta.clear();
    }
}
