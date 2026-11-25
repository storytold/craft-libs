//! `TextRun`, the one description of UI text for measure, layout and paint, and `TextLayout`, its laid-out form.

use crate::fonts::StackFacts;
use crate::runtime::egui::memory;
use crate::runtime::egui::text_layout;
pub use crate::style::Span;
use crate::style::text_measure::{paragraph_job, span_format};
use crate::style::{DataColor, LetterCase, StyleColor, TypeRole, UiTokens};
use craft_i18n::Direction;
use egui::epaint::text::FontCacheRevision;
use egui::text::{LayoutJob, TextWrapping};
use egui::{Color32, Context, Galley};
use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
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
    /// The colour of the link spans; `None` takes the link look's.
    pub link_color: Option<Color32>,
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
            link_color: None,
        }
    }

    /// A run of `spans`.
    pub fn spans(spans: Vec<Span<'a>>) -> Self {
        TextRun { spans, role: TypeRole::Label, color: None, wrap: Wrap::Line, ltr: false, align: HAlign::Start, rotation: 0.0, link_color: None }
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
        if role.letter_case() == LetterCase::Upper {
            for span in &mut self.spans {
                span.text = Cow::Owned(span.text.to_uppercase());
            }
        }
        self
    }

    /// Sets the colour of the run's link spans.
    pub fn link_color(mut self, color: Color32) -> Self {
        self.link_color = Some(color);
        self
    }

    /// Sets the colour.
    pub fn color(mut self, color: StyleColor) -> Self {
        self.color = Some(color);
        self
    }

    /// Sets the colour from data, painted as given: text drawn over document content (a smart guide's label in the
    /// guide's colour), whose colour belongs to the document and not to a state. It fills the colour slot of
    /// [`color`](Self::color), as [`IconSpec::data_color`](super::IconSpec::data_color) does for an icon.
    pub fn data_color(mut self, color: DataColor) -> Self {
        self.color = Some(StyleColor(color.0));
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
#[derive(Clone)]
struct LayoutKey {
    run: TextRun<'static>,
    tokens: u64,
    token_values: UiTokens,
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
    token_values: &'a UiTokens,
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
    /// The pass that last read or wrote the entry.
    used: u64,
    /// What the entry retains, as [`retained_bytes`] estimates it.
    bytes: usize,
}

/// Passes an entry stays without being read: egui's galley cache drops a galley unused for one pass; the memo keeps
/// an entry longer, so text whose body replays its kept measurement for a few passes still finds its layout when the
/// body runs again.
const UNUSED_PASSES: u64 = 60;
/// The most the memo retains in all, as [`retained_bytes`] estimates it: the per-entry caps in `put` bound one
/// entry, this bounds many wide paragraphs at many widths.
const BUDGET_BYTES: usize = 32 * 1024 * 1024;
/// Where an eviction over the budget stops, below the budget so that one eviction covers many new entries.
const EVICT_TO_BYTES: usize = BUDGET_BYTES / 4 * 3;

#[derive(Default)]
struct TextMemo {
    entries: HashMap<u64, Vec<CachedText>>,
    /// The sum of the entries' `bytes`.
    bytes: usize,
    /// The current pass, as of the last access.
    pass: u64,
    generation: u64,
    font_revision: Option<FontCacheRevision>,
}

fn with_memo<R>(ctx: &Context, f: impl FnOnce(&mut TextMemo) -> R) -> R {
    // Mark the viewport as using fonts so same-pass changes wait, just as a fresh layout would.
    let font_revision = ctx.fonts_mut(|fonts| fonts.cache_revision_and_mark_used());
    let pass = ctx.cumulative_pass_nr();
    memory::with::<TextMemo, _>(ctx, |memo| {
        if memo.font_revision.as_ref() != Some(&font_revision) {
            memo.entries.clear();
            memo.bytes = 0;
            memo.generation = memo.generation.wrapping_add(1);
            memo.font_revision = Some(font_revision);
        }
        if memo.pass != pass {
            memo.pass = pass;
            memo.retain(|entry| pass.saturating_sub(entry.used) < UNUSED_PASSES);
        }
        f(memo)
    })
}

/// What a memo entry keeps alive: its key's text, and its galley's text, glyphs and mesh.
fn retained_bytes(key: &LayoutKey, layout: Option<&TextLayout>) -> usize {
    use egui::epaint::Vertex;
    use egui::epaint::text::{Glyph, LayoutSection};
    use std::mem::size_of;
    let text: usize = key.run.spans.iter().map(|span| size_of::<Span<'_>>() + span.text.len()).sum();
    let galley = layout.map_or(0, |layout| {
        let galley = &layout.galley;
        let glyphs: usize = galley.rows.iter().map(|placed| placed.row.glyphs.len()).sum();
        galley.job.text.len()
            + galley.job.sections.len() * size_of::<LayoutSection>()
            + glyphs * size_of::<Glyph>()
            + galley.num_vertices * size_of::<Vertex>()
            + galley.num_indices * size_of::<u32>()
            + layout.full.as_ref().map_or(0, |full| full.len())
    });
    size_of::<CachedText>() + text + galley
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
            && key.token_values.same_source_tokens(self.token_values)
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
                strikethrough: span.strikethrough,
                strong: span.strong,
                link: span.link,
                flag: span.flag,
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
                link_color: self.run.link_color,
            },
            tokens: self.tokens,
            token_values: self.token_values.clone(),
            direction: self.direction,
            ink: self.ink,
            fonts: self.fonts,
            width: self.width,
            scale: self.scale,
        }
    }
}

