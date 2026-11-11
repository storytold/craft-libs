//! A decoded frame fitted in a panel, with a status line.

#[cfg(feature = "screen-testing")]
mod stories;

use craft_behavior::{Axis, Metrics, Sense};
use craft_theme::Stateful;
use craft_ui::contract::{Access, Common, Component, Cx, Input, MeasureCx, Name, PaintUi, Props, Role, Styled as _};
use craft_ui::primitives::{PaintShape, TextRun};
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, StyleColor, StyleCx, TextLook, TypeRole};
use egui::{CornerRadius, Rect, Response, TextureId, Vec2, pos2};

/// How a frame fills its box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum FrameFit {
    /// Scaled to fit, centred, the rest the canvas colour (letterbox or pillarbox).
    #[default]
    Letterbox,
    /// The part a crop leaves (left, top, right, bottom fractions, each clamped to 0 to 0.99) stretched over the
    /// whole box, as PowerPoint shows a cropped video.
    Crop([f32; 4]),
}

/// The rect a frame of `size` pixels fills inside `area` and the part of the frame it shows, as texture
/// coordinates. A frame without a size has an empty rect at the centre of `area`. Non-finite crop values count as 0;
/// the visible part keeps at least 1 % of each axis.
pub fn frame_rect(area: Rect, size: Vec2, fit: FrameFit) -> (Rect, Rect) {
    match fit {
        FrameFit::Letterbox => {
            let valid = size.x.is_finite() && size.y.is_finite() && size.x > 0.0 && size.y > 0.0;
            let scale = if valid { (area.width() / size.x).min(area.height() / size.y) } else { 0.0 };
            (Rect::from_center_size(area.center(), size * scale), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)))
        }
        FrameFit::Crop(crop) => (area, crop_uv(crop)),
    }
}

/// The part of the frame a crop leaves visible, as texture coordinates.
fn crop_uv(crop: [f32; 4]) -> Rect {
    let [left, top, right, bottom] = crop.map(|v| if v.is_finite() { v.clamp(0.0, 0.99) } else { 0.0 });
    Rect::from_min_max(pos2(left, top), pos2((1.0 - right).max(left + 0.01), (1.0 - bottom).max(top + 0.01)))
}

/// The style of the preview: the status line's look.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PreviewStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The status line under the frame.
    #[style(text)]
    status: TextLook,
    /// Behind the frame, where it does not reach.
    backdrop: StyleColor,
}

/// A preview panel: the frame fitted on the canvas colour and, under it, a status line in the caption role ("Movie.mov \u{b7}
/// 00:01:02:03", "Offline", a decode error). Without a frame it shows the status alone, centred. Not interactive;
/// AccessKit: an image named by the status.
#[derive(craft_ui::contract::Component, Props)]
#[component(content_painter)]
pub struct FramePreview {
    #[common]
    common: Common,
    frame: Option<(TextureId, Vec2)>,
    /// How the frame fits its box; letterbox by default.
    #[prop]
    fit: FrameFit,
    /// The status line text; an empty text shows no line.
    #[prop(into)]
    status: Option<Label>,
}

impl FramePreview {
    /// A preview named `label` of `frame` (texture and size in pixels); `None` shows the status alone.
    pub fn new(label: impl Into<Label>, frame: Option<(TextureId, Vec2)>) -> Self {
        FramePreview { common: Common::new(), frame, fit: FrameFit::Letterbox, status: None }.label(label)
    }

    fn status_text(&self) -> Option<&str> {
        self.status.as_ref().map(Label::as_str).filter(|text| !text.is_empty())
    }
}

impl Component for FramePreview {
    const ROLE: Role = Role::IMAGE;
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = PreviewStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: FramePreviewStyleKey) -> PreviewStyle {
        PreviewStyle {
            metrics: Metrics::new(),
            status: TextLook::new(TypeRole::Caption, Stateful::new(cx.color(cx.tokens.color.text.muted))),
            backdrop: cx.color(cx.tokens.color.surface.canvas),
        }
    }

    fn fills(&self) -> Option<Axis> {
        Some(Axis::Y)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(cx.room.unwrap_or(0.0), 0.0)
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let rect = cx.rect;
        let run = self.status_text().map(|text| TextRun::new(text).role(cx.style.status.role).color(cx.resolve(&cx.style.status.color)).elide());
        let line = run.as_ref().map(|run| ui.draw().layout(run, rect.width()).size());
        match (self.frame, run, line) {
            (Some((texture, size)), run, line) => {
                let strip = line.map_or(0.0, |line| line.y);
                let area = Rect::from_min_max(rect.min, pos2(rect.max.x, (rect.max.y - strip).max(rect.min.y)));
                ui.draw().fill(&PaintShape::Rect(area, CornerRadius::ZERO), cx.style.backdrop);
                let (target, uv) = frame_rect(area, size, self.fit);
                ui.content_painter(cx).picture(texture, target, uv, 0.0);
                if let (Some(run), Some(line)) = (run, line) {
                    let at = Rect::from_min_size(pos2(rect.center().x - line.x / 2.0, rect.max.y - line.y), line);
                    ui.draw().text(at, &run);
                }
            }
            (None, Some(run), Some(line)) => {
                ui.draw().text(Rect::from_center_size(rect.center(), line), &run);
            }
            (None, _, _) => {}
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        if let Some(text) = self.status_text() {
            access.name(Name::Text(text.to_owned()));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
