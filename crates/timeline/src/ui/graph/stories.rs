//! Stories of the graph editor: Position X and Y curves with four keys, one selected with its handles, in the value
//! graph; a range that differs across a selection; and one the check refuses.

use super::{GraphCurve, GraphEditor, GraphKey, GraphMode, GraphRange};
use crate::time::{Tick, TimeView};
use crate::timecode::TimeFormat;
use crate::ui::keyframes::{KeyframeId, KeyframeInterpolation};
use crate::ui::view::TimelineView;
use craft_behavior::{Mixed, Selection};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _, Validity};
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::{Color32, Rect, Ui, Vec2};

/// Seconds between a curve's samples.
const SAMPLE_STEP: f64 = 0.05;
/// The seconds a Bezier key's handles reach to each side.
const HANDLE_REACH: f64 = 0.8;
/// The graph's height over its width.
const GRAPH_ASPECT: f32 = 0.45;
/// Points per second of the graph's time axis.
const POINTS_PER_SECOND: f64 = 90.0;
/// The sequence's end, in seconds.
const END_SECONDS: f64 = 8.0;
/// The widest value range the invalid story accepts.
const WIDEST: f64 = 200.0;

fn secs(s: f64) -> Tick {
    Tick::from_seconds_f64(s)
}

fn format() -> TimeFormat {
    TimeFormat::timecode(crate::rates::FrameRate::FPS_29_97)
}

fn bezier_like(interpolation: KeyframeInterpolation) -> bool {
    !matches!(interpolation, KeyframeInterpolation::Linear | KeyframeInterpolation::Hold)
}

/// A key with horizontal handles when it is Bezier-like and none otherwise.
fn key(id: u64, at: f64, value: f64, interpolation: KeyframeInterpolation) -> GraphKey {
    let key = GraphKey::new(KeyframeId::new(id), secs(at), value, interpolation);
    if bezier_like(interpolation) { key.handles((secs(at - HANDLE_REACH), value), (secs(at + HANDLE_REACH), value)) } else { key }
}

/// The curve through `keys`: Hold steps, Linear lines, the rest an ease.
fn curve(id: u64, name: &str, color: Color32, keys: Vec<GraphKey>) -> GraphCurve {
    let (Some(first), Some(last)) = (keys.first(), keys.last()) else { return GraphCurve::new(id, name, Vec::new(), keys).color(color) };
    let (from, to) = (first.time.seconds(), last.time.seconds());
    let steps = ((to - from) / SAMPLE_STEP).ceil() as usize;
    let samples = (0..=steps)
        .map(|step| {
            let t = (from + step as f64 * SAMPLE_STEP).min(to);
            let at = keys.partition_point(|k| k.time.seconds() <= t).saturating_sub(1);
            let (a, b) = (&keys[at], keys.get(at + 1).unwrap_or(&keys[at]));
            let span = (b.time - a.time).seconds();
            let x = if span > 0.0 { ((t - a.time.seconds()) / span).clamp(0.0, 1.0) } else { 0.0 };
            let eased = match a.interpolation {
                KeyframeInterpolation::Hold => 0.0,
                KeyframeInterpolation::Linear => x,
                _ => x * x * (3.0 - 2.0 * x),
            };
            (secs(t), a.value + (b.value - a.value) * eased)
        })
        .collect();
    GraphCurve::new(id, name, samples, keys).color(color)
}

fn curves() -> Vec<GraphCurve> {
    use KeyframeInterpolation::{AutoBezier, Bezier, Linear};
    vec![
        curve(
            1,
            "Position X",
            Color32::from_rgb(0xE5, 0x4B, 0x4B),
            vec![key(1, 0.0, 120.0, Bezier), key(2, 2.0, 340.0, Bezier), key(3, 4.0, 180.0, AutoBezier), key(4, 6.0, 420.0, Bezier)],
        ),
        curve(
            2,
            "Position Y",
            Color32::from_rgb(0x4B, 0xB5, 0x6E),
            vec![key(5, 0.0, 60.0, Linear), key(6, 2.0, 140.0, Bezier), key(7, 4.0, 300.0, Bezier), key(8, 6.0, 220.0, Linear)],
        ),
    ]
}

/// The state of a graph story: its time view, curves and selection.
#[derive(Clone, Debug, PartialEq)]
struct Demo {
    view: TimeView,
    curves: Vec<GraphCurve>,
    selection: Selection<KeyframeId>,
}

impl Demo {
    fn new() -> Self {
        Demo { view: TimeView::new(Tick::ZERO, POINTS_PER_SECOND), curves: curves(), selection: Selection::single(KeyframeId::new(2)) }
    }
}

/// Shows the editor over `range` inside a timeline view over `demo`'s view.
fn shown(ui: &mut Ui, demo: &mut Demo, mut range: Bind<'_, GraphRange>) -> StoryOutput {
    let Demo { view, curves, selection } = demo;
    let (curves, selection) = (&*curves, &*selection);
    TimelineView::new("Time", Bind::bound(view), format(), secs(END_SECONDS))
        .show(ui, |ui, _cx| {
            let full = ui.available_rect_before_wrap();
            let rect = Rect::from_min_size(full.min, Vec2::new(full.width(), full.width() * GRAPH_ASPECT));
            GraphEditor::new("Graph", range.reborrowed(), curves, selection).playhead(secs(3.0)).show_at(ui, rect)
        })
        .inner
        .map(StoryOutput::from)
        .unwrap_or_default()
}

/// The bottom-bar button that switches the editor to the speed graph.
const SPEED_BUTTON: &str = "Edit Speed Graph";

/// Presses the speed graph button: the editor's range changes its mode.
fn show_the_speed_graph(fixture: &mut Fixture) {
    let Some(button) = fixture.by_label(SPEED_BUTTON) else { return };
    fixture.click(&button);
}

#[story(GraphEditor<'static>, covers = [Default], drive = Drive::Custom(show_the_speed_graph))]
fn position_curves(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut demo = data.value("demo", Demo::new()).clone();
    let range = data.value("range", GraphRange::default());
    let out = shown(ui, &mut demo, Bind::bound(range));
    *data.value("demo", Demo::new()) = demo;
    out
}

#[story(GraphEditor<'static>, covers = [])]
fn speed_graph(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut demo = data.value("demo", Demo::new()).clone();
    let start = GraphRange { mode: GraphMode::Speed, ..GraphRange::default() };
    let range = data.value("range", start);
    let out = shown(ui, &mut demo, Bind::bound(range));
    *data.value("demo", Demo::new()) = demo;
    out
}

#[story(GraphEditor<'static>, covers = [Mixed])]
fn ranges_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut demo = data.value("demo", Demo::new()).clone();
    let range = data.value("range", Mixed::<GraphRange>::Mixed);
    let out = shown(ui, &mut demo, Bind::bound_mixed(range));
    *data.value("demo", Demo::new()) = demo;
    out
}

#[story(GraphEditor<'static>, covers = [Invalid])]
fn range_wider_than_the_limit(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut demo = data.value("demo", Demo::new()).clone();
    let wide = GraphRange { high: WIDEST + 1.0, ..GraphRange::default() };
    let range = data.value("range", wide);
    let narrow = |range: &GraphRange| {
        if range.high - range.low > WIDEST { Validity::Refuse(Message::new("The range is wider than the graph shows")) } else { Validity::Ok }
    };
    let out = shown(ui, &mut demo, Bind::bound(range).check(narrow));
    *data.value("demo", Demo::new()) = demo;
    out
}
