//! `Link`: text in the accent colour, always underlined, that reports a click.

use crate::contract::{Access, Common, Component, Cx, MeasureCx, PaintUi, Pointer as _, Role, Styled as _};
use crate::primitives::{Span, TextLayout, TextRun, Wrap};
use crate::style::{ComponentStyle, FocusLook, Stateful, StyleCx, TextLook, TypeRole};
use craft_behavior::{Flow, Metrics, PointerButton, PressPhase, Sense};
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
}

/// A link: drawn in the accent colour in a type role (body by default) and underlined, so colour is not its only cue.
/// A click, Enter or Space opens the URL in the same tab; a middle click, or a click with a modifier held, opens it in a new tab. The caller can also act on `clicked()`. AccessKit reports the role `Link`, with the URL when
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
        TextRun::spans(vec![Span::new(label).underline()]).role(self.role).wrap(self.wrap)
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
            text: TextLook::new(TypeRole::Body, Stateful::new(color).off(States::new(color, color, color, cx.color(colors.text.disabled)))),
        }
    }

    /// A middle click opens the link in a new tab.
    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::MIDDLE_CLICK
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let layout = cx.layout(&self.run(cx.resolved.label));
        Vec2::new(layout.size().x, painted_height(&layout))
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let middle_click = match cx.press().phase {
            PressPhase::Release { start, click: true, .. } => start.button == PointerButton::Middle,
            _ => false,
        };
        let activation = cx.activation();
        if let Some(url) = &self.url
            && (activation.is_some() || middle_click)
        {
            let modified = activation.is_some_and(|activation| !activation.mods.is_none());
            cx.open_url(url, middle_click || modified);
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
        let spare = if layout.galley.rows.len() <= 1 { (cx.content_rect.height() - painted_height(&layout)).max(0.0) } else { 0.0 };
        let top = cx.content_rect.top() + spare / 2.0;
        let line = Flow::new(cx.content_rect, cx.flow.is_rtl()).take_start(layout.size().x).0;
        draw.galley(Pos2::new(line.left(), top), &layout);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// The height a link paints: its text, and the underline where it reaches below the text's box.
fn painted_height(layout: &TextLayout) -> f32 {
    layout.size().y.max(layout.galley.mesh_bounds.max.y)
}
