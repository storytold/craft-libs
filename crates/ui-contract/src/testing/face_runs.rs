//! `FaceRuns`: the story card of [`Draw::face_run`](crate::primitives::Draw::face_run), text drawn in a document's own
//! face, each run on a baseline rule drawn under it.

use std::sync::Arc;

use crate::contract::{Common, Component, Cx, MeasureCx, PaintUi, Role};
use crate::fonts::{FaceBytes, deflated_default_definitions};
use crate::primitives::{FaceRun, PaintShape, StrokeWidth};
use crate::runtime::access::{Access, Name};
use crate::style::{ComponentStyle, DataColor, StyleColor, StyleCx};
use craft_behavior::Metrics;
use egui::{Color32, Context, Pos2, Response, Vec2};

/// The face the runs are set in, standing in for a document's font: egui's Hack, which has no kana.
const FACE: &str = "Hack";
/// The key the face is registered under.
const KEY: &str = "story:Hack#0";

/// The runs: their text and the size each asks for, in points.
const RUNS: [(&str, f32); 3] = [
    // In the face, at its size.
    ("Typed in Hack, 24 pt", 24.0),
    // Kana the face lacks, drawn by the interface's CJK face beside the face's Latin.
    ("Hack ファイルの名前", 24.0),
    // Far larger than the font atlas holds: drawn at the cap instead.
    ("A", 4000.0),
];

/// How far apart the baselines lie, in sizes of the run above: a line of text and the room under it.
const LEADING: f32 = 1.5;

/// The look of the card: its ink and the baseline rule.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FaceRunsStyle {
    #[style(metrics)]
    metrics: Metrics,
    ink: StyleColor,
    baseline: StyleColor,
}

/// Three runs in a document face, as an input method's composition is drawn in the text's own font: at its own size,
/// with kana the face lacks drawn by the interface fonts, and a run asked at 4,000 points drawn at the atlas's cap.
/// Each run sits on the rule drawn along its baseline.
#[derive(Component)]
#[component(exempt(
    case = "text-size",
    reason = "The 4,000-point run is drawn at the font atlas's cap, which is in device pixels: at twice the scale the cap, and the card, are half as many points."
))]
pub struct FaceRuns {
    #[common]
    common: Common,
}

impl FaceRuns {
    /// The card.
    pub fn new() -> Self {
        FaceRuns { common: Common::new() }
    }

    /// Each run with the size it asks for, the size it is drawn at and its baseline from the card's top.
    fn baselines(ctx: &Context) -> Vec<(&'static str, f32, f32, f32)> {
        let mut top = 0.0;
        RUNS.into_iter()
            .map(|(text, size)| {
                let drawn = FaceRun::new(text, KEY, size, DataColor::new(Color32::PLACEHOLDER)).drawn_size(ctx);
                let baseline = top + drawn;
                top += drawn * LEADING;
                (text, size, drawn, baseline)
            })
            .collect()
    }
}

impl Default for FaceRuns {
    fn default() -> Self {
        FaceRuns::new()
    }
}

/// The face's bytes, read once, when the face is first drawn.
fn load() -> Option<FaceBytes> {
    let definitions = deflated_default_definitions();
    let data = definitions.font_data.get(FACE)?;
    Some(FaceBytes::new(Arc::from(data.bytes()), data.index))
}

impl Component for FaceRuns {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    type Style = FaceRunsStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: FaceRunsStyleKey) -> FaceRunsStyle {
        FaceRunsStyle { metrics: Metrics::new(), ink: cx.color(cx.tokens.color.text.normal), baseline: cx.color(cx.tokens.color.accent.solid) }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let height = Self::baselines(cx.ui.ctx()).last().map_or(0.0, |(_, _, drawn, baseline)| baseline + drawn * (LEADING - 1.0));
        Vec2::new(cx.room.unwrap_or(0.0), height)
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let rect = cx.rect;
        let draw = ui.draw();
        let ink = DataColor::new(cx.style.ink.egui());
        for (text, size, _, baseline) in Self::baselines(&cx.core.ctx) {
            let start = Pos2::new(rect.left(), rect.top() + baseline);
            let layout = draw.face_run(start, &FaceRun::new(text, KEY, size, ink), load);
            let end = Pos2::new(rect.left() + layout.size().x, start.y);
            draw.stroke(&PaintShape::Path(vec![start, end]), StrokeWidth::Hairline, cx.style.baseline);
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        // The runs are the card's content: they name it.
        access.name(Name::Text(RUNS.map(|(text, _)| text).join("\n")));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
