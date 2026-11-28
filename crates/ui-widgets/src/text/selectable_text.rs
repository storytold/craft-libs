//! `SelectableText`: read-only text the user can focus, select and copy.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use crate::contract::{Access, Common, Component, Cx, MeasureCx, PaintUi, PartsCx, Props as _, Role, Styled as _};
use crate::primitives::{Span, TextRun, Wrap};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx, TextLook, TypeRole};
use craft_behavior::{Insets, Metrics, PartKey, Sense};
use craft_theme::{Stateful, States};
use egui::{Rect, Response, Vec2};

/// The selection part that shows the text.
const TEXT: PartKey = PartKey::new("text");

/// The style of selectable text: padding from the control metrics when it is inset like a field's text, no surface,
/// the body role in the normal text colour.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SelectableStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(text)]
    text: TextLook,
}

/// Read-only text the user focuses with a press to select and copy, which Tab passes over. Its node is a label named
/// by `label`, with the text as its value.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct SelectableText<'a> {
    #[common]
    common: Common,
    spans: Vec<Span<'a>>,
    wrap: Wrap,
    mono: bool,
    /// The text is inset by the control padding, as the text of a field.
    #[style_key]
    #[prop(flag)]
    field_inset: bool,
}

impl<'a> SelectableText<'a> {
    /// Selectable `text` named `label`: one plain span, not copied.
    pub fn new(label: impl Into<Label>, text: &'a str) -> Self {
        Self::spans(label, vec![Span::new(text)])
    }

    /// Selectable text of styled `spans` named `label`, selected, copied and named as one text.
    pub fn spans(label: impl Into<Label>, spans: Vec<Span<'a>>) -> Self {
        SelectableText { common: Common::new(), spans, wrap: Wrap::Line, mono: false, field_inset: false }.label(label)
    }

    /// The text wraps over several lines.
    #[must_use]
    pub fn multiline(mut self) -> Self {
        self.wrap = Wrap::Wrap;
        self
    }

    /// Monospace text.
    #[must_use]
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    fn run(&self, role: TypeRole) -> TextRun<'_> {
        let spans = if self.mono { self.spans.iter().cloned().map(Span::mono).collect() } else { self.spans.clone() };
        TextRun::spans(spans).role(role).wrap(self.wrap)
    }
}

impl Component for SelectableText<'_> {
    const ROLE: Role = Role::LABEL;
    const INPUT: crate::contract::Input = crate::contract::Input::Display;
    const OVERLAYS: bool = false;
    type Style = SelectableStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, key: SelectableTextStyleKey) -> SelectableStyle {
        let text = &cx.tokens.color.text;
        let color = cx.color(text.normal);
        let off = States::new(color, color, color, cx.color(text.disabled));
        let metrics = if key.field_inset { Metrics::new().padding(Insets::horizontal(cx.control().pad_x)) } else { Metrics::new() };
        SelectableStyle { metrics, text: TextLook::new(TypeRole::Body, Stateful::new(color).off(off)) }
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        cx.layout(&self.run(cx.style.text.role)).size()
    }

    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.selectable_text(TEXT, &self.run(cx.style.text.role));
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.value(self.run(TypeRole::Body).full_text());
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        ui.selectable_text(TEXT, cx.content_rect, &self.run(cx.style.text.role).color(cx.resolve(&cx.style.text.color)));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
