//! Stories of the ruler: frame, second and minute zoom with the marker strip above, a work area and cache bars, a
//! playhead that differs across a selection, and one the check refuses.

use super::{RenderBarState, RenderSpan, SplitMarks, TimelineRuler, TimelineRulerResponse};
use crate::markers::{Marker, MarkerColor, MarkerId};
use crate::rates::FrameRate;
use crate::time::{Tick, TimeRange, TimeView};
use crate::timecode::TimeFormat;
use crate::ui::markers::MarkerStrip;
use crate::ui::view::TimelineView;
use craft_behavior::Mixed;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _, Validity};
use craft_ui::layout_components::Stack;
use craft_ui::runtime::env::Message;
use craft_ui::testing::{StoryData, StoryOutput, story};
use egui::Ui;

const RATE: FrameRate = FrameRate::FPS_29_97;

/// The end of the sequence the ruler stories show, in seconds.
const END_SECONDS: f64 = 90.0;

/// Points per second at about twelve points a frame, at the default timeline zoom and at four points a second.
const FRAME_ZOOM: f64 = 360.0;
const SECOND_ZOOM: f64 = 40.0;
const MINUTE_ZOOM: f64 = 4.0;

/// The latest time the invalid story accepts for the playhead, in seconds.
const LATEST_SECONDS: f64 = 10.0;

fn secs(s: f64) -> Tick {
    Tick::from_seconds_f64(s)
}

fn span(start: f64, end: f64) -> TimeRange {
    TimeRange::from_bounds(secs(start), secs(end))
}

/// 29.97 drop-frame timecode.
fn format() -> TimeFormat {
    TimeFormat::timecode(RATE).drop_frame(true)
}

/// The state of a ruler story: its view, playhead, In and Out, work area and render bar.
#[derive(Clone, Debug, PartialEq)]
struct Demo {
    view: TimeView,
    playhead: Tick,
    work_area: TimeRange,
    render: Vec<RenderSpan>,
}

impl Demo {
    fn at(start: Tick, zoom: f64) -> Self {
        Demo {
            view: TimeView::new(start, zoom),
            playhead: secs(5.0),
            work_area: span(2.0, 60.0),
            render: vec![
                RenderSpan::new(span(1.0, 8.0), RenderBarState::Rendered, "Rendered preview"),
                RenderSpan::new(span(8.0, 20.0), RenderBarState::OnDisk, "Frames on disk"),
                RenderSpan::new(span(20.0, 31.0), RenderBarState::Rendered, "Rendered preview"),
            ],
        }
    }
}

/// A point marker and a range marker, as the timeline panel's strip shows them above its ruler.
fn markers() -> [Marker; 2] {
    let marker = |id, start, duration, name: &str, color| Marker {
        id: MarkerId(id),
        start: secs(start),
        duration: secs(duration),
        name: name.into(),
        comment: String::new(),
        color: MarkerColor(color),
    };
    [marker(1, 6.0, 0.0, "Check this cut", 0), marker(2, 30.0, 8.0, "Music swell", 8)]
}

/// Shows the ruler `show` makes inside a strip timeline view over `view`, which publishes the time axis the ruler
/// reads. `show` builds the ruler and shows it on every run of the view's body.
fn in_view(ui: &mut Ui, view: &mut TimeView, mut show: impl FnMut(&mut Ui) -> Option<TimelineRulerResponse>) -> StoryOutput {
    TimelineView::new("Time", Bind::bound(view), format(), secs(END_SECONDS))
        .strip()
        .show(ui, |ui, _cx| show(ui))
        .inner
        .flatten()
        .map(StoryOutput::from)
        .unwrap_or_default()
}

fn ruler<'a>(playhead: Bind<'a, Tick>, demo_work: TimeRange, render: &'a [RenderSpan]) -> TimelineRuler<'a> {
    TimelineRuler::new("Time ruler", playhead)
        .in_out((Some(secs(3.0)), Some(secs(40.0))))
        .work_area(demo_work)
        .render_bar(render)
        .split_marks(split_marks())
}

fn split_marks() -> SplitMarks {
    SplitMarks { video_in: Some(secs(12.0)), audio_out: Some(secs(26.0)), ..SplitMarks::default() }
}

/// The marker strip above the ruler over `demo`, as the timeline panel stacks them.
fn with_markers(ui: &mut Ui, demo: &mut Demo) -> StoryOutput {
    let Demo { view, playhead, work_area, render } = demo;
    let markers = markers();
    in_view(ui, view, |ui| {
        Stack::new()
            .show(ui, |ui, _| {
                MarkerStrip::new("Markers", &markers).show(ui);
                ruler(Bind::bound(&mut *playhead), *work_area, render).show(ui)
            })
            .inner
    })
}

#[story(TimelineRuler<'static>, covers = [])]
fn ruler_at_frame_zoom(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    with_markers(ui, data.value("frame", Demo::at(secs(4.0), FRAME_ZOOM)))
}

#[story(TimelineRuler<'static>, covers = [Default])]
fn ruler_at_second_zoom(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    with_markers(ui, data.value("second", Demo::at(Tick::ZERO, SECOND_ZOOM)))
}

#[story(TimelineRuler<'static>, covers = [])]
fn ruler_at_minute_zoom(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    with_markers(ui, data.value("minute", Demo::at(Tick::ZERO, MINUTE_ZOOM)))
}

#[story(TimelineRuler<'static>, covers = [Mixed])]
fn playheads_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut view = *data.value("view", TimeView::new(Tick::ZERO, SECOND_ZOOM));
    let playhead = data.value("playhead", Mixed::<Tick>::Mixed);
    let shown = in_view(ui, &mut view, |ui| Some(TimelineRuler::new("Time ruler", Bind::bound_mixed(&mut *playhead)).show(ui)));
    *data.value("view", TimeView::new(Tick::ZERO, SECOND_ZOOM)) = view;
    shown
}

#[story(TimelineRuler<'static>, covers = [Invalid])]
fn playhead_after_the_limit(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut view = *data.value("view", TimeView::new(Tick::ZERO, SECOND_ZOOM));
    let playhead = data.value("playhead", secs(LATEST_SECONDS + 1.0));
    let shown = in_view(ui, &mut view, |ui| {
        let before_the_limit = |t: &Tick| {
            if *t > secs(LATEST_SECONDS) { Validity::Refuse(Message::new("The playhead is past the limit")) } else { Validity::Ok }
        };
        Some(TimelineRuler::new("Time ruler", Bind::bound(&mut *playhead).check(before_the_limit)).show(ui))
    });
    *data.value("view", TimeView::new(Tick::ZERO, SECOND_ZOOM)) = view;
    shown
}
