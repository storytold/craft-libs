//! The empty-state text of a collection: faint body text at the reading start, a stack gap above and
//! below, that is its own label node: the inline empty state of the source apps' panels and lists.

use crate::contract::{Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role, Show as _, Styled as _};
use crate::fonts::stack_facts;
use crate::primitives::TextEnv;
use crate::primitives::{TextRun, Wrap};
use crate::runtime::env::{Env, Label};
use crate::style::{ComponentStyle, Insets, StyleCx, TextLook, TypeRole, ink_color};
use craft_behavior::{Metrics, Sense};
use craft_theme::Stateful;
use egui::{Id, Rect, Response, Ui, Vec2};

/// The style of the empty-state text.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
pub(super) struct EmptyTextStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(text)]
    text: TextLook,
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
        TextRun::new(text).role(ROLE).wrap(Wrap::Wrap)
    }

    /// The padding around the text: a stack gap above and below, and the rows' side padding so the text starts where
    /// their labels do.
    fn padding(tokens: &crate::style::UiTokens, pad_x: f32) -> Insets {
        Insets::new(pad_x, pad_x, tokens.space.gap.stack, tokens.space.gap.stack)
    }
}

/// The empty text's type role.
const ROLE: TypeRole = TypeRole::Body;

/// The height the empty text of `props.empty` takes at `width`, padding included.
pub(in crate::contract) fn height(ui: &Ui, text: &Label, width: f32) -> f32 {
    let env = Env::of(ui);
    let text = env.view().resolve(text);
    let layout_env =
        TextEnv { tokens: &env.tokens, direction: env.text_direction, ink: ink_color(&env.tokens, env.ink), fonts: stack_facts(ui.ctx()) };
    let pad = EmptyText::padding(&env.tokens, env.control(env.size).pad_x);
    EmptyText::run(text.as_str()).layout(ui.ctx(), &layout_env, (width - pad.start - pad.end).max(0.0)).size().y + pad.top + pad.bottom
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

    fn style(cx: &StyleCx<'_>, _: EmptyTextStyleKey) -> EmptyTextStyle {
        EmptyTextStyle {
            metrics: Metrics::new().padding(EmptyText::padding(cx.tokens, cx.control().pad_x)),
            text: TextLook::new(ROLE, Stateful::new(cx.color(cx.tokens.color.text.faint))),
        }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        cx.layout(&Self::run(cx.resolved.label)).size()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        ui.draw().text(cx.content_rect, &Self::run(cx.core.resolved.label).color(cx.resolve(&cx.style.text.color)));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.core.response.clone()
    }
}
