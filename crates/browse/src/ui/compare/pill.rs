//! `RenderPill`: a compare tile's render state over its picture.

use craft_behavior::{Insets, Metrics, Sense};
use craft_ui::runtime::access::Access;
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role, Styled as _};
use craft_ui::display::Spinner;
use craft_ui::primitives::TextRun;
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, Stateful, StyleCx, SurfaceLook, TextLook, TypeRole, UiTokens};
use egui::{Rect, Response, Vec2, pos2};

/// The pill: a pending render on the scrim with a spinner in `text.on_strong`, a failed one on `status.bad.solid` in
/// `text.on_status`.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct RenderPillStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    #[style(text)]
    text: TextLook,
    /// The spinner's side.
    spinner: f32,
    /// The space between the spinner and the text.
    gap: f32,
}

/// "Rendering…" with a spinner while a render is overdue, or "Render failed" with the message as its tooltip. The tile
/// it sits on carries the text as its description, so the pill has no node of its own.
#[derive(craft_ui::contract::Component)]
pub(super) struct RenderPill {
    #[common]
    common: Common,
    /// The failed look in place of the pending one.
    #[style_key]
    failed: bool,
}

impl RenderPill {
    /// The pill of a pending render reading `text`.
    pub(super) fn pending(text: impl Into<Label>) -> Self {
        RenderPill { common: Common::new(), failed: false }.label(text)
    }

    /// The pill of a failed render reading `text`, with `why` as its tooltip when there is one.
    pub(super) fn failed(text: impl Into<Label>, why: Option<String>) -> Self {
        let pill = RenderPill { common: Common::new(), failed: true }.label(text);
        match why {
            Some(why) => pill.tooltip(why),
            None => pill,
        }
    }

    /// The text's start along the pill and, while pending, the spinner's square.
    fn layout(&self, cx: &Cx<'_, Self>, text: Vec2) -> (f32, Option<Rect>) {
        let content = cx.content_rect;
        if self.failed {
            return (content.center().x - text.x / 2.0, None);
        }
        let side = cx.style.spinner;
        let rtl = cx.env.direction().is_rtl();
        let x = if rtl { content.right() - side } else { content.left() };
        let spinner = Rect::from_min_size(pos2(x, content.center().y - side / 2.0), Vec2::splat(side));
        let start = if rtl { content.left() } else { content.left() + side + cx.style.gap };
        (start, Some(spinner))
    }
}

impl Component for RenderPill {
    const ROLE: Role = Role::LABEL;
    const PARTS: bool = false;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: craft_behavior::Input = craft_behavior::Input::Display;
    const OVERLAYS: bool = false;
    type Style = RenderPillStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, key: RenderPillStyleKey) -> RenderPillStyle {
        let colors = &cx.tokens.color;
        let (fill, ink) = if key.failed { (colors.status.bad.solid, colors.text.on_status) } else { (colors.surface.scrim, colors.text.on_strong) };
        let fill = Stateful::new(cx.color(fill));
        let control = cx.control();
        let pad_y = cx.tokens.space.pad.chip;
        RenderPillStyle {
            surface: SurfaceLook::new(fill.clone()).radius(UiTokens::corner(cx.tokens.shape.radius.pill)),
            metrics: Metrics::new().padding(Insets::new(control.pad_x, control.pad_x, pad_y, pad_y)),
            text: TextLook::new(TypeRole::Caption, Stateful::new(cx.color(ink))).on(fill),
            spinner: control.icon,
            gap: cx.tokens.space.gap.inline,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let text = cx.layout(&TextRun::new(cx.resolved.label).role(cx.style.text.role)).size();
        if self.failed { text } else { Vec2::new(text.x + cx.style.gap + cx.style.spinner, text.y.max(cx.style.spinner)) }
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let run = TextRun::new(cx.resolved.label).role(cx.style.text.role).color(cx.resolve(&cx.style.text.color));
        let layout = ui.draw().layout(&run, cx.content_rect.width());
        let (start, spinner) = self.layout(cx, layout.size());
        ui.draw().galley(pos2(start, cx.content_rect.center().y - layout.size().y / 2.0), &layout);
        if let Some(square) = spinner {
            ui.child("spinner", square, Spinner::new());
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
