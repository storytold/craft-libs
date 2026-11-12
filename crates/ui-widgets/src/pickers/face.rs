//! `SelectFace`: the closed box of a dropdown picker, the text and a trailing chevron.

use crate::contract::{Common, Component, Cx, MeasureCx, PaintUi, Pointer as _, PopupHandle, Props as _, Role, Styled as _};
use crate::list::{RowThumb, ThumbAspect, ThumbPaint};
use crate::primitives::{ContentPainterCx, IconSpec, TextRun};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, IconLook, Ink, StyleCx, TextLook, TypeRole};
use crate::text::{Adornment, FieldStyle, place, reserve};
use craft_behavior::{Flow, Metrics, Proposal, Sense};
use craft_theme::{Stateful, States};
use egui::{Pos2, Rect, Response, Vec2};

/// The style of a select face: the field's metrics, the text, the faint placeholder, the thumbnail's height and its
/// caption, and the chevron. The host owns the
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
    /// The line under the thumbnail.
    #[style(text)]
    caption: TextLook,
    /// The gap between the thumbnail and its caption: `gap.tight`.
    caption_gap: f32,
    /// The field look the chevron is placed by.
    field: FieldStyle,
}

/// The closed box of a picker: `text` elided to the room left of a trailing chevron. The box is as wide as the text
/// and the chevron need, at least a field wide; a thumbnail alone takes only its own width. A host shows it as the look of one of its parts with
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
    /// A short line centred under the thumbnail (a brush's size); it shows only with a thumbnail.
    #[prop(into)]
    thumb_caption: Option<Label>,
    /// Whether the text shows beside the thumbnail; `false` shows the thumbnail alone, as wide as the thumbnail and
    /// the chevron need. The text is the face's accessible name either way.
    #[style_key]
    #[prop]
    name: bool,
}

impl<'a> SelectFace<'a> {
    /// A face showing `text`, which is also its name.
    pub fn new(text: impl Into<Label>) -> Self {
        SelectFace { common: Common::new(), placeholder: false, thumb: None, leading: None, thumb_caption: None, name: true }.label(text)
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

    /// The width of the thumbnail's column, as wide as the picture and its caption `caption` wide, and its gap to the
    /// text; nothing without a thumbnail.
    fn lead(&self, style: &SelectFaceStyle, caption: f32) -> f32 {
        self.thumb.as_ref().map_or(0.0, |(aspect, _)| aspect.of(style.thumb).max(caption) + if self.name { style.field.metrics().gap } else { 0.0 })
    }

    /// The caption's run, when there is a thumbnail to show it under.
    fn caption_run<'s>(&'s self, look: &TextLook) -> Option<TextRun<'s>> {
        self.thumb.as_ref().and(self.thumb_caption.as_ref()).map(|caption| TextRun::new(caption.as_str()).role(look.role))
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

    fn style(cx: &StyleCx<'_>, key: SelectFaceStyleKey) -> SelectFaceStyle {
        let field = FieldStyle::at(cx);
        // A thumbnail alone keeps the field's padding and height, not the widths a field keeps for its text.
        let metrics = field.metrics();
        let metrics = if key.name { metrics } else { metrics.ideal_width(0.0).min_size(Vec2::new(0.0, metrics.min_size.y)) };
        let off = cx.color(cx.tokens.color.text.disabled);
        let (ink, muted) = (cx.ink(Ink::OnPanel), cx.color(cx.tokens.color.text.faint));
        let caption = cx.color(cx.tokens.color.text.muted);
        let ink = Stateful::new(ink).off(States::new(ink, ink, ink, off));
        SelectFaceStyle {
            metrics,
            text: TextLook::new(TypeRole::Body, cx.host_ink(ink.clone())),
            placeholder: TextLook::new(TypeRole::Body, cx.host_muted_ink(Stateful::new(muted).off(States::new(muted, muted, muted, off)))),
            chevron: IconLook::new(field.glyph().side, cx.host_ink(ink)),
            thumb: cx.tokens.size.thumb.sm,
            caption: TextLook::new(TypeRole::Caption, cx.host_muted_ink(Stateful::new(caption).off(States::new(caption, caption, caption, off)))),
            caption_gap: cx.tokens.space.gap.tight,
            field,
        }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        // At its least the face elides its name to an ellipsis.
        let text = match (self.name, cx.proposal) {
            (true, Proposal::Min) => {
                let ellipsis = cx.layout(&TextRun::new("…").role(cx.style.text.role)).size().x;
                Vec2::new(ellipsis, cx.layout(&TextRun::new(cx.resolved.label).role(cx.style.text.role)).size().y)
            }
            (true, _) => cx.layout(&TextRun::new(cx.resolved.label).role(cx.style.text.role)).size(),
            (false, _) => Vec2::ZERO,
        };
        let caption = self.caption_run(&cx.style.caption).map_or(Vec2::ZERO, |run| cx.layout(&run).size());
        let picture = match (&self.thumb, caption.y > 0.0) {
            (None, _) => 0.0,
            (Some(_), false) => cx.style.thumb,
            (Some(_), true) => cx.style.thumb + cx.style.caption_gap + caption.y,
        };
        Vec2::new(self.lead(cx.style, caption.x) + text.x + reserve(&cx.style.field, &self.leading_glyphs(), &[Self::chevron()]), text.y.max(picture))
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let chevron = Self::chevron();
        let rtl = cx.flow.is_rtl();
        let mut flow = Flow::new(cx.content_rect, rtl);
        let caption = self.caption_run(&cx.style.caption).map(|run| ui.draw().layout(&run.color(cx.resolve(&cx.style.caption.color)), f32::INFINITY));
        let caption_size = caption.as_ref().map_or(Vec2::ZERO, |caption| caption.size());
        if let Some((aspect, paint)) = self.thumb.take() {
            let (cell, rest) = flow.take_start(aspect.of(cx.style.thumb).max(caption_size.x));
            let below = if caption.is_some() { cx.style.caption_gap + caption_size.y } else { 0.0 };
            let top = cell.center().y - (cx.style.thumb + below) / 2.0;
            let picture = Rect::from_min_size(
                Pos2::new(cell.center().x - aspect.of(cx.style.thumb) / 2.0, top),
                Vec2::new(aspect.of(cx.style.thumb), cx.style.thumb),
            );
            let _ = ui.child("thumb", picture, RowThumb::new(paint));
            if let Some(caption) = &caption {
                ui.draw().galley(Pos2::new(cell.center().x - caption_size.x / 2.0, picture.bottom() + cx.style.caption_gap), caption);
            }
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
