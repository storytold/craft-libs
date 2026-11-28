//! `FontPreviewBanner`: the family the picker previews, named and set in its own face, on a well at the top of the
//! list.

use craft_behavior::Sense;
use craft_theme::TypeRole;
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role};
use craft_ui::primitives::TextRun;
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, Corners, DataColor, Metrics, StyleColor, StyleCx, SurfaceKind};
use egui::{Color32, Pos2, Rect, Vec2};

use crate::panel::faces::{self, Sample};
use crate::panel::specimen::SampleFace;

use super::SAMPLE_EM;

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The style of the banner: the well's padding, the name line and the sample band.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(crate) struct FontPreviewBannerStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// Around the name and the sample, inside the well: `space.pad.regular`.
    pad: f32,
    /// Between the name and the sample: `space.gap.tight`.
    gap: f32,
    /// The name's line: the caption role's line height.
    name_line: f32,
    /// The name's ink: `text.faint`.
    name: StyleColor,
    /// The sample band: the large thumbnail side (`size.thumb.lg`).
    band: f32,
    /// The sample's ink: `text.normal`.
    ink: StyleColor,
}

/// The preview at the top of the font list: a well (`SurfaceKind::Sunken`) as wide as the list, the family's name in
/// the caption role and under it `text` in the family's face, one line cut at the end. Until the face is ready the band
/// stays empty. The label names it for a screen reader.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(content_painter)]
pub(crate) struct FontPreviewBanner<'a> {
    #[common]
    common: Common,
    name: &'a str,
    text: &'a str,
    face: SampleFace<'a>,
    /// The sample in black on a white paper over its band, in every theme (`FontPickerSettings::samples_on_white`).
    #[prop]
    on_white: bool,
}

impl<'a> FontPreviewBanner<'a> {
    /// The banner of the family named `name`, setting `text` in `face`, named `label`.
    pub(crate) fn new(label: impl Into<Label>, name: &'a str, text: &'a str, face: SampleFace<'a>) -> Self {
        FontPreviewBanner { common: Common::new(), name, text, face, on_white: false }.label(label)
    }
}

impl Component for FontPreviewBanner<'_> {
    const ROLE: Role = Role::LABEL;
    const PARTS: bool = false;
    const INPUT: craft_ui::contract::Input = craft_ui::contract::Input::Display;
    const OVERLAYS: bool = false;
    type Style = FontPreviewBannerStyle;
    type Output = egui::Response;

    fn style(cx: &StyleCx<'_>, _: FontPreviewBannerStyleKey) -> FontPreviewBannerStyle {
        let tokens = cx.tokens;
        FontPreviewBannerStyle {
            metrics: Metrics::new(),
            pad: tokens.space.pad.regular,
            gap: tokens.space.gap.tight,
            name_line: tokens.typography.get(TypeRole::Caption).line_height,
            name: cx.color(tokens.color.text.faint),
            band: tokens.size.thumb.lg,
            ink: cx.color(tokens.color.text.normal),
        }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    /// As wide as the name or the sample, whichever is wider, in the room less the padding (it fills the width the flow
    /// offers beyond that); as high as the padding, the name line, the gap and the sample band.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let style = cx.style;
        let inner = cx.room.map(|room| (room - 2.0 * style.pad).max(0.0));
        let sample = self.face.ready(cx).map_or(0.0, |font| {
            let ink = DataColor::new(Color32::PLACEHOLDER);
            let sample = Sample::new(self.text, &font, style.band * SAMPLE_EM, ink);
            faces::sample_line(|doc| cx.layout_doc(doc), sample, style.band, inner).layout.size().x
        });
        let name = TextRun::new(self.name).role(TypeRole::Caption);
        let name = inner.map_or_else(|| cx.layout(&name), |width| cx.layout_at(&name, width)).size().x;
        Vec2::new(2.0 * style.pad + sample.max(name), 2.0 * style.pad + style.name_line + style.gap + style.band)
    }

    fn fills(&self) -> Option<craft_behavior::Axis> {
        Some(craft_behavior::Axis::X)
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        ui.draw().surface(cx.rect, SurfaceKind::Sunken, Corners::All);
        let inner = cx.rect.shrink(style.pad);
        let name = Rect::from_min_size(inner.min, Vec2::new(inner.width(), style.name_line));
        ui.draw().text(name, &TextRun::new(self.name).role(TypeRole::Caption).color(style.name));
        let band = Rect::from_min_max(Pos2::new(inner.left(), name.bottom() + style.gap), inner.max);
        let points = style.band * SAMPLE_EM;
        let painter = ui.content_painter(cx);
        let ink = faces::sample_ink(&painter, band, self.on_white, style.ink.egui());
        if let Some(font) = self.face.ready(cx) {
            faces::paint_sample(&painter, band, Sample::new(self.text, &font, points, ink));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}
