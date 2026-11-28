//! `WordRun`: a wrapping run of pickable words in one Tab stop.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use crate::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, Name, Outcome, PaintUi, PartsCx, Pointer as _, Props as _,
    Role, SetPosition, Styled as _,
};
use crate::primitives::{PaintShape, Span, TextRun};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, Flags, FocusLook, StyleColor, StyleCx, TextLook, TypeRole, UiTokens};
use craft_behavior::{Flow, Metrics, NavShape, PartKey, PartRef, Proposal, Roving, Sense, Value};
use craft_theme::{Rgba, Stateful, States};
use egui::{CornerRadius, Rect, Response, Vec2, pos2, vec2};
use std::ops::Deref;

/// A word of the run: its text on its fill.
const WORD: PartKey = PartKey::new("word");

/// How a word of a `WordRun` shows.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WordState {
    /// Plain.
    #[default]
    Normal,
    /// Struck through, in the faint text colour.
    Struck,
    /// Underlined.
    Underlined,
}

/// A word of a `WordRun`.
#[derive(Clone, Debug, PartialEq)]
pub struct RunWord<K> {
    /// What the word is picked as.
    pub key: K,
    /// The word's text.
    pub text: String,
    /// How the word shows.
    pub state: WordState,
}

/// The style of a word run: the body role in the normal text colour, a struck word in the faint one, and the fill
/// under each word, clear until hovered and the text-selection fill under the picked word.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct WordRunStyle {
    /// The gap between words on a line.
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// A word's text.
    #[style(text)]
    text: TextLook,
    /// A struck word's text and its line.
    #[style(text)]
    faint: TextLook,
    /// The fill under a word.
    fill: Stateful<StyleColor>,
    /// The corners of a word's fill.
    radius: CornerRadius,
    /// The room on each side of a word's text, inside its fill.
    pad: f32,
    /// The room a word's text leaves in its fill on both sides together.
    pad_total: f32,
}

/// A wrapping run of pickable words in one Tab stop: the arrows move focus over the words in reading order, Home and
/// End to the first and last, and Space, Enter or a click picks the focused word into `selected`. Tab enters the run
/// on the picked word, else on the first. Its node is a list box with an option per word.
#[derive(crate::contract::Component)]
#[component(probe(WordRun<'static, usize>))]
pub struct WordRun<'a, K: Value + 'static> {
    #[common]
    common: Common,
    words: &'a [RunWord<K>],
    selected: Bind<'a, Option<K>>,
    /// The key of the word picked this pass; set by `update`.
    picked: Option<K>,
}

impl<'a, K: Value + 'static> WordRun<'a, K> {
    /// A run named `label` of `words`; the picked word's key is `selected`.
    pub fn new(label: impl Into<Label>, words: &'a [RunWord<K>], selected: Bind<'a, Option<K>>) -> Self {
        WordRun { common: Common::new(), words, selected, picked: None }.label(label)
    }

    fn part(at: usize) -> PartRef {
        WORD.at(at as u64)
    }
}

/// What a `WordRun` returns.
#[non_exhaustive]
pub struct WordRunOutput<K> {
    /// The edit response of the picked key.
    pub edit: EditResponse<Option<K>>,
    /// The key of the word picked in this pass.
    pub picked: Option<K>,
}

impl<K> Deref for WordRunOutput<K> {
    type Target = EditResponse<Option<K>>;

    fn deref(&self) -> &Self::Target {
        &self.edit
    }
}

impl<K: Value> Outcome for WordRunOutput<K> {
    type Value = Option<K>;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// The text of `word` in `role`, underlined or struck through as the word's state says.
fn word_text<K>(word: &RunWord<K>, role: TypeRole) -> TextRun<'_> {
    let span = Span::new(word.text.as_str());
    let span = match word.state {
        WordState::Underlined => span.underline(),
        // PLAN: assumed Span::strikethrough(self) -> Self (pdf-chrome-w3)
        WordState::Struck => span.strikethrough(),
        _ => span,
    };
    TextRun::spans(vec![span]).role(role)
}

