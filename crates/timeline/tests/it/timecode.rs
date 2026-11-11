//! The timecode field: a drag scrubs a field that is not in edit.

use craft_timeline::ui::TimecodeField;
use craft_timeline::{FrameRate, Tick, TimeFormat};
use craft_ui::contract::{Bind, Role, Show as _};
use craft_ui::testing::{Fixture, Story, StoryData, StoryOutput, trial};
use egui::{Ui, Vec2};

/// One and a half seconds: "00:00:01:15" at 30 frames per second.
fn start() -> Tick {
    Tick::from_seconds_f64(1.5)
}

fn field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    TimecodeField::new("Time", Bind::bound(data.value("time", start())), TimeFormat::timecode(FrameRate::FPS_30)).show(ui).edit.into()
}

fn fixture() -> Fixture {
    let mut fixture = Fixture::story(&Story::new("timecode", field));
    fixture.settle();
    fixture
}

fn time(fixture: &mut Fixture) -> Tick {
    *fixture.data().value("time", start())
}

#[trial]
fn a_drag_on_a_timecode_field_moves_the_time() {
    let mut fixture = fixture();
    let target = fixture.by_role(Role::SPIN_BUTTON).first().copied().expect("the field");
    fixture.drag(&target, Vec2::new(20.0, 0.0));
    fixture.settle();
    assert!(time(&mut fixture) > start(), "a drag to the right moved the time on");
}
