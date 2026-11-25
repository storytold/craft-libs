//! `Link`: text in the accent colour, underlined unless the app's `LinkLook` says not, that reports a click.

use super::ButtonOutput;
use crate::contract::{Access, Common, Component, Cx, MeasureCx, Name, PaintUi, PartsCx, Pointer as _, Role, Styled as _};
use crate::primitives::{Span, TextLayout, TextRun, Wrap};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, FocusLook, Stateful, StyleCx, TextLook, TypeRole};
use craft_behavior::{CursorIcon, Flow, Metrics, PartKey, PointerButton, PressPhase, Proposal, Sense};
use craft_theme::States;
use craft_theme::variant::Tone;
use egui::{Pos2, Rect, Vec2};
use std::ops::Range;

/// The part a link in a sentence takes the pointer on, one per row the link's characters cover, and its `Link` node
/// below the row's run.
const LINK: PartKey = PartKey::new("link");

/// The access node of one row of a link's sentence.
const LINE: PartKey = PartKey::new("line");

/// The style of a link.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct LinkStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The link look (`StyleCx::link_look`), or the tone's colour in its place.
    #[style(text)]
    text: TextLook,
    /// A link standing alone is underlined (`LinkLook::Underlined`).
    underline: bool,
}

/// What `parts` found of a link in a sentence, for `update` and `access`.
#[derive(Clone, Debug, Default)]
struct Laid {
    /// The rows the link's characters cover: the parts `LINK.at(0)..LINK.at(rows)`.
    rows: u64,
    /// The paragraph's rows in order.
    lines: Vec<LaidLine>,
    rtl: bool,
}

/// One row of a link's sentence, as its access node reads it.
#[derive(Clone, Debug)]
struct LaidLine {
    text: String,
    /// Each character's start offset from the row's left edge, and its advance.
    positions: Vec<f32>,
    widths: Vec<f32>,
    bounds: Rect,
    /// The link's characters in this row and their rect, when it covers some.
    link: Option<(Range<usize>, Rect)>,
}

/// A link: drawn in the accent colour in a type role (body by default) and underlined, so colour is not its only cue
/// (`LinkLook::Plain` drops the underline of a link standing alone).
/// A click, Enter or Space opens the URL in the same tab; a middle click, or a click with a modifier held, opens it in a new tab. The caller can also act on `clicked()`. AccessKit reports the role `Link`, with the URL when
/// it has one; in a sentence the node is the paragraph, and the link is a `Link` node below each run it covers.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct Link {
    #[common]
    common: Common,
    /// The URL AccessKit reports for the link.
    #[prop(into)]
    url: Option<String>,
    /// The type role, to match the text around it.
    #[prop]
    role: TypeRole,
    /// How the text fits its room: cut with an ellipsis (`Wrap::Elide`, or `Wrap::ElidePath` for a path, so
    /// the name at its end stays) instead of clipping.
    #[prop]
    wrap: Wrap,
    /// The tone the link takes its colour from, in place of the accent.
    #[style_key]
    #[prop]
    tone: Option<Tone>,
    /// The text before and after the link when it sits in a sentence.
    sentence: Option<(Label, Label)>,
    laid: Laid,
}

impl Link {
    /// A link labelled `label`, in body type.
    pub fn new(label: impl Into<Label>) -> Self {
        use crate::contract::Props as _;
        Link { common: Common::new(), url: None, role: TypeRole::Body, wrap: Wrap::Line, tone: None, sentence: None, laid: Laid::default() }
            .label(label)
    }

    /// Sets the link in a wrapped paragraph `before` + the label + `after`, in the body ink. Only the link's characters
    /// take the pointer. It is one Tab stop, and click, Enter and Space activate it.
    #[must_use]
    pub fn in_sentence(mut self, before: impl Into<Label>, after: impl Into<Label>) -> Self {
        self.sentence = Some((before.into(), after.into()));
        self.wrap = Wrap::Wrap;
        self
    }

    /// The sentence's text before and after the link in the env's words.
    fn around(&self, resolve: impl Fn(&Label) -> Label) -> Option<(Label, Label)> {
        self.sentence.as_ref().map(|(before, after)| (resolve(before), resolve(after)))
    }

