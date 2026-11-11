//! The empty-state text of a collection: centred muted text that is its own label node.

use crate::contract::{Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role, Show as _};
use crate::fonts::stack_facts;
use crate::primitives::TextEnv;
use crate::primitives::{HAlign, Span, TextRun, Wrap};
use crate::runtime::env::{Env, Label};
use crate::style::{ComponentStyle, StyleCx, TypeRole, ink_color};
use craft_behavior::{Metrics, Sense};
use egui::{Id, Rect, Response, Ui, Vec2};

/// The style of the empty-state text.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
pub(super) struct EmptyTextStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The text a collection with no rows shows (`CollectionProps::empty`).
#[derive(crate::contract::Component)]
#[component(unregistered)]
pub(super) struct EmptyText {
    #[common]
    common: Common,
}

impl EmptyText {
    pub(super) fn new(text: Label) -> Self {
        EmptyText { common: Common::new() }.label(text)
    }

    fn run<'t>(text: &'t str) -> TextRun<'t> {
        TextRun::spans(vec![Span::new(text).muted()]).role(TypeRole::Label).wrap(Wrap::Wrap).align(HAlign::Center)
    }
}

/// The height the empty text of `props.empty` takes at `width`.
pub(in crate::contract) fn height(ui: &Ui, text: &Label, width: f32) -> f32 {
    let env = Env::of(ui);
    let text = env.view().resolve(text);
    let layout_env =
        TextEnv { tokens: &env.tokens, direction: env.text_direction, ink: ink_color(&env.tokens, env.ink), fonts: stack_facts(ui.ctx()) };
    EmptyText::run(text.as_str()).layout(ui.ctx(), &layout_env, width).size().y
}

/// Shows the empty text of `props.empty` in `rect`.
pub(in crate::contract) fn show(ui: &mut Ui, id: Id, text: Label, rect: Rect) {
    EmptyText::new(text).id(id).show_at(ui, rect);
}

impl Component for EmptyText {
    const ROLE: Role = Role::LABEL;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = EmptyTextStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: EmptyTextStyleKey) -> EmptyTextStyle {
        EmptyTextStyle { metrics: Metrics::new() }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        cx.layout(&Self::run(cx.resolved.label)).size()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        ui.draw().text(cx.rect, &Self::run(cx.core.resolved.label));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.core.response.clone()
    }
}