impl TextMemo {
    /// The entry for `key`, marked as used in this pass.
    fn get(&mut self, key: &LayoutQuery<'_>) -> Option<&CachedText> {
        let pass = self.pass;
        let entry = self.entries.get_mut(&key.hash())?.iter_mut().find(|entry| key.matches(&entry.key))?;
        entry.used = pass;
        Some(entry)
    }

    fn put(&mut self, key: &LayoutQuery<'_>, layout: Option<Arc<TextLayout>>, least: Option<f32>) {
        // Large document text should not pin galleys in a UI-label cache.
        if key.run.spans.len() > 1024 || key.run.spans.iter().fold(0usize, |len, span| len.saturating_add(span.text.len())) > 16 * 1024 {
            return;
        }
        let hash = key.hash();
        let pass = self.pass;
        if let Some(entry) = self.entries.get_mut(&hash).and_then(|bucket| bucket.iter_mut().find(|entry| key.matches(&entry.key))) {
            if layout.is_some() {
                entry.layout = layout;
            }
            if least.is_some() {
                entry.least = least;
            }
            entry.used = pass;
            let bytes = retained_bytes(&entry.key, entry.layout.as_deref());
            self.bytes = self.bytes - entry.bytes + bytes;
            entry.bytes = bytes;
            return;
        }
        let key = key.owned();
        let bytes = retained_bytes(&key, layout.as_deref());
        if self.bytes + bytes > BUDGET_BYTES {
            self.evict_to(EVICT_TO_BYTES.saturating_sub(bytes));
        }
        self.bytes += bytes;
        self.entries.entry(hash).or_default().push(CachedText { key, layout, least, used: pass, bytes });
    }

    /// Drops the least recently used entries until the memo retains no more than `bytes`.
    fn evict_to(&mut self, bytes: usize) {
        let mut ages: Vec<(u64, usize)> = self.entries.values().flatten().map(|entry| (entry.used, entry.bytes)).collect();
        ages.sort_unstable_by_key(|&(used, _)| used);
        let mut left = self.bytes;
        let mut last = None;
        for (used, size) in ages {
            if left <= bytes {
                break;
            }
            left -= size;
            last = Some(used);
        }
        if let Some(last) = last {
            self.retain(|entry| entry.used > last);
        }
    }

    fn retain(&mut self, keep: impl Fn(&CachedText) -> bool) {
        let mut bytes = 0;
        self.entries.retain(|_, bucket| {
            bucket.retain(|entry| keep(entry));
            bytes += bucket.iter().map(|entry| entry.bytes).sum::<usize>();
            !bucket.is_empty()
        });
        self.bytes = bytes;
    }

    #[cfg(all(feature = "testing", any(test, feature = "suite")))]
    fn len(&self) -> usize {
        self.entries.values().map(Vec::len).sum()
    }
}

