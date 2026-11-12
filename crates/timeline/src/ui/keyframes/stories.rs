//! Stories of the keyframe lane, the keyframe navigator and the animation toggle.

use craft_behavior::Selection;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::testing::{Drive, StoryData, StoryOutput, stage_width, story};
use egui::Ui;

use super::{AnimationToggle, KeyframeId, KeyframeInterpolation, KeyframeLane, KeyframeMark, KeyframeNavigator};
use crate::rates::FrameRate;
use crate::time::{Tick, TimeRange, TimeView};
use crate::timecode::TimeFormat;
use crate::ui::TimelineView;

/// The span the lane story's view fits to the stage's width, in seconds: every key shows.
const SHOWN_SECONDS: f64 = 12.0;

fn seconds(at: f64) -> Tick {
    Tick::from_seconds_f64(at)
}

fn keys() -> Vec<KeyframeMark> {
    let key = |id: u64, at: f64, interpolation| KeyframeMark::new(KeyframeId::new(id), seconds(at), interpolation);
    vec![
        key(1, 1.0, KeyframeInterpolation::Linear),
        key(2, 3.0, KeyframeInterpolation::Hold),
        key(3, 5.0, KeyframeInterpolation::Bezier),
        key(4, 7.5, KeyframeInterpolation::Linear),
        key(5, 10.0, KeyframeInterpolation::Bezier),
    ]
}

/// The view a story starts from: [`SHOWN_SECONDS`] across the stage's width.
fn start_view(ui: &Ui) -> TimeView {
    TimeView::fit(TimeRange::new(Tick::ZERO, seconds(SHOWN_SECONDS)), stage_width(ui), 0.0)
}

/// Shows the lane `show` makes inside a strip timeline view over `view`, which publishes the time axis the lane reads.
/// `show` builds the lane and shows it on every run of the view's body.
fn in_view<R>(ui: &mut Ui, view: &mut TimeView, mut show: impl FnMut(&mut Ui) -> R) -> StoryOutput
where
    StoryOutput: From<R>,
{
    let format = TimeFormat::timecode(FrameRate::FPS_24);
    TimelineView::new("Timeline", Bind::bound(view), format, seconds(12.0))
        .strip()
        .show(ui, |ui, _| show(ui))
        .inner
        .map(StoryOutput::from)
        .unwrap_or_default()
}

fn selected() -> Selection<KeyframeId> {
    Selection::new(vec![KeyframeId::new(2), KeyframeId::new(3)], Some(KeyframeId::new(3)), Some(KeyframeId::new(2)))
}

/// The playhead of the lane story, in seconds.
const PLAYHEAD: f64 = 4.0;

/// A property's keys as the timeline panel shows them: the stopwatch and the navigator on one line, as in the
/// headers, and the lane under them across the time area. The navigator's arrows move the playhead to the previous and
/// the next key.
#[story(KeyframeLane<'static>, covers = [Default])]
fn linear_hold_and_bezier_keys(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (keys, selection) = (keys(), selected());
    let mut playhead = *data.value("playhead", seconds(PLAYHEAD));
    let (before, after, at) =
        (keys.iter().any(|k| k.time < playhead), keys.iter().any(|k| k.time > playhead), keys.iter().any(|k| k.time == playhead));
    let start = start_view(ui);
    let mut view = *data.value("view", start);
    let mut animated = *data.value("animated", true);
    let shown = Stack::new()
        .show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                AnimationToggle::new(Bind::bound(&mut animated)).show(ui);
                let navigator = KeyframeNavigator::new("Keyframe navigator", before, after, at).animated(animated).show(ui);
                if navigator.previous {
                    playhead = keys.iter().rev().find(|k| k.time < playhead).map_or(playhead, |k| k.time);
                } else if navigator.next {
                    playhead = keys.iter().find(|k| k.time > playhead).map_or(playhead, |k| k.time);
                }
            });
            in_view(ui, &mut view, |ui| KeyframeLane::new("Keyframes", &keys, &selection).playhead(playhead).show(ui))
        })
        .inner
        .unwrap_or_default();
    *data.value("view", start) = view;
    *data.value("animated", true) = animated;
    *data.value("playhead", seconds(PLAYHEAD)) = playhead;
    shown
}

#[story(KeyframeNavigator, covers = [Default], drive = Drive::Never)]
fn navigator_between_keys(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let out = KeyframeNavigator::new("Keyframe navigator", true, true, false).show(ui);
    out.response.map(StoryOutput::from).unwrap_or_default()
}

#[story(AnimationToggle<'static>, covers = [Default])]
fn stopwatch(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    AnimationToggle::new(Bind::bound(data.value("animated", false))).show(ui).into()
}
