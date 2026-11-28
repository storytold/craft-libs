//! `FontSample`: text in a font face at the user's sample size, on one line at the reading start of its rect.

use std::borrow::Cow;
use std::sync::Arc;

use craft_behavior::Sense;
use craft_ui::contract::{Common, Component, Cx, DisabledLook, MeasureCx, PaintUi, Props as _, Role};
use craft_ui::fonts::FontRef;
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, DataColor, Metrics, StyleColor, StyleCx};
use egui::{Color32, Vec2};

use crate::Catalog;
use crate::FaceInfo;
use crate::panel::faces::{self, Owner, Sample, Shown};
use crate::panel::subsets::SubsetFont;

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
    Face { catalog: &'a Arc<Catalog>, owner: Owner, face: &'a FaceInfo },
    /// A subset of a face from the preview service.
    Subset(&'a SubsetFont),
    /// A face the host gives, ready as it is (a host row of the picker).
    Font(&'a FontRef),
    /// No face is here to draw with: the sample keeps its band and shows nothing.
    Missing,
}

impl SampleFace<'_> {
    /// The font that draws the sample once its face is ready; none while it loads, when it is not available, or for
    /// [`SampleFace::Missing`]. A sample is laid out and drawn only in this font, never in the interface face, which
    /// may lack the sample's characters.
    pub(crate) fn ready(&self, at: &impl craft_ui::primitives::FacesCx) -> Option<FontRef> {
        match self {
            SampleFace::Face { catalog, owner, face } => ready(faces::shown(at, catalog, *owner, face)),
            SampleFace::Subset(subset) => ready(faces::subset(at, subset)),
            SampleFace::Font(font) => Some((*font).clone()),
            SampleFace::Missing => None,
        }
    }
}

fn ready(shown: Shown) -> Option<FontRef> {
    match shown {
        Shown::Ready(font) => Some(font),
        _ => None,
    }
}

/// The face a card or a style row draws its sample in, as the panel holds it.
#[derive(Clone)]
pub(crate) enum HeldFace {
    /// A face of `catalog`.
    Local { owner: Owner, face: Box<FaceInfo>, catalog: Arc<Catalog> },
    /// A subset of a face from the preview service.
    Subset(SubsetFont),
}

impl HeldFace {
    /// `face` of `owner`'s catalog `catalog`.
    pub(crate) fn local(owner: Owner, face: FaceInfo, catalog: &Arc<Catalog>) -> HeldFace {
        HeldFace::Local { owner, face: Box::new(face), catalog: Arc::clone(catalog) }
    }

    /// Whether the face is installed here.
    pub(crate) fn installed(&self) -> bool {
        matches!(self, HeldFace::Local { owner: Owner::Installed, .. })
    }

    /// The face of a sample: `face`, or none.
    pub(crate) fn sample(face: Option<&HeldFace>) -> SampleFace<'_> {
        match face {
            Some(HeldFace::Local { owner, face, catalog }) => SampleFace::Face { catalog, owner: *owner, face },
            Some(HeldFace::Subset(subset)) => SampleFace::Subset(subset),
            None => SampleFace::Missing,
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
    /// Black type on a white paper over the whole rect, in every theme (`FontPickerSettings::samples_on_white`).
    #[prop]
    on_white: bool,
    /// The height of the band the sample is set in, from the caller's sizes (a list row's thumbnail side), instead of
    /// [`LINE_EM`] of its size.
    #[prop(quantity = "band height the caller's layout gives, in points")]
    band: Option<f32>,
    /// The variation coordinates (axis tag, value) the face is drawn at, over its own: one registered variable face
    /// draws every named instance and every axis value.
    #[prop]
    coords: &'a [([u8; 4], f32)],
}

impl<'a> FontSample<'a> {
    /// A sample of `text` in `face` at `points`, named `label`.
    pub(crate) fn new(label: impl Into<Label>, text: impl Into<Cow<'a, str>>, face: SampleFace<'a>, points: f32) -> Self {
        FontSample { common: Common::new(), text: text.into(), face, points, on_white: false, band: None, coords: &[] }.label(label)
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
        let sample = Sample::new(&self.text, &font, self.points, ink).at(self.coords);
        let line = faces::sample_line(|doc| cx.layout_doc(doc), sample, band, cx.room);
        Vec2::new(line.layout.size().x, band)
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let painter = ui.content_painter(cx);
        let ink = faces::sample_ink(&painter, cx.rect, self.on_white, cx.style.ink.egui());
        if let Some(font) = self.face.ready(cx) {
            faces::paint_sample(&painter, cx.content_rect, Sample::new(&self.text, &font, self.points, ink).at(self.coords));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}