impl TextRun<'_> {
    /// All inputs read by `job`, the cutting and row-alignment steps.
    fn layout_key<'a>(&'a self, ctx: &Context, env: &'a TextEnv<'_>, max_width: f32) -> LayoutQuery<'a> {
        LayoutQuery {
            run: self,
            tokens: env.tokens.key(),
            token_values: env.tokens,
            direction: env.direction,
            ink: env.ink,
            fonts: env.fonts,
            // A `Line` run lays out at infinite width whatever the room (`job`), so every room shares one entry.
            width: if self.wrap == Wrap::Line { f32::INFINITY } else { max_width }.to_bits(),
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
        // Every cut goes through the one cut that `least` measures, so a run shown at its least keeps its
        // readable characters.
        let galley = match &full {
            Some(full) => self.layout_cut(ctx, env, full, max_width),
            None => {
                let job = self.job(env, self.spans.iter().map(|s| (s, s.text.as_ref())), max_width);
                let galley = text_layout::layout(ctx, job.clone());
                if matches!(self.wrap, Wrap::Wrap | Wrap::Clamp(_)) && galley.size().x > max_width {
                    // A single unbroken word can be wider than the room even after word wrapping.
                    let mut job = job;
                    job.wrap.break_anywhere = true;
                    text_layout::layout(ctx, job)
                } else {
                    galley
                }
            }
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

    /// The whole layout and the width at its readable least, sharing the cached unbounded layout.
    pub(crate) fn least(&self, ctx: &Context, env: &TextEnv<'_>) -> LeastLayout {
        let whole = self.layout(ctx, env, f32::INFINITY);
        let width = self.least_width(ctx, env);
        LeastLayout { width, whole }
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
        let whole = self.layout(ctx, env, f32::INFINITY).size().x;
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
            } else if span.link {
                self.link_color.unwrap_or(base)
            } else {
                base
            };
            let background = if span.highlight { to_color(env.tokens.color.state.r#match) } else { Color32::TRANSPARENT };
            let mut format = span_format(env.tokens, env.fonts, self.role, span, color, background);
            if span.underline {
                format.underline = egui::Stroke::new(env.tokens.stroke.thin, color);
            }
            if span.strikethrough {
                format.strikethrough = egui::Stroke::new(env.tokens.stroke.thin, color);
            }
            if let Some(tone) = span.flag {
                format.underline = egui::Stroke::new(env.tokens.stroke.thin, to_color(tone.ink(env.tokens)));
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

/// A run laid out whole on its line, and its width at its readable least: what a text that elides measures at
/// `Proposal::Min`.
#[derive(Clone)]
#[non_exhaustive]
pub struct LeastLayout {
    /// The run laid out whole, as a layout in unbounded room lays it out.
    pub whole: TextLayout,
    /// The width at its readable least; never wider than `whole`.
    pub width: f32,
}

impl LeastLayout {
    /// The size the run takes at its least: its least width and its whole line's height.
    pub fn size(&self) -> egui::Vec2 {
        egui::Vec2::new(self.width, self.whole.size().y)
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
    /// The full text when it differs from the galley's, which a cut shortened.
    full: Option<Arc<str>>,
    /// The first row's baseline, from the layout's top.
    pub first_baseline: f32,
    /// The bidi runs of a single-row layout, built on first read and shared by the layout's clones.
    runs: Arc<OnceLock<Vec<LayoutRun>>>,
    /// The run's paint angle, in radians clockwise about the layout's top-left corner.
    pub rotation: f32,
}

impl TextLayout {
    pub(crate) fn new(galley: Arc<Galley>, elided: bool, full: String) -> TextLayout {
        let full = (galley.job.text != full).then(|| Arc::<str>::from(full));
        Self::from_job(galley, elided, full)
    }

    /// The layout of `galley`; `full` is the full text when it differs from the galley's.
    pub(super) fn from_job(galley: Arc<Galley>, elided: bool, full: Option<Arc<str>>) -> TextLayout {
        let first_baseline = galley.rows.first().map_or(0.0, |placed| match placed.row.glyphs.first() {
            Some(glyph) => placed.pos.y + glyph.pos.y,
            None => placed.pos.y + placed.row.size.y,
        });
        TextLayout { galley, elided, full, first_baseline, runs: Arc::default(), rotation: 0.0 }
    }

    /// The layout with `galley` in place of its own, the same text placed elsewhere.
    pub(super) fn with_galley(self, galley: Arc<Galley>) -> TextLayout {
        TextLayout { galley, runs: Arc::default(), ..self }
    }

    /// The full text, as the tooltip and accessible name of an elided run.
    pub fn full(&self) -> &str {
        self.full.as_deref().unwrap_or(&self.galley.job.text)
    }

    /// The bidi runs of a single-row layout; empty for a wrapped one.
    pub fn runs(&self) -> &[LayoutRun] {
        self.runs.get_or_init(|| if self.galley.rows.len() <= 1 { Self::build_row_runs(&self.galley, 0) } else { Vec::new() })
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

    /// Every row as its text, its rect from the galley's origin and its bidi runs: the one per-row geometry, read by
    /// the AccessKit run nodes and by a component that hits parts of a wrapped run, such as a link in a sentence.
    pub fn row_lines(&self) -> Vec<(String, egui::Rect, Vec<LayoutRun>)> {
        (0..self.galley.rows.len())
            .map(|at| {
                let placed = &self.galley.rows[at];
                let text = placed.row.glyphs.iter().map(|glyph| glyph.chr).collect();
                let runs = if self.galley.rows.len() == 1 { self.runs().to_vec() } else { Self::build_row_runs(&self.galley, at) };
                (text, egui::Rect::from_min_size(placed.pos, placed.row.size), runs)
            })
            .collect()
    }

    /// The galley size.
    pub fn size(&self) -> egui::Vec2 {
        self.galley.size()
    }

    /// Where a caret before character `chars` (counted from the start of the text) stands, from the layout's left edge:
    /// an input method's cursor inside its composition. Past the end, the end of the text.
    pub fn char_x(&self, chars: usize) -> f32 {
        self.galley.pos_from_cursor(egui::text::CCursor::new(chars)).min.x
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

#[cfg(all(feature = "testing", any(test, feature = "suite")))]
pub(crate) mod memo_tests {
    use super::{
        Arc, BUDGET_BYTES, Context, Cow, Direction, HAlign, Span, StackFacts, StyleColor, TextEnv, TextRun, TypeRole, UNUSED_PASSES, UiTokens, Wrap,
        with_memo,
    };
    use crate::testing::trial;
    use craft_theme::{Appearance, Contrast, Mode, Size, TextSize, builtins, resolve};

    fn tokens() -> UiTokens {
        let appearance =
            Appearance { mode: Mode::Dark, contrast: Contrast::Standard, size: Size::Compact, text_size: TextSize::Small, reduced_motion: false };
        UiTokens::new(Arc::new(resolve(&builtins()[0], appearance, None).tokens), Contrast::Standard, Size::Compact)
    }

    fn env(tokens: &UiTokens) -> TextEnv<'_> {
        TextEnv { tokens, direction: Direction::Ltr, ink: crate::style::rgba_color(tokens.tokens().color.text.normal), fonts: StackFacts::default() }
    }

    /// Runs one pass of `ctx` with `f`.
    fn pass(ctx: &Context, f: impl FnMut(&Context)) {
        let mut output = ctx.run_pass(Default::default(), f);
        output.textures_delta.clear();
    }

    fn entries(ctx: &Context) -> usize {
        with_memo(ctx, |memo| memo.len())
    }

    #[trial]
    fn cached_layout_does_not_call_egui_layout_for_a_sentinel() {
        let tokens = tokens();
        let env = env(&tokens);
        let ctx = Context::default();
        let run = TextRun::new("cached layout");
        let mut first = None;
        pass(&ctx, |ctx| first = Some(run.layout(ctx, &env, 100.0).galley));
        // Let egui evict its own galleys while our external memo retains the layout.
        for _ in 0..2 {
            pass(&ctx, |_| {});
        }
        pass(&ctx, |ctx| {
            assert_eq!(ctx.fonts(|fonts| fonts.num_galleys_in_cache()), 0);
            let cached = run.layout(ctx, &env, 100.0);
            assert!(first.as_ref().is_some_and(|first| Arc::ptr_eq(first, &cached.galley)));
            assert_eq!(ctx.fonts(|fonts| fonts.num_galleys_in_cache()), 0, "a memo hit must not call egui layout_job");
        });
    }

    #[trial]
    fn layouts_reuse_equal_inputs_and_font_updates_invalidate_them() {
        let tokens = tokens();
        let env = env(&tokens);
        let ctx = Context::default();
        pass(&ctx, |ctx| {
            let run = TextRun::new("a label long enough to elide").elide();
            let first = run.layout(ctx, &env, 80.0);
            let second = run.layout(ctx, &env, 80.0);
            assert!(Arc::ptr_eq(&first.galley, &second.galley));
            // The runs are built on first read, once for every copy of the memo's layout.
            assert!(first.runs.get().is_none());
            assert_eq!(first.full(), "a label long enough to elide");
            assert!(!first.runs().is_empty());
            assert!(second.runs.get().is_some());
            let again = run.layout(ctx, &env, 80.0);
            assert_eq!(again.full(), first.full());
            assert_eq!(again.runs(), first.runs());
            let before = entries(ctx);
            let _ = run.layout(ctx, &env, 120.0);
            let _ = run.least_width(ctx, &env);
            let least_key = run.layout_key(ctx, &env, f32::INFINITY);
            with_memo(ctx, |memo| {
                assert_eq!(memo.len(), before + 2, "a new room and the least width each add an entry");
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
            changed.spans[0].flag = Some(craft_theme::variant::Tone::Bad);
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
                let before = entries(ctx);
                let _ = changed.layout(ctx, &env, 80.0);
                let _ = changed.layout(ctx, &env, 80.0);
                assert_eq!(entries(ctx), before + 1);
            }
            let rtl = TextEnv { direction: Direction::Rtl, ..env };
            let ink = TextEnv { ink: crate::style::rgba_color(tokens.tokens().color.text.muted), ..env };
            assert_ne!(ink.ink, env.ink);
            let semibold = TextEnv { fonts: StackFacts { semibold: true, ..env.fonts }, ..env };
            let system_ui = TextEnv { fonts: StackFacts { system_ui: true, ..env.fonts }, ..env };
            let line_ratio = TextEnv { fonts: StackFacts { line_ratio: 1.25, ..env.fonts }, ..env };
            for changed in [rtl, ink, semibold, system_ui, line_ratio] {
                let before = entries(ctx);
                let _ = run.layout(ctx, &changed, 80.0);
                assert_eq!(entries(ctx), before + 1);
            }
            let mut changed_tokens = (**tokens.tokens()).clone();
            let equal_tokens = UiTokens::new(Arc::new(changed_tokens.clone()), Contrast::Standard, Size::Compact);
            let equal_env = TextEnv { tokens: &equal_tokens, ..env };
            let equal_key = run.layout_key(ctx, &equal_env, 80.0);
            with_memo(ctx, |memo| assert!(memo.get(&equal_key).is_some(), "equal token values from distinct Arcs may reuse the entry"));
            changed_tokens.color.text.muted = craft_theme::Rgba([12, 34, 56, 255]);
            let changed_tokens = UiTokens::new(Arc::new(changed_tokens), Contrast::Standard, Size::Compact);
            let styled = TextEnv { tokens: &changed_tokens, ..env };
            let mut collision = run.layout_key(ctx, &styled, 80.0);
            collision.tokens = tokens.key();
            with_memo(ctx, |memo| assert!(memo.get(&collision).is_none(), "a token-hash collision must not reuse a different token set"));
            let before = entries(ctx);
            let _ = run.layout(ctx, &styled, 80.0);
            assert_eq!(entries(ctx), before + 1);
        });
        // A direct egui font replacement bypasses framework hooks; the revision must detect it.
        let mut definitions = ctx.fonts(|fonts| fonts.definitions().clone());
        definitions.families.insert(egui::FontFamily::Name("memo trial".into()), Vec::new());
        ctx.set_fonts(definitions);
        pass(&ctx, |ctx| assert_eq!(entries(ctx), 0));
    }

    #[trial]
    fn idle_text_reuses_galleys_and_changed_text_lays_out_once_per_width() {
        let tokens = tokens();
        let env = env(&tokens);
        let ctx = Context::default();
        let run = TextRun::new("the initial label for the cache trial").wrap(Wrap::Wrap);
        pass(&ctx, |ctx| {
            let _ = run.layout(ctx, &env, 80.0);
            let _ = run.layout(ctx, &env, 120.0);
        });
        pass(&ctx, |ctx| {
            let before = ctx.fonts(|fonts| fonts.num_galleys_in_cache());
            let _ = run.layout(ctx, &env, 80.0);
            let _ = run.layout(ctx, &env, 120.0);
            assert_eq!(ctx.fonts(|fonts| fonts.num_galleys_in_cache()), before);
            let changed = TextRun::new("a different label for the cache trial").wrap(Wrap::Wrap);
            let _ = changed.layout(ctx, &env, 80.0);
            let _ = changed.layout(ctx, &env, 120.0);
            assert_eq!(ctx.fonts(|fonts| fonts.num_galleys_in_cache()), before + 2);
        });
        let scale = 2.0;
        ctx.set_pixels_per_point(scale);
        pass(&ctx, |ctx| {
            assert_eq!(ctx.pixels_per_point(), scale);
            let before = entries(ctx);
            let _ = run.layout(ctx, &env, 80.0);
            let after = entries(ctx);
            let _ = run.layout(ctx, &env, 80.0);
            assert_eq!(entries(ctx), after);
            assert_eq!(after, before + 1, "a new scale has its own cached layout");
        });
    }

    #[trial]
    fn a_line_label_keeps_one_entry_at_every_room() {
        let tokens = tokens();
        let env = env(&tokens);
        let ctx = Context::default();
        let line = TextRun::new("a one-line label");
        let wrapped = TextRun::new("a wrapping paragraph").wrap(Wrap::Wrap);
        pass(&ctx, |ctx| {
            let first = line.layout(ctx, &env, 100.0);
            let before = entries(ctx);
            // A window resize offers the label a new room every pass.
            for room in (101..150).map(|room| room as f32) {
                assert!(Arc::ptr_eq(&line.layout(ctx, &env, room).galley, &first.galley), "a Line run hits at room {room}");
            }
            let _ = line.least_width(ctx, &env);
            assert_eq!(entries(ctx), before, "the label's rooms and its least width share one entry");
            for room in [100.0, 120.0, 140.0] {
                let _ = wrapped.layout(ctx, &env, room);
            }
            assert_eq!(entries(ctx), before + 3, "a wrapping run keeps one entry per room");
        });
    }

    #[trial]
    fn a_used_label_stays_while_a_changing_label_churns() {
        let tokens = tokens();
        let env = env(&tokens);
        let ctx = Context::default();
        let label = TextRun::new("a static label");
        let mut first = None;
        let passes = 3 * UNUSED_PASSES;
        for at in 0..passes {
            pass(&ctx, |ctx| {
                let shown = label.layout(ctx, &env, 100.0).galley;
                assert!(Arc::ptr_eq(first.get_or_insert_with(|| shown.clone()), &shown), "the static label hits in pass {at}");
                let _ = TextRun::new(format!("frame {at}")).layout(ctx, &env, 100.0);
            });
        }
        // Each pass leaves one entry for the changing label; only the passes still kept hold theirs.
        let kept = entries(&ctx);
        assert!(kept > 1 && (kept as u64) <= UNUSED_PASSES + 1, "{kept} entries after {passes} passes");
    }

    #[trial]
    fn many_wide_paragraphs_at_many_widths_stay_within_the_budget() {
        let tokens = tokens();
        let env = env(&tokens);
        let ctx = Context::default();
        let paragraph = "A paragraph wide enough to wrap into many rows at every width the trial offers. ".repeat(150);
        let run = TextRun::new(&paragraph).wrap(Wrap::Wrap);
        pass(&ctx, |ctx| {
            let mut added = 0;
            let mut room = 200.0;
            // Lay out until the layouts would have retained twice the budget.
            while added <= 2 * BUDGET_BYTES {
                let layout = run.layout(ctx, &env, room);
                added += super::retained_bytes(&run.layout_key(ctx, &env, room).owned(), Some(&layout));
                with_memo(ctx, |memo| {
                    assert!(memo.bytes <= BUDGET_BYTES, "{} bytes retained", memo.bytes);
                    assert_eq!(memo.bytes, memo.entries.values().flatten().map(|entry| entry.bytes).sum::<usize>());
                });
                assert!(Arc::ptr_eq(&run.layout(ctx, &env, room).galley, &layout.galley), "the newest layout is kept");
                room += 7.0;
            }
        });
    }
}
