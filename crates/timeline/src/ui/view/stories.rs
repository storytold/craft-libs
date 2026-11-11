//! Stories of `TimelineView`: the time area over a 30 s sequence with its bar.

use super::TimelineView;
use crate::rates::FrameRate;
use crate::time::{Tick, TimeView};
use crate::timecode::TimeFormat;
use craft_ui::contract::{Bind, ShowContainer as _, Validity};
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, pointer, story};
use egui::{Event, Modifiers, MouseWheelSource, MouseWheelUnit, TouchPhase, Ui, Vec2};

/// The sequence's end, in seconds.
const END_SECONDS: f64 = 30.0;
/// Points per second of the shown view.
const POINTS_PER_SECOND: f64 = 24.0;
/// The closest zoom the invalid story accepts, in points per second.
const CLOSEST_ZOOM: f64 = 100.0;
/// An Alt+wheel turn of this many points zooms the time area.
const ZOOM_TURN: f32 = 10.0;

fn format() -> TimeFormat {
    TimeFormat::timecode(FrameRate::FPS_29_97).drop_frame(true)
}

/// Zooms the time area under the pointer with Alt+wheel: a drive for the stories that show a time area.
pub(crate) fn zoom_the_time_area(fixture: &mut Fixture) {
    let at = fixture.shown().center();
    let turn = Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: Vec2::Y * ZOOM_TURN,
        phase: TouchPhase::Move,
        modifiers: Modifiers::ALT,
        source: MouseWheelSource::Wheel,
    };
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![turn]);
    fixture.idle(2);
}

#[story(TimelineView<'static>, covers = [Default], drive = Drive::Custom(zoom_the_time_area))]
fn timeline_view(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = data.value("view", TimeView::new(Tick::ZERO, POINTS_PER_SECOND));
    let out = TimelineView::new("Time", Bind::bound(view), format(), Tick::from_seconds_f64(END_SECONDS)).show(ui, |_, _| {});
    out.output.into()
}

#[story(TimelineView<'static>, covers = [Invalid], drive = Drive::Never)]
fn view_closer_than_the_limit(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = data.value("close", TimeView::new(Tick::ZERO, CLOSEST_ZOOM * 2.0));
    let not_too_close = |view: &TimeView| {
        if view.points_per_second > CLOSEST_ZOOM { Validity::Refuse(Message::new("The time area cannot zoom in that far")) } else { Validity::Ok }
    };
    let out = TimelineView::new("Time", Bind::bound(view).check(not_too_close), format(), Tick::from_seconds_f64(END_SECONDS)).show(ui, |_, _| {});
    out.output.into()
}