/// The position of the picked word, when `selected` names one of `words`.
fn picked_at<K: PartialEq>(words: &[RunWord<K>], selected: Option<&Option<K>>) -> Option<usize> {
    let key = selected?.as_ref()?;
    words.iter().position(|word| word.key == *key)
}

/// The word Tab enters the run on: the picked word, else the first. Read from the bound value each pass.
fn lead<K: PartialEq>(words: &[RunWord<K>], selected: Option<&Option<K>>) -> Option<usize> {
    (!words.is_empty()).then(|| picked_at(words, selected).unwrap_or(0))
}

/// Where the words sit: each word's line and its start offset in the line, for widths in reading order.
struct Placement {
    /// Each word's width, cut to the line.
    widths: Vec<f32>,
    cells: Vec<(usize, f32)>,
    lines: usize,
    /// The extent of the longest line.
    extent: f32,
}

/// Flows words `wants` wide into lines `room` wide, `gap` apart: a word that does not fit what is left of a line
/// starts the next, and a word wider than a line is cut to the line. With widths 30, 40 and 50 in a room of 80 and a
/// gap of 5 the first two share line 0 (at 0 and 35, ending at 75) and the third starts line 1 at 0.
fn place(wants: &[f32], room: f32, gap: f32) -> Placement {
    let mut cells = Vec::with_capacity(wants.len());
    let mut widths = Vec::with_capacity(wants.len());
    let (mut line, mut at, mut extent) = (0, 0.0_f32, 0.0_f32);
    for &want in wants {
        let width = want.min(room);
        if at > 0.0 && at + width > room {
            line += 1;
            at = 0.0;
        }
        cells.push((line, at));
        widths.push(width);
        extent = extent.max(at + width);
        at += width + gap;
    }
    let lines = if wants.is_empty() { 0 } else { line + 1 };
    Placement { widths, cells, lines, extent }
}

/// The words measured and placed.
struct Arranged {
    /// The width each word wants: its text and padding.
    wants: Vec<f32>,
    /// A line's height: the tallest word.
    line: f32,
    placement: Placement,
}

impl Arranged {
    fn size(&self) -> Vec2 {
        Vec2::new(self.placement.extent, self.placement.lines as f32 * self.line)
    }

    /// The rect of word `at` in `content`, its line's start edge the reading start.
    fn rect(&self, at: usize, content: Rect, rtl: bool) -> Rect {
        let (line, start) = self.placement.cells[at];
        let top = content.min.y + line as f32 * self.line;
        let line = Rect::from_min_size(pos2(content.min.x, top), vec2(content.width(), self.line));
        Flow::new(line, rtl).span(start, self.placement.widths[at])
    }
}

/// Measures `words` in the run's role and flows them into `room`. At the least proposal the room is the widest word,
/// so each word keeps its whole text.
fn arrange<K: Value>(cx: &MeasureCx<'_, WordRun<'_, K>>, words: &[RunWord<K>], room: f32) -> Arranged {
    let role = cx.style.text.role;
    let sizes: Vec<Vec2> = words.iter().map(|word| cx.layout_at(&word_text(word, role), f32::INFINITY).size()).collect();
    let pad_total = cx.style.pad_total;
    let wants: Vec<f32> = sizes.iter().map(|size| size.x + pad_total).collect();
    let line = sizes.iter().map(|size| size.y).fold(cx.layout_at(&TextRun::new("x").role(role), f32::INFINITY).size().y, f32::max);
    let room = if cx.proposal == Proposal::Min { wants.iter().copied().fold(0.0, f32::max).min(room) } else { room };
    Arranged { placement: place(&wants, room, cx.gap()), wants, line }
}

impl<K: Value + 'static> Component for WordRun<'_, K> {
    const ROLE: Role = Role::LIST_BOX;
    const OVERLAYS: bool = false;
    const SINGLE_VALUE: bool = true;
    type Style = WordRunStyle;
    type Output = WordRunOutput<K>;

