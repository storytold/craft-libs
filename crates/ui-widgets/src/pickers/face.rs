//! `SelectFace`: the closed box of a dropdown picker, the text and a trailing chevron.

use crate::contract::{Common, Component, Cx, MeasureCx, PaintUi, Pointer as _, PopupHandle, Props as _, Role, Styled as _};
use crate::list::{RowThumb, ThumbAspect, ThumbPaint};
use crate::primitives::{ContentPainterCx, IconSpec, TextRun};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, IconLook, Ink, StyleCx, TextLook, TypeRole};
use crate::text::{Adornment, FieldStyle, place, reserve};
use craft_behavior::{Flow, Metrics, Proposal, Sense};
use craft_theme::{Stateful, States};
use egui::{Rect, Response, Vec2};

/// The style of a select face: the field's metrics, the text, the faint placeholder, the thumbnail's height and the
/// chevron. The host owns the
/// surface (the field well), so a face paints only its content.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SelectFaceStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The picked option's text.
    #[style(text)]
    text: TextLook,
    /// The text shown when nothing is picked.
    #[style(text)]
    placeholder: TextLook,
    /// The chevron.
    #[style(icon)]
    chevron: IconLook,
    /// The height of the thumbnail: `size.thumb.sm`; its width follows the aspect.
    thumb: f32,
    /// The field look the chevron is placed by.
    field: FieldStyle,
}

/// The closed box of a picker: `text` elided to the room left of a trailing chevron. The box is as wide as the text
/// and the chevron need, at least a field wide. A host shows it as the look of one of its parts with
/// `ui.face(part, SelectFace::new(text))` over the field well it paints itself, and gives the box the width of its
/// widest text so the width does not change with the pick. A thumbnail goes before the text, or alone.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(sample = SelectFace::sample)]
pub struct SelectFace<'a> {
    #[common]
    common: Common,
    /// The text is a placeholder, not a pick: it shows in the faint ink.
    #[prop(flag)]
    placeholder: bool,
    thumb: Option<(ThumbAspect, ThumbPaint<'a>)>,
    /// The picked entry's icon, drawn before the text in the chevron's ink.
    #[prop]
    leading: Option<IconSpec<'static>>,
    /// Whether the text shows beside the thumbnail; `false` shows the thumbnail alone. The text is the face's
    /// accessible name either way.
    #[prop]
    name: bool,
}

impl<'a> SelectFace<'a> {
    /// A face showing `text`, which is also its name.
    pub fn new(text: impl Into<Label>) -> Self {
        SelectFace { common: Common::new(), placeholder: false, thumb: None, leading: None, name: true }.label(text)
    }

    /// A picture `aspect` times as wide as high before the text; `paint` draws into the picture's rect through the
    /// content painter, which is clipped to it. The picture's height comes from the style.
    #[must_use]
    pub fn thumb(mut self, aspect: ThumbAspect, paint: impl FnOnce(&ContentPainterCx, Rect) + 'a) -> Self {
        self.thumb = Some((aspect, Box::new(paint)));
        self
    }

    #[cfg(feature = "testing")]
    fn sample() -> SelectFace<'static> {
        SelectFace::new("Multiply")
    }

    /// The width of the thumbnail and its gap to the text, or nothing without a thumbnail.
    fn lead(&self, style: &SelectFaceStyle) -> f32 {
        self.thumb.as_ref().map_or(0.0, |(aspect, _)| aspect.of(style.thumb) + if self.name { style.field.metrics().gap } else { 0.0 })
    }

    fn leading_glyphs(&self) -> Vec<Adornment> {
        self.leading.iter().map(|icon| Adornment::glyph(icon.clone())).collect()
    }

    fn chevron() -> Adornment {
        Adornment::glyph(crate::controls::disclosure::DisclosureGlyph::dropdown())
    }
}

impl Component for SelectFace<'_> {
    const ROLE: Role = Role::LABEL;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const INPUT: craft_behavior::Input = craft_behavior::Input::Display;
    const OVERLAYS: bool = false;
    type Style = SelectFaceStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: SelectFaceStyleKey) -> SelectFaceStyle {
        let field = FieldStyle::at(cx);
        let off = cx.color(cx.tokens.color.text.disabled);
        let (ink, muted) = (cx.ink(Ink::OnPanel), cx.color(cx.tokens.color.text.faint));
        let ink = Stateful::new(ink).off(States::new(ink, ink, ink, off));
        SelectFaceStyle {
            metrics: field.metrics(),
            text: TextLook::new(TypeRole::Body, cx.host_ink(ink.clone())),
            placeholder: TextLook::new(TypeRole::Body, cx.host_muted_ink(Stateful::new(muted).off(States::new(muted, muted, muted, off)))),
            chevron: IconLook::new(field.glyph().side, cx.host_ink(ink)),
            thumb: cx.tokens.size.thumb.sm,
            field,
        }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        // At its least the face elides its name to nothing.
        let text = match (self.name, cx.proposal) {
            (true, Proposal::Min) => Vec2::new(0.0, cx.layout(&TextRun::new(cx.resolved.label).role(cx.style.text.role)).size().y),
            (true, _) => cx.layout(&TextRun::new(cx.resolved.label).role(cx.style.text.role)).size(),
            (false, _) => Vec2::ZERO,
        };
        let picture = if self.thumb.is_some() { cx.style.thumb } else { 0.0 };
        Vec2::new(self.lead(cx.style) + text.x + reserve(&cx.style.field, &self.leading_glyphs(), &[Self::chevron()]), text.y.max(picture))
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let chevron = Self::chevron();
        let rtl = cx.flow.is_rtl();
        let mut flow = Flow::new(cx.content_rect, rtl);
        if let Some((aspect, paint)) = self.thumb.take() {
            let (cell, rest) = flow.take_start(aspect.of(cx.style.thumb));
            let picture = Rect::from_center_size(cell.center(), Vec2::new(cell.width(), cx.style.thumb));
            let _ = ui.child("thumb", picture, RowThumb::new(paint));
            flow = if self.name { rest.take_start(cx.style.field.metrics().gap).1 } else { rest };
        }
        let leading = self.leading_glyphs();
        let placed = place(flow.rect(), &cx.style.field, &leading, std::slice::from_ref(&chevron), rtl);
        if let (Some(rect), Some(icon)) = (placed.leading.first(), leading.first()) {
            icon.paint(ui, *rect, cx.resolve(&cx.style.chevron.color));
        }
        if self.name {
            let look = if self.placeholder { &cx.style.placeholder } else { &cx.style.text };
            let run = TextRun::new(cx.resolved.label).role(look.role).elide().color(cx.resolve(&look.color));
            ui.draw().text(placed.text, &run);
        }
        if let Some(rect) = placed.trailing.first() {
            chevron.paint(ui, *rect, cx.resolve(&cx.style.chevron.color));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

#[cfg(feature = "testing")]
mod stories;

/// Opens the popup of a picker whose face part toggles it, from the keyboard: Enter and Space (an activation by key)
/// toggle it. A click on the face reaches the popup through the part's `toggles`, so the pointer is not read here.
///
/// A custom picker face calls it from `update` with the popup it declares, so it opens and closes from the keyboard
/// as the built-in faces do. Escape and a press outside close the popup through its dismissal rule.
pub fn toggle_on_key<C: Component>(cx: &mut Cx<'_, C>, popup: PopupHandle) {
    if cx.activation().is_some_and(|activation| activation.by_key) {
        if popup.is_open(cx) {
            popup.close(cx);
        } else {
            popup.open(cx);
        }
    }
}
