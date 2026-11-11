//! `Link`: text in the accent colour, always underlined, that reports a click.

use crate::contract::{Access, Common, Component, Cx, MeasureCx, PaintUi, Pointer as _, Role, Styled as _};
use crate::primitives::{StrokeWidth, TextLayout, TextRun, Wrap};
use crate::style::{ComponentStyle, FocusLook, Stateful, StyleCx, TextLook, TypeRole};
use craft_behavior::{CursorIcon, CursorPriority, Flow, Metrics, PointerButton, PressPhase, Sense};
use craft_theme::variant::Tone;
use craft_theme::{Contrast, States};
use egui::{Pos2, Response, Vec2};

/// The style of a link.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct LinkStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The text and underline colour: `accent.solid`, as PhotoCraft's links; `accent.text` at high contrast, which meets the text contrast `accent.solid` does not.
    #[style(text)]
    text: TextLook,
    /// The underline's width, resolved by `measure` and `paint` alike.
    underline: StrokeWidth,
}

/// A link: drawn in the accent colour in a type role (body by default) and underlined, so colour is not its only cue.
/// A click, Enter or Space opens the URL in the same tab and a middle click opens it in a new tab; the caller can also act on `clicked()`. AccessKit reports the role `Link`, with the URL when
/// it has one.
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
    /// How the text fits its room: cut with an ellipsis (`Wrap::Elide`, or `Wrap::ElidePath` for a path segment, so
    /// the end stays) instead of clipping.
    #[prop]
    wrap: Wrap,
    /// The tone the link takes its colour from, in place of the accent.
    #[style_key]
    #[prop]
    tone: Option<Tone>,
}

impl Link {
    /// A link labelled `label`, in body type.
    pub fn new(label: impl Into<crate::runtime::env::Label>) -> Self {
        use crate::contract::Props as _;
        Link { common: Common::new(), url: None, role: TypeRole::Body, wrap: Wrap::Line, tone: None }.label(label)
    }

    fn run<'a>(&self, label: &'a str) -> TextRun<'a> {
        TextRun::new(label).role(self.role).wrap(self.wrap)
    }
}

impl Component for Link {
    const ROLE: Role = Role::LINK;
    const PARTS: bool = false;
    const OVERLAYS: bool = false;
    type Style = LinkStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, key: LinkStyleKey) -> LinkStyle {
        let colors = &cx.tokens.color;
        let accent = if cx.contrast() == Contrast::High { colors.accent.text } else { colors.accent.solid };
        let color = key.tone.map_or_else(|| cx.color(accent), |tone| cx.tone(tone));
        LinkStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            underline: StrokeWidth::Thin,
            text: TextLook::new(TypeRole::Body, Stateful::new(color).off(States::new(color, color, color, cx.color(colors.text.disabled)))),
        }
    }

    /// A middle click opens the link in a new tab.
    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::MIDDLE_CLICK
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let layout = cx.layout(&self.run(cx.resolved.label));
        let underline = cx.style.underline.points(cx.tokens, cx.ui.ctx().pixels_per_point());
        Vec2::new(layout.size().x, reach(&layout, underline))
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let middle_click = match cx.press().phase {
            PressPhase::Release { start, click: true, .. } => start.button == PointerButton::Middle,
            _ => false,
        };
        if let Some(url) = &self.url
            && (cx.activation().is_some() || middle_click)
        {
            cx.open_url(url, middle_click);
        }
        if cx.hit() {
            cx.cursor(CursorIcon::PointingHand, CursorPriority::Part);
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        if let Some(url) = &self.url {
            access.url(url.as_str());
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let color = cx.resolve(&cx.style.text.color);
        let draw = ui.draw();
        let run = self.run(cx.resolved.label).color(color);
        let layout = draw.layout(&run, cx.content_rect.width());
        // A single line is centred with its underline in a taller rect, so the underline follows the text.
        let underline = draw.width(cx.style.underline);
        let total = reach(&layout, underline);
        let spare = if layout.galley.rows.len() <= 1 { (cx.content_rect.height() - total).max(0.0) } else { 0.0 };
        let top = cx.content_rect.top() + spare / 2.0;
        let line = Flow::new(cx.content_rect, cx.flow.is_rtl()).take_start(layout.size().x).0;
        draw.galley(Pos2::new(line.left(), top), &layout);
        // The stroke's top edge sits on the line box's bottom, so the whole stroke lies below every descender.
        let y = top + line_bottom(&layout).unwrap_or(layout.size().y) + underline / 2.0;
        draw.line_snapped(cx.style.underline, color.egui(), Pos2::new(line.left(), y), Pos2::new(line.right(), y));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// The bottom of the first line's glyph box, from the top of the layout: the baseline plus the face's descent and
/// the line's extra leading below the glyphs.
fn line_bottom(layout: &TextLayout) -> Option<f32> {
    layout.galley.rows.first().and_then(|placed| placed.row.glyphs.first().map(|glyph| placed.pos.y + glyph.logical_rect().bottom()))
}

/// The height a link takes: its text, and the underline below the first line's box when that reaches below the text.
fn reach(layout: &TextLayout, underline: f32) -> f32 {
    line_bottom(layout).map_or(layout.size().y, |bottom| layout.size().y.max(bottom + underline))
}
