//! `Text`: static text in a type role, a shade and optionally a tone.

use crate::contract::{Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role, Styled as _};
use crate::primitives::{DocRun, DocText, TextRun, Wrap};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx, TextLook, TypeRole};
use craft_behavior::{Metrics, Proposal, Sense};
use craft_theme::{Stateful, States, variant::Tone};
use egui::{Response, Vec2};

#[cfg(feature = "testing")]
mod stories;

/// How strongly the text reads against its surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Shade {
    /// The normal text colour.
    #[default]
    Normal,
    /// Secondary text.
    Muted,
    /// Captions and hints.
    Faint,
}

/// Where text that does not fit is cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Elide {
    /// The text wraps at the room's width.
    #[default]
    Off,
    /// One line, cut with an ellipsis at the end.
    End,
    /// One line, cut in the middle so the end of a path stays.
    Path,
}

/// The style of static text.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TextStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The text in its shade.
    #[style(text)]
    text: TextLook,
}

/// Text in a type role (body by default) and a shade (normal by default), or in the colour of a tone. It wraps at the room's width unless
/// `elide` keeps it on one line; a line that is cut shows whole in a tooltip. It takes no focus and cannot be
/// selected; `SelectableText` is the focusable form.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct Text {
    #[common]
    common: Common,
    /// How strongly the text reads.
    #[style_key]
    #[prop]
    shade: Shade,
    /// The tone the text takes its colour from, in place of the shade's.
    #[style_key]
    #[prop]
    tone: Option<Tone>,
    /// The type role.
    #[style_key]
    #[prop]
    role: TypeRole,
    /// Where text that does not fit is cut.
    #[prop]
    elide: Elide,
    /// The styled spans that replace the label's text; empty for plain text.
    spans: Vec<DocRun<'static>>,
}

impl Text {
    /// `text`, which is also its accessible name, in body type and the normal shade.
    pub fn new(text: impl Into<Label>) -> Self {
        Text { common: Common::new(), shade: Shade::Normal, tone: None, role: TypeRole::Body, elide: Elide::Off, spans: Vec::new() }.label(text)
    }

    /// Text of `spans`, each with its own font, size, colour and lines; the joined text is its accessible name. It wraps
    /// at the room's width, or stays on one row cut at the end under any `elide` but `Off`. The type role, shade and
    /// tone do not apply: every span names its own.
    pub fn spans(spans: &[DocRun<'_>]) -> Self {
        let label: String = spans.iter().map(|span| span.text.as_ref()).collect();
        let mut text = Text::new(label);
        text.spans = spans.iter().cloned().map(DocRun::into_owned).collect();
        text
    }

    fn doc(&self) -> DocText<'_> {
        let doc = DocText::new(&self.spans);
        if self.elide == Elide::Off { doc } else { doc.elide() }
    }

    fn run<'a>(&self, role: TypeRole, label: &'a str) -> TextRun<'a> {
        let wrap = match self.elide {
            Elide::Off => Wrap::Wrap,
            Elide::End => Wrap::Elide,
            Elide::Path => Wrap::ElidePath,
        };
        TextRun::new(label).role(role).wrap(wrap)
    }
}

impl Component for Text {
    const ROLE: Role = Role::LABEL;
    const PARTS: bool = false;
    const INPUT: craft_behavior::Input = craft_behavior::Input::Display;
    const OVERLAYS: bool = false;
    type Style = TextStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, key: TextStyleKey) -> TextStyle {
        let text = &cx.tokens.color.text;
        let color = match key.tone {
            Some(tone) => cx.tone(tone),
            None => cx.color(match key.shade {
                Shade::Normal => text.normal,
                Shade::Muted => text.muted,
                Shade::Faint => text.faint,
            }),
        };
        let off = States::new(color, color, color, cx.color(text.disabled));
        TextStyle { metrics: Metrics::new(), text: TextLook::new(key.role, Stateful::new(color).off(off)) }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let wrap = cx.column_room();
        if !self.spans.is_empty() {
            let doc = DocText { wrap: Some(wrap.unwrap_or(f32::INFINITY)), ..self.doc() };
            let layout = doc.layout(cx.ui.ctx(), cx.tokens, cx.env.text_direction());
            return match cx.room {
                Some(room) if layout.elided => Vec2::new(room, layout.size().y),
                _ => layout.size(),
            };
        }
        // At its least an elided line is the ellipsis.
        let band = if cx.proposal == Proposal::Min && self.elide != Elide::Off { 0.0 } else { wrap.unwrap_or(f32::INFINITY) };
        let layout = cx.layout_at(&self.run(cx.style.text.role, cx.resolved.label), band);
        // A cut line takes the whole room: its width must not follow where the last glyph happened to fit.
        match cx.room {
            Some(room) if layout.elided => Vec2::new(room, layout.size().y),
            _ => layout.size(),
        }
    }

    fn sense(&self) -> Sense {
        super::display_sense(&self.common, self.elide != Elide::Off)
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if !self.spans.is_empty() {
            ui.draw().doc(cx.content_rect, &self.doc());
            return;
        }
        ui.draw().text(cx.content_rect, &self.run(cx.style.text.role, cx.resolved.label).color(cx.resolve(&cx.style.text.color)));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