    /// The run: the link span alone, underlined unless `style` says not, or the paragraph `before` + link + `after`
    /// (empty stretches left out), the link always underlined.
    fn run<'a>(&self, style: &LinkStyle, label: &'a str, (before, after): (&'a str, &'a str)) -> TextRun<'a> {
        let link = Span::new(label).link();
        let link = if style.underline || self.sentence.is_some() { link.underline() } else { link };
        let spans = [Span::new(before), link, Span::new(after)];
        TextRun::spans(spans.into_iter().filter(|span| span.link || !span.text.is_empty()).collect()).role(self.role).wrap(self.wrap)
    }

    /// Whether the link was activated this pass: on the node, or on one of its parts in a sentence.
    fn activated(&self, cx: &Cx<'_, Self>) -> bool {
        cx.activation().is_some() || (0..self.laid.rows).any(|row| cx.part_activated(LINK.at(row)).is_some())
    }
}

/// The text of the stretches around a link, as `Link::run` takes them.
fn stretches(around: &Option<(Label, Label)>) -> (&str, &str) {
    around.as_ref().map_or(("", ""), |(before, after)| (before.as_str(), after.as_str()))
}

/// Where the layout is painted in the content rect: a single line is centred with its underline in a taller rect, so
/// the underline follows the text.
fn origin(content: Rect, flow: &Flow, layout: &TextLayout) -> Pos2 {
    let spare = if layout.galley.rows.len() <= 1 { (content.height() - painted_height(layout)).max(0.0) } else { 0.0 };
    let line = Flow::new(content, flow.is_rtl()).take_start(layout.size().x).0;
    Pos2::new(line.left(), content.top() + spare / 2.0)
}

impl Component for Link {
    const ROLE: Role = Role::LINK;
    const ALT_ROLES: &'static [Role] = &[Role::PARAGRAPH];
    const PARTS: bool = true;
    const OVERLAYS: bool = false;
    type Style = LinkStyle;
    type Output = ButtonOutput;

    fn style(cx: &StyleCx<'_>, key: LinkStyleKey) -> LinkStyle {
        let text = match key.tone {
            Some(tone) => {
                let color = cx.tone(tone);
                TextLook::new(TypeRole::Body, Stateful::new(color).off(States::new(color, color, color, cx.color(cx.tokens.color.text.disabled))))
            }
            // PLAN: `link_look` takes the enabled state while the style is per key, so the off state is the look's own.
            None => cx.link_look(true),
        };
        LinkStyle { metrics: Metrics::new(), focus: cx.focus_look(), text, underline: cx.env::<super::LinkLook>() == super::LinkLook::Underlined }
    }

    /// A middle click opens the link in a new tab. In a sentence the node only hovers: its parts take the click.
    fn sense(&self) -> Sense {
        if self.sentence.is_some() { Sense::HOVER } else { Sense::CLICK | Sense::FOCUS | Sense::MIDDLE_CLICK }
    }

    fn cut_first(&self) -> bool {
        matches!(self.wrap, Wrap::Elide | Wrap::ElideStart | Wrap::ElideMiddle | Wrap::ElidePath)
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let around = self.around(|label| cx.env.resolve(label));
        let run = self.run(cx.style, cx.resolved.label, stretches(&around));
        // At its least a link that elides is at its readable least.
        if cx.proposal == Proposal::Min && self.cut_first() {
            let least = cx.least(&run);
            return Vec2::new(least.width, painted_height(&least.whole));
        }
        let layout = cx.layout_at(&run, cx.room.unwrap_or(f32::INFINITY));
        Vec2::new(layout.size().x, painted_height(&layout))
    }