    fn style(cx: &StyleCx<'_>, _: WordRunStyleKey<K>) -> WordRunStyle {
        let (color, state) = (&cx.tokens.color.text, &cx.tokens.color.state);
        let off = cx.color(color.disabled);
        let ink = cx.color(color.normal);
        let faint = cx.color(color.faint);
        let clear = cx.color(Rgba::TRANSPARENT);
        let picked = cx.color(state.text_selection);
        let fill = Stateful::new(clear)
            .off(States::new(clear, cx.color(state.hover), cx.color(state.pressed), clear))
            .on(States::new(picked, picked, picked, picked));
        let gap = cx.tokens.space.gap.tight;
        let pad = cx.tokens.space.pad.control;
        WordRunStyle {
            metrics: Metrics::new().gap(gap),
            focus: cx.focus_look(),
            text: TextLook::new(TypeRole::Body, Stateful::new(ink).off(States::new(ink, ink, ink, off))),
            faint: TextLook::new(TypeRole::Body, Stateful::new(faint).off(States::new(faint, faint, faint, off))),
            fill,
            radius: UiTokens::corner(cx.tokens.shape.radius.sm),
            pad,
            pad_total: 2.0 * pad,
        }
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    /// The lines the words take in the column's room; the framework reuses the measure while its inputs hold.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        arrange(cx, self.words, cx.column_room().unwrap_or(f32::INFINITY)).size()
    }

    /// A focusable part per word, one roving group, entered on the picked word or the first. A word cut to its line
    /// shows its whole text as its tooltip.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let content = cx.content_rect;
        let arranged = arrange(cx, self.words, content.width());
        let rtl = cx.flow.is_rtl();
        let picked = picked_at(self.words, cx.value());
        let lead = lead(self.words, cx.value());
        for (at, word) in self.words.iter().enumerate() {
            let mut part = cx.part(Self::part(at), arranged.rect(at, content, rtl)).sense(Sense::CLICK | Sense::FOCUS).focusable();
            if arranged.placement.widths[at] < arranged.wants[at] {
                part = part.tooltip(Label::from(word.text.clone()));
            }
            if picked == Some(at) {
                part = part.flags(Flags::SELECTED);
            }
            if lead == Some(at) {
                part.selected();
            }
        }
        if !self.words.is_empty() {
            cx.roving(Roving::new(NavShape::Flat));
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, Option<K>>> {
        Some(self.selected.reborrow())
    }

    /// A click on a word, or Space or Enter on the focused one, picks it and reports it; the bound key changes only
    /// when it names another word. The activation is the pass's own input, which egui gives only to a frame's first
    /// pass, so a second pass of the frame picks nothing again.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(at) = (0..self.words.len()).find(|&at| cx.part_activated(Self::part(at)).is_some()) else { return };
        let key = self.words[at].key.clone();
        self.picked = Some(key.clone());
        if cx.value().and_then(Option::as_ref) != Some(&key) {
            cx.set(Some(key));
        }
    }

    /// Each word is an option named by its text, at its place in the run; the picked one is selected.
    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        let count = self.words.len();
        for (at, word) in self.words.iter().enumerate() {
            let node = access.node(Self::part(at), Role::LIST_BOX_OPTION);
            node.name = Name::Text(word.text.clone());
            node.position = Some(SetPosition::new(at).of(count));
        }
    }

    /// Each word's fill, then its text: a struck word in the faint colour with a line through it, an underlined word
    /// with its underline.
    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let draw = ui.draw();
        for (at, word) in self.words.iter().enumerate() {
            let part = Self::part(at);
            // `None` means `parts` declared no rect for this word, so there is nothing to paint.
            let Some(rect) = cx.part_if_declared(part).map(|result| result.rect) else { continue };
            draw.fill(&PaintShape::Rect(rect, style.radius), cx.resolve_part(part, &style.fill));
            let struck = word.state == WordState::Struck;
            let look = if struck { &style.faint } else { &style.text };
            let color = cx.resolve_part(part, &look.color);
            let run = word_text(word, look.role).color(color).elide();
            let text = rect.shrink2(vec2(style.pad, 0.0));
            draw.text(text, &run);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> WordRunOutput<K> {
        WordRunOutput { picked: self.picked.clone(), edit: cx.edit_response() }
    }
}
