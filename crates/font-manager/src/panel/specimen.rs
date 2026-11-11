//! `FontSample`: text in a font face at the user's sample size, on one line at the reading start of its rect.

use craft_behavior::Sense;
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role};
use craft_ui::fonts::FontRef;
use craft_ui::primitives::{DocRun, DocText};
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, DataColor, Metrics, StyleColor, StyleCx};
use egui::{Color32, Vec2};

use crate::Catalog;
use crate::FaceInfo;
use crate::panel::faces::{self, Owner, Shown};

#[cfg(feature = "screen-testing")]
mod stories;

/// The ink of a sample: the text colour for a face that drew, the muted one for the interface face standing in.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(crate) struct FontSampleStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The sample in its own face (`text.normal`).
    ink: StyleColor,
    /// The sample in the interface face, because its own is not available (`text.muted`).
    muted: StyleColor,
}

/// The face a sample is drawn in.
#[derive(Clone, Copy)]
pub(crate) enum SampleFace<'a> {
    /// A face of `catalog`, which the document-face registry loads when the sample is measured or painted.
    Face { catalog: &'a Catalog, owner: Owner, face: &'a FaceInfo },
    /// No face is here to draw with: the sample shows in the interface face.
    Interface,
}

impl SampleFace<'_> {
    fn shown(&self, at: &impl craft_ui::primitives::FacesCx) -> Shown {
        match self {
            SampleFace::Face { catalog, owner, face } => faces::shown(at, catalog, *owner, face),
            SampleFace::Interface => Shown::Unavailable,
        }
    }
}

/// A sample of `text` in `face`, `points` high. While the face loads the sample shows nothing and keeps its band;
/// a face that is not available shows the sample in the interface face, muted. `points` is the user's sample size
/// (data, as `DocRun::size`), not a UI size. The label names the sample for a screen reader (the family and the
/// sample text).
///
/// Its measure is the sample laid out in the face at `points` (`MeasureCx::layout_doc`).
#[derive(craft_ui::contract::Component)]
#[component(content_painter)]
pub(crate) struct FontSample<'a> {
    #[common]
    common: Common,
    text: &'a str,
    face: SampleFace<'a>,
    points: f32,
}

impl<'a> FontSample<'a> {
    /// A sample of `text` in `face` at `points`, named `label`.
    pub(crate) fn new(label: impl Into<Label>, text: &'a str, face: SampleFace<'a>, points: f32) -> Self {
        FontSample { common: Common::new(), text, face, points }.label(label)
    }
}

impl Component for FontSample<'_> {
    const ROLE: Role = Role::LABEL;
    const PARTS: bool = false;
    const INPUT: craft_ui::contract::Input = craft_ui::contract::Input::Display;
    const OVERLAYS: bool = false;
    type Style = FontSampleStyle;
    type Output = egui::Response;

    fn style(cx: &StyleCx<'_>, _: FontSampleStyleKey) -> FontSampleStyle {
        let text = &cx.tokens.color.text;
        FontSampleStyle { metrics: Metrics::new(), ink: cx.color(text.normal), muted: cx.color(text.muted) }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        // A face that is loading keeps the band the interface face would take.
        let font = match self.face.shown(cx) {
            Shown::Ready(font) => font,
            _ => FontRef::proportional(),
        };
        let runs = [DocRun::new(self.text, font, self.points, DataColor::new(Color32::PLACEHOLDER))];
        cx.layout_doc(&DocText::new(&runs)).size()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let painter = ui.content_painter(cx);
        let ink = |color: StyleColor| DataColor::new(color.egui());
        match self.face.shown(cx) {
            Shown::Ready(font) => faces::paint_sample(&painter, cx.content_rect, &font, self.text, self.points, ink(cx.style.ink)),
            Shown::Unavailable => {
                faces::paint_sample(&painter, cx.content_rect, &FontRef::proportional(), self.text, self.points, ink(cx.style.muted));
            }
            _ => {}
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}
