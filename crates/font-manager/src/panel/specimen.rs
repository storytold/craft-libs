//! `FontSample`: text in a font face at the user's sample size, on one line at the reading start of its rect.

use std::borrow::Cow;

use craft_behavior::Sense;
use craft_ui::contract::{Common, Component, Cx, DisabledLook, MeasureCx, PaintUi, Props as _, Role};
use craft_ui::fonts::FontRef;
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, DataColor, Metrics, StyleColor, StyleCx};
use egui::{Color32, Vec2};

use crate::Catalog;
use crate::FaceInfo;
use crate::panel::faces::{self, Owner, Shown};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The band of a sample whose caller sets none, in ems of its size: a usual line height (Inter's line box is 1.21 em).
const LINE_EM: f32 = 1.2;

/// The ink of a sample.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(crate) struct FontSampleStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The sample in its own face (`text.normal`).
    ink: StyleColor,
}

/// The face a sample is drawn in.
#[derive(Clone, Copy)]
pub(crate) enum SampleFace<'a> {
    /// A face of `catalog`, which the document-face registry loads when the sample is measured or painted.
    Face { catalog: &'a Catalog, owner: Owner, face: &'a FaceInfo },
    /// No face is here to draw with: the sample keeps its band and shows nothing.
    Missing,
}

impl SampleFace<'_> {
    /// The font that draws the sample once its face is ready; none while it loads, when it is not available, or for
    /// [`SampleFace::Missing`]. A sample is laid out and drawn only in this font, never in the interface face, which
    /// may lack the sample's characters.
    pub(crate) fn ready(&self, at: &impl craft_ui::primitives::FacesCx) -> Option<FontRef> {
        match self {
            SampleFace::Face { catalog, owner, face } => match faces::shown(at, catalog, *owner, face) {
                Shown::Ready(font) => Some(font),
                _ => None,
            },
            SampleFace::Missing => None,
        }
    }
}

/// A sample of `text` in `face`, `points` high. Until the face is ready (while it loads, and for good when it is not
/// available) the sample shows nothing and keeps its band. `points` is the user's sample size (data, as
/// `DocRun::size`), not a UI size. The label names the sample for a screen reader (the family and the sample text).
///
/// The sample is as high as its band ([`FontSample::band`], else [`LINE_EM`] of `points`) whatever its face, and sits
/// on the band's baseline, smaller than `points` only where its ink would leave the band (`faces::sample_line`): every
/// face's sample in a list of bands sits at one height, and none is clipped. Its width is the sample's layout cut with
/// an ellipsis where the room does not fit it (`faces::sample_text`, the layout `paint` draws). Without a ready face
/// it reserves the most that layout can take, the room's width, so the face arriving never widens it.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(content_painter)]
pub(crate) struct FontSample<'a> {
    #[common]
    common: Common,
    text: Cow<'a, str>,
    face: SampleFace<'a>,
    points: f32,
    /// The height of the band the sample is set in, from the caller's sizes (a list row's thumbnail side), instead of
    /// [`LINE_EM`] of its size.
    #[prop(quantity = "band height the caller's layout gives, in points")]
    band: Option<f32>,
}

impl<'a> FontSample<'a> {
    /// A sample of `text` in `face` at `points`, named `label`.
    pub(crate) fn new(label: impl Into<Label>, text: impl Into<Cow<'a, str>>, face: SampleFace<'a>, points: f32) -> Self {
        FontSample { common: Common::new(), text: text.into(), face, points, band: None }.label(label)
    }
}

impl Component for FontSample<'_> {
    const DISABLED: DisabledLook = DisabledLook::Faded;
    const ROLE: Role = Role::LABEL;
    const PARTS: bool = false;
    const INPUT: craft_ui::contract::Input = craft_ui::contract::Input::Display;
    const OVERLAYS: bool = false;
    type Style = FontSampleStyle;
    type Output = egui::Response;

    fn style(cx: &StyleCx<'_>, _: FontSampleStyleKey) -> FontSampleStyle {
        FontSampleStyle { metrics: Metrics::new(), ink: cx.color(cx.tokens.color.text.normal) }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let band = self.band.unwrap_or(self.points * LINE_EM);
        let Some(font) = self.face.ready(cx) else {
            return Vec2::new(cx.room.unwrap_or(0.0), band);
        };
        let ink = DataColor::new(Color32::PLACEHOLDER);
        let line = faces::sample_line(|doc| cx.layout_doc(doc), &font, &self.text, self.points, band, cx.room, ink);
        Vec2::new(line.layout.size().x, band)
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if let Some(font) = self.face.ready(cx) {
            let ink = DataColor::new(cx.style.ink.egui());
            faces::paint_sample(&ui.content_painter(cx), cx.content_rect, &font, &self.text, self.points, ink);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}
