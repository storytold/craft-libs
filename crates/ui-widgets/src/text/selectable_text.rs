//! `SelectableText`: read-only text the user can focus, select and copy.

#[cfg(feature = "testing")]
mod stories;

use super::text_part;
use crate::contract::{Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, PartsCx, Props as _, Role};
use crate::primitives::{TextOptions, TextRun};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{Insets, Metrics, PlainText, Sense};
use egui::{Rect, Vec2};

/// The style of selectable text: padding from the control metrics, no surface.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SelectableStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// Read-only text the user can focus, select and copy.
#[derive(crate::contract::Component)]
#[component(exempt(case = "state:Mixed", reason = "Read-only text has no mixed state."))]
#[component(exempt(case = "state:Invalid", reason = "Read-only text has no value a check can refuse."))]
#[component(exempt(case = "values", reason = "Read-only text has no value a drive can change."))]
pub struct SelectableText<'a> {
    #[common]
    common: Common,
    value: Bind<'a, String>,
    options: TextOptions,
}

impl SelectableText<'_> {
    /// Selectable `text` named `label`.
    pub fn new(label: impl Into<Label>, text: &str) -> Self {
        SelectableText { common: Common::new(), value: Bind::given(text.to_owned()), options: TextOptions::default() }.label(label).read_only(true)
    }

    /// The text wraps over several lines.
    #[must_use]
    pub fn multiline(mut self) -> Self {
        self.options.lines = Some(usize::MAX);
        self
    }

    /// Monospace text.
    #[must_use]
    pub fn mono(mut self) -> Self {
        self.options.mono = true;
        self
    }
}

impl Component for SelectableText<'_> {
    const ROLE: Role = Role::TEXT_INPUT;
    const INPUT: crate::contract::Input = crate::contract::Input::Text;
    type Style = SelectableStyle;
    type Output = EditResponse<String>;

    fn style(cx: &StyleCx<'_>, _: SelectableTextStyleKey) -> SelectableStyle {
        SelectableStyle { metrics: Metrics::new().padding(Insets::horizontal(cx.control().pad_x)) }
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let text = cx.value().map_or("", String::as_str);
        let run = TextRun::new(text);
        cx.layout(&if self.options.multiline() { run.wrap(crate::primitives::Wrap::Line) } else { run }).size()
    }

    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        if let Some(value) = cx.value() {
            cx.text(text_part::<String>(), cx.content_rect, &PlainText, value, &self.options);
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, String>> {
        Some(self.value.reborrow())
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<String> {
        cx.edit_response()
    }
}
