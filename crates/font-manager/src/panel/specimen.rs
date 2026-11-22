//! `FontSample`: text in a font face at the user's sample size, on one line at the reading start of its rect.

use std::borrow::Cow;

use craft_behavior::Sense;
use craft_ui::contract::{Common, Component, Cx, DisabledLook, MeasureCx, PaintUi, Props as _, Role};
use craft_ui::fonts::FontRef;
use craft_ui::primitives::DocRun;
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, DataColor, Metrics, StyleColor, StyleCx};
use egui::{Color32, Vec2};

use crate::Catalog;
use crate::FaceInfo;
use crate::panel::faces::{self, Owner, Shown};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The height of a sample whose face is not ready, in ems of its size: a usual line height (Inter's line box is 1.21
/// em), so a row keeps about the height it has once the face draws, without laying the text out in a face that may
/// lack its characters.
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
/// Its measure is the sample laid out in the face at `points` on one row, cut with an ellipsis where the room does
/// not fit it (`faces::sample_text`, the layout `paint` draws). Without a ready face it reserves the most that layout
/// can take, the room's width, at [`LINE_EM`] of `points` high: the band the sample will draw in, so the face
/// arriving never widens it.
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
}

impl<'a> FontSample<'a> {
    /// A sample of `text` in `face` at `points`, named `label`.
    pub(crate) fn new(label: impl Into<Label>, text: impl Into<Cow<'a, str>>, face: SampleFace<'a>, points: f32) -> Self {
        FontSample { common: Common::new(), text: text.into(), face, points, on_white: false }.label(label)
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
        let Some(font) = self.face.ready(cx) else {
            return Vec2::new(cx.room.unwrap_or(0.0), self.points * LINE_EM);
        };
        let runs = [DocRun::new(self.text.as_ref(), font, self.points, DataColor::new(Color32::PLACEHOLDER))];
        cx.layout_doc(&faces::sample_text(&runs, cx.room)).size()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let painter = ui.content_painter(cx);
        let ink = faces::sample_ink(&painter, cx.rect, self.on_white, cx.style.ink.egui());
        if let Some(font) = self.face.ready(cx) {
            faces::paint_sample(&painter, cx.content_rect, &font, &self.text, self.points, ink);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}
