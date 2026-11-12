//! `TimecodeReadout`: one row of the Timecode panel, a time over its caption.

use craft_behavior::{Axis, Sense};
use craft_theme::{Stateful, States};
use craft_ui::contract::{Common, Component, Cx, Input, MeasureCx, PaintUi, Props as _, Role, Styled as _};
use craft_ui::primitives::{Span, TextRun};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{ActivateKeys, KeyUse};
use craft_ui::style::{ComponentStyle, FocusLook, Metrics, StyleColor, StyleCx, TextLook, TypeRole};
use egui::{Pos2, Response, Vec2};

/// The look of a readout: the time in the timecode type, the caption under it.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(crate) struct TimecodeReadoutStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The time: mono with tabular figures, in the accent ink on the panel's first row.
    #[style(text)]
    value: TextLook,
    /// The caption: muted caption type.
    #[style(text)]
    caption: TextLook,
    /// Between the time and the caption.
    gap: f32,
}

/// A time centred over its caption ("Current Time \u{b7} Program"), read-only: no hover, press or selection look. It is
/// one Tab stop and a click focuses it, so the panel's context menu opens from it by a secondary click, the Menu key or
/// Shift+F10. AccessKit: a label named by the spoken text (the caption and the time).
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(sample = TimecodeReadout::sample)]
pub(crate) struct TimecodeReadout<'a> {
    #[common]
    common: Common,
    value: &'a str,
    caption: &'a str,
    /// The panel's first row: the time in the accent ink.
    #[style_key]
    #[prop]
    accent: bool,
}

impl<'a> TimecodeReadout<'a> {
    /// `value` over `caption`, named `spoken` for a screen reader.
    pub(crate) fn new(spoken: impl Into<Label>, value: &'a str, caption: &'a str) -> Self {
        TimecodeReadout { common: Common::new(), value, caption, accent: false }.label(spoken)
    }

    fn value_run(&self, role: TypeRole) -> TextRun<'a> {
        TextRun::spans(vec![Span::new(self.value).tabular()]).role(role)
    }

    fn caption_run(&self, role: TypeRole) -> TextRun<'a> {
        TextRun::new(self.caption).role(role)
    }
}

impl TimecodeReadout<'static> {
    /// The first row of a panel, for the suite.
    #[cfg(feature = "screen-testing")]
    fn sample() -> Self {
        TimecodeReadout::new("Current Time \u{b7} Program: 00:01:02:03", "00:01:02:03", "Current Time \u{b7} Program").accent(true)
    }
}

impl Component for TimecodeReadout<'_> {
    const ROLE: Role = Role::LABEL;
    const PARTS: bool = false;
    /// A focusable click target whose click does nothing: Space and Enter do not activate it.
    const INPUT: Input = Input::Pointer;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_activate(ActivateKeys::Neither);
    const OVERLAYS: bool = false;
    type Style = TimecodeReadoutStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, key: TimecodeReadoutStyleKey) -> TimecodeReadoutStyle {
        let colors = &cx.tokens.color;
        let ink = |color: StyleColor| Stateful::new(color).off(States::new(color, color, color, cx.color(colors.text.disabled)));
        let value = cx.color(if key.accent { colors.accent.text } else { colors.text.normal });
        TimecodeReadoutStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            value: TextLook::new(TypeRole::Mono, ink(value)),
            caption: TextLook::new(TypeRole::Caption, ink(cx.color(colors.text.muted))),
            gap: cx.tokens.space.gap.stack,
        }
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::CONTEXT_CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    /// The readout spans its room, so the time and the caption centre in the panel.
    fn fills(&self) -> Option<Axis> {
        Some(Axis::X)
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let value = cx.layout(&self.value_run(cx.style.value.role)).size();
        let caption = cx.layout(&self.caption_run(cx.style.caption.role)).size();
        Vec2::new(value.x.max(caption.x), value.y + cx.style.gap + caption.y)
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let rect = cx.content_rect;
        let draw = ui.draw();
        let centred = |top: f32, size: Vec2| Pos2::new(rect.left() + (rect.width() - size.x).max(0.0) / 2.0, top);
        let value = draw.layout(&self.value_run(cx.style.value.role).color(cx.resolve(&cx.style.value.color)), rect.width());
        let caption = draw.layout(&self.caption_run(cx.style.caption.role).color(cx.resolve(&cx.style.caption.color)), rect.width());
        let below = rect.top() + value.size().y + cx.style.gap;
        draw.galley(centred(rect.top(), value.size()), &value);
        draw.galley(centred(below, caption.size()), &caption);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
