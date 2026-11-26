//! `Text`: static text in a type role, a shade and optionally a tone.

use crate::contract::{Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role, Styled as _};
use crate::primitives::{DocRun, DocText, Span, TextRun, Wrap};
use crate::runtime::access::{Access, Numeric};
use crate::runtime::env::Label;
use std::borrow::Cow;

use crate::style::{ComponentStyle, StyleCx, TextLook, TypeRole};
use craft_behavior::{Metrics, Proposal, Sense};
use craft_theme::{Stateful, States, variant::Tone};
use egui::{Response, Vec2};

#[cfg(feature = "suite")]
pub(crate) mod stories;

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
    /// One line, cut in the middle, the end keeping a quarter of what stays: a file name.
    Middle,
    /// One line, cut at the start so the end stays.
    Start,
    /// One line, a path that drops leading folders first so the name at its end stays.
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
    /// The interface spans that replace the label's text, in the type role, shade and tone of the text; empty for plain text.
    styled: Vec<Span<'static>>,
    /// Sets the text's ideal and least width to those of this text, so a readout keeps its width while its value changes.
    #[prop(into)]
    sample: Option<Label>,
    /// Whether every span is italic, a value set apart from the words around it.
    italic: bool,
    /// Gives the text an accessible numeric value, `value`, so tests and assistive technology read a position or a count
    /// without parsing the translated text.
    #[prop(quantity = "an accessible numeric value: a position or a count")]
    numeric: Option<f64>,
}

impl Text {
    /// `text`, which is also its accessible name, in body type and the normal shade.
    pub fn new(text: impl Into<Label>) -> Self {
        Text {
            common: Common::new(),
            shade: Shade::Normal,
            tone: None,
            role: TypeRole::Body,
            elide: Elide::Off,
            spans: Vec::new(),
            styled: Vec::new(),
            sample: None,
            italic: false,
            numeric: None,
        }
        .label(text)
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

    /// Interface text from styled `spans`; the joined text is its accessible name. The spans take the type role, shade
    /// and tone of the text, as plain text does; each span adds its own emphasis (italic, strong, a flag underline).
    pub fn styled(spans: Vec<Span<'static>>) -> Self {
        let label: String = spans.iter().map(|span| span.text.as_ref()).collect();
        let mut text = Text::new(label);
        text.styled = spans;
        text
    }

    /// Sets the text in italics: a value set apart from the words around it. The resolved label is what is set, so a
    /// catalogue word shows in its own language.
    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    fn doc(&self) -> DocText<'_> {
        let doc = DocText::new(&self.spans);
        if self.elide == Elide::Off { doc } else { doc.elide() }
    }

    /// The spans of the text: its styled spans when it has them, else `label` as one, each italic under `italic`.
    fn spans_of<'a>(&self, label: &'a str) -> Vec<Span<'a>> {
        let mut spans: Vec<Span<'a>> = if self.styled.is_empty() { vec![Span::new(label)] } else { self.styled.clone() };
        if self.italic {
            spans.iter_mut().for_each(|span| span.italic = true);
        }
        spans
    }

    /// The run of the whole text.
    fn run<'a>(&self, role: TypeRole, label: &'a str) -> TextRun<'a> {
        let run = if self.styled.is_empty() && !self.italic { TextRun::new(label) } else { TextRun::spans(self.spans_of(label)) };
        self.shaped(run, role)
    }

    /// One run per word of the text, each in the style of the span it is in, to find the narrowest the text can wrap to.
    fn word_runs<'a>(&self, role: TypeRole, label: &'a str) -> Vec<TextRun<'a>> {
        let mut runs = Vec::new();
        for span in self.spans_of(label) {
            for word in span.text.split_whitespace() {
                let mut one = span.clone();
                one.text = Cow::Owned(word.to_owned());
                runs.push(self.shaped(TextRun::spans(vec![one]), role));
            }
        }
        runs
    }

    fn shaped<'a>(&self, run: TextRun<'a>, role: TypeRole) -> TextRun<'a> {
        let wrap = match self.elide {
            Elide::Off => Wrap::Wrap,
            Elide::End => Wrap::Elide,
            Elide::Middle => Wrap::ElideMiddle,
            Elide::Start => Wrap::ElideStart,
            Elide::Path => Wrap::ElidePath,
        };
        run.role(role).wrap(wrap)
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
        let role = cx.style.text.role;
        let sample = self.sample.as_ref().map(|sample| cx.layout_at(&self.run(role, sample.as_str()), f32::INFINITY).size().x);
        if let Some(width) = sample
            && matches!(cx.proposal, Proposal::Ideal | Proposal::Min)
        {
            let height = cx.layout_at(&self.run(role, cx.resolved.label), f32::INFINITY).size().y;
            return Vec2::new(width, height);
        }
        // A line breaks between words, never inside one: a wrapped text is never narrower than its widest word, up to
        // an enclosing reading-width cap.
        let widest = || {
            self.word_runs(role, cx.resolved.label)
                .iter()
                .map(|word| cx.layout_at(word, f32::INFINITY).size().x)
                .fold(0.0, f32::max)
                .min(cx.wrap_limit().unwrap_or(f32::INFINITY))
        };
        if cx.proposal == Proposal::Min && self.elide == Elide::Off {
            let widest = widest();
            return Vec2::new(widest, cx.layout_at(&self.run(role, cx.resolved.label), widest).size().y);
        }
        // At its least an elided line is at its readable least.
        if cx.proposal == Proposal::Min {
            let run = self.run(role, cx.resolved.label);
            return cx.least(&run).size();
        }
        // A wrapping text is never narrower than its sample.
        let band = wrap.unwrap_or(f32::INFINITY).max(if self.elide == Elide::Off { sample.unwrap_or(0.0) } else { 0.0 });
        let mut layout = cx.layout_at(&self.run(role, cx.resolved.label), band);
        // In a row the room is what the earlier siblings left, which can be less than a word; the row's shrink step
        // then makes room for the word.
        if self.elide == Elide::Off && layout.galley.rows.len() > 1 {
            let widest = widest();
            if widest > band {
                layout = cx.layout_at(&self.run(role, cx.resolved.label), widest);
            }
        }
        // A cut line takes the whole room: its width must not follow where the last glyph happened to fit.
        match cx.room {
            Some(room) if layout.elided => Vec2::new(room, layout.size().y),
            _ => layout.size(),
        }
    }

    fn cut_first(&self) -> bool {
        self.elide != Elide::Off
    }

    fn sense(&self) -> Sense {
        super::display_sense(&self.common, self.elide != Elide::Off)
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if let Some(value) = self.numeric {
            access.numeric(Numeric::new(value, value..=value).text(cx.resolved.label));
        }
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