    /// In a sentence, one part per row the link's characters cover, each as wide as those characters on the row;
    /// only the first is a Tab stop.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        self.laid = Laid::default();
        let around = self.around(|label| cx.env.resolve(label));
        let Some((before, label)) = around.as_ref().map(|(before, _)| (before.as_str(), cx.resolved.label)) else { return };
        let run = self.run(cx.style, label, stretches(&around));
        let layout = cx.layout_at(&run, cx.content_rect.width());
        let from = before.chars().count();
        let chars = from..from + label.chars().count();
        let origin = origin(cx.content_rect, &cx.flow, &layout).to_vec2();
        let mut lines = Vec::new();
        let mut first = 0;
        for (text, row, runs) in layout.row_lines() {
            let count = text.chars().count();
            let positions: Vec<f32> = runs.iter().flat_map(|run| run.positions.iter().copied()).collect();
            let widths: Vec<f32> = runs.iter().flat_map(|run| run.widths.iter().copied()).collect();
            let (start, end) = (chars.start.max(first), chars.end.min(first + count));
            let covered = positions.iter().zip(&widths).skip(start.saturating_sub(first)).take(end.saturating_sub(start));
            let (left, right) = covered.fold((f32::INFINITY, f32::NEG_INFINITY), |(left, right), (at, width)| (left.min(*at), right.max(at + width)));
            let link = (left < right).then(|| {
                let (min, max) = (Pos2::new(row.min.x + left, row.min.y), Pos2::new(row.min.x + right, row.max.y));
                (start - first..end - first, Rect::from_min_max(min, max).translate(origin))
            });
            lines.push(LaidLine { text, positions, widths, bounds: row.translate(origin), link });
            first += count;
        }
        self.laid.rtl = cx.flow.is_rtl();
        self.laid.lines = lines;
        let rects: Vec<Rect> = self.laid.lines.iter().filter_map(|line| line.link.as_ref().map(|(_, rect)| *rect)).collect();
        self.laid.rows = rects.len() as u64;
        // PLAN: a middle click on a part has no reader in `Cx`, so a link in a sentence opens in a new tab only with a modifier.
        for (row, rect) in rects.into_iter().enumerate() {
            let part = cx.part(LINK.at(row as u64), rect).cursor(CursorIcon::PointingHand);
            if row == 0 {
                part.sense(Sense::CLICK | Sense::FOCUS).focusable();
            } else {
                part.sense(Sense::CLICK);
            }
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let middle_click = match cx.press().phase {
            PressPhase::Release { start, click: true, .. } => start.button == PointerButton::Middle,
            _ => false,
        };
        let activated = self.activated(cx);
        if let Some(url) = &self.url
            && (activated || middle_click)
        {
            let modified = cx.activation().is_some_and(|activation| !activation.mods.is_none());
            cx.open_url(url, middle_click || modified);
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        if self.sentence.is_some() {
            // The paragraph is named by its text, so the link's name is its own `Link` node's alone.
            access.role(Role::PARAGRAPH).name(Name::Text(self.laid.lines.iter().map(|line| line.text.as_str()).collect()));
        } else if let Some(url) = &self.url {
            access.url(url);
        }
        // A run node holds one line, so each row has its own, its offsets from its own left edge. The link's characters
        // on it are the `Link` node of their part, which takes focus and the click.
        let mut piece = 0;
        for (row, line) in self.laid.lines.iter().enumerate() {
            let run = LINE.at(row as u64);
            access
                .node(run, Role::TEXT_RUN)
                .text_run(line.text.clone(), line.positions.clone(), line.widths.clone(), self.laid.rtl)
                .bounds(line.bounds);
            let Some((chars, rect)) = &line.link else { continue };
            let node = access.node(LINK.at(piece), Role::LINK).under(run).bounds(*rect);
            node.name = Name::Text(line.text.chars().skip(chars.start).take(chars.len()).collect());
            if let Some(url) = &self.url {
                node.url(url.clone());
            }
            piece += 1;
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let color = cx.resolve(&cx.style.text.color);
        let draw = ui.draw();
        let around = self.around(|label| cx.env.resolve(label));
        let run = self.run(cx.style, cx.resolved.label, stretches(&around)).link_color(color.egui());
        let layout = draw.layout(&run, cx.content_rect.width());
        draw.galley(origin(cx.content_rect, &cx.flow, &layout), &layout);
    }

    fn finish(self, cx: Cx<'_, Self>) -> ButtonOutput {
        ButtonOutput::of(&cx, self.activated(&cx))
    }
}

/// The height a link paints: its text, and the underline where it reaches below the text's box.
fn painted_height(layout: &TextLayout) -> f32 {
    layout.size().y.max(layout.galley.mesh_bounds.max.y)
}
