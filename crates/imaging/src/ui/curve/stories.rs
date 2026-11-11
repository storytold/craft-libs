//! Stories of the curve editor, the preview and the tone ramp.

use super::graph::{CurveGraph, PLOT};
use super::{Curve, CurveChannel, CurveEditor, CurvePreview, ToneRamp};
use crate::ui::histogram::{ChannelColor, HistogramData};
use craft_behavior::Mixed;
use craft_ui::contract::{Bind, Show as _, Validity};
use craft_ui::runtime::env::Message;
use craft_ui::style::DataColor;
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::{Color32, Ui};

/// Sample bins of an image's channel: two hills, the channel's mean shifted by `shift`.
pub(crate) fn story_bins(shift: f32) -> Vec<u32> {
    (0..256)
        .map(|k| {
            let x = k as f32;
            let hill = |mean: f32, spread: f32, height: f32| height * (-((x - mean) / spread).powi(2)).exp();
            (hill(70.0 + shift, 28.0, 900.0) + hill(180.0 - shift, 40.0, 600.0) + 20.0) as u32
        })
        .collect()
}

fn channels() -> [CurveChannel; 4] {
    [
        CurveChannel::new("RGB", ChannelColor::Neutral),
        CurveChannel::new("Red", ChannelColor::Red),
        CurveChannel::new("Green", ChannelColor::Green),
        CurveChannel::new("Blue", ChannelColor::Blue),
    ]
}

fn bent() -> Curve {
    Curve::smooth(vec![[0.0, 0.0], [64.0, 48.0], [192.0, 214.0], [255.0, 255.0]])
}

/// The RGB composite with the other channels' curves behind it and a histogram under it.
#[story(CurveEditor<'static>, covers = [Default], drive = Drive::Drag(Grip::new(PLOT)))]
fn curve_editor(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let overlays = data
        .value(
            "overlays",
            vec![bent(), Curve::smooth(vec![[0.0, 12.0], [128.0, 140.0], [255.0, 255.0]]), Curve::identity(255.0), Curve::identity(255.0)],
        )
        .clone();
    let histogram = HistogramData::tone(&story_bins(0.0), ChannelColor::Neutral);
    let curve = data.value("curve", bent());
    let output = CurveEditor::new("Curve", Bind::bound(curve))
        .channels(&channels())
        .overlays(&overlays)
        .histogram(&histogram)
        .hint("Click to add a point · drag off or Ctrl-click or Delete to remove")
        .show(ui);
    output.into()
}

/// Pencil mode: a drag draws one output per input level.
#[story(CurveEditor<'static>, covers = [], drive = Drive::Drag(Grip::new(PLOT)))]
fn curve_editor_pencil(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let curve = data.value("drawn", Curve::Map((0..=255).map(|k| k as f32).collect()));
    CurveEditor::new("Curve", Bind::bound(curve)).pencil().show(ui).into()
}

/// A contour: corner points and fields in percent; the Corner check box toggles the selected point.
#[story(CurveEditor<'static>, covers = [], drive = Drive::Never)]
fn curve_editor_corners(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let points = vec![[0.0, 0.0], [64.0, 160.0], [128.0, 96.0], [192.0, 224.0], [255.0, 255.0]];
    let curve = data.value("contour", Curve::from_parts(points, vec![false, false, true, false, false]));
    CurveEditor::new("Contour", Bind::bound(curve)).corners().percent().show(ui).into()
}

/// Fields in percent of the levels, with the selection kept by the host.
#[story(CurveEditor<'static>, covers = [], drive = Drive::Never)]
fn curve_editor_percent(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let selected = *data.value("selected", Some(1usize));
    let curve = data.value("percent", bent());
    let output = CurveEditor::new("Curve", Bind::bound(curve)).percent().selection(selected).show(ui);
    *data.value("selected", Some(1usize)) = output.selected;
    output.into()
}

/// The plot of the curve editor alone.
#[story(CurveGraph<'static>, covers = [Default], drive = Drive::Drag(Grip::new(PLOT)))]
fn curve_graph(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    CurveGraph::new("Curve", Bind::bound(data.value("curve", bent()))).show(ui).into()
}

#[story(CurveGraph<'static>, covers = [Mixed], drive = Drive::Never)]
fn curve_graph_that_differs(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    CurveGraph::new("Curve", Bind::bound_mixed(data.value("mixed", Mixed::<Curve>::Mixed))).show(ui).into()
}

/// A graph that refuses a curve of more than two points.
#[story(CurveGraph<'static>, covers = [Invalid], drive = Drive::Never)]
fn curve_graph_that_is_refused(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse = |curve: &Curve| {
        if curve.points().is_some_and(|points| points.len() > 2) { Validity::Refuse(Message::new("Too many points")) } else { Validity::Ok }
    };
    CurveGraph::new("Curve", Bind::bound(data.value("refused", bent())).check(refuse)).show(ui).into()
}

/// One curve, read-only.
#[story(CurvePreview<'static>, covers = [Default], drive = Drive::Never)]
fn curve_preview(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let curve = data.value("curve", bent()).clone();
    CurvePreview::new("Preset", &curve).show(ui).into()
}

/// The ramp the tone bars and the slider tracks share.
#[story(ToneRamp<'static>, covers = [Default], drive = Drive::Never)]
fn tone_ramp(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let stops = [DataColor::new(Color32::from_gray(0)), DataColor::new(Color32::from_gray(u8::MAX))];
    ToneRamp::new(&stops).show(ui).into()
}
