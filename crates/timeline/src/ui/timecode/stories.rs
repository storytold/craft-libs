//! Stories of the timecode field and the Timecode panel.

use super::TimecodePanelSettings;
use super::{TimecodeField, TimecodePanel, TimecodeRow, TimecodeRowKind, TimecodeSource, TimecodeText};
use crate::rates::FrameRate;
use crate::time::Tick;
use crate::timecode::{TimeDisplay, TimeFormat};
use craft_behavior::Mixed;
use craft_ui::contract::{Bind, Show as _, Validity};
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// 29.97 frames per second with drop-frame counting.
fn drop_frame() -> TimeFormat {
    TimeFormat::timecode(FrameRate::new(30_000, 1_001)).drop_frame(true)
}

/// Typed into a field whose time is after one o'clock: another time, in drop-frame timecode.
const TYPED_TIME: &str = "01:00:10;00";

/// The latest time the invalid story accepts, in seconds.
const LATEST_SECONDS: f64 = 3_600.0;

/// A time of some hundred thousand hours: its text is longer than the field's sample.
const LONG_SECONDS: f64 = 1.0e9;

#[story(TimecodeField<'static>, covers = [Default], drive = Drive::Type(TYPED_TIME))]
fn timecode_field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = Tick::from_seconds_f64(3_723.5);
    TimecodeField::new("Current time", Bind::bound(data.value("time", start)), drop_frame()).show(ui).edit.into()
}

#[story(TimecodeField<'static>, covers = [Mixed], drive = Drive::Never)]
fn timecode_field_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    TimecodeField::new("Current time", Bind::bound_mixed(data.value("mixed", Mixed::<Tick>::Mixed)), drop_frame()).show(ui).edit.into()
}

#[story(TimecodeField<'static>, covers = [Invalid], drive = Drive::Never)]
fn timecode_field_past_the_limit(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let before_the_limit = |time: &Tick| {
        if *time > Tick::from_seconds_f64(LATEST_SECONDS) { Validity::Refuse(Message::new("The time is past the limit")) } else { Validity::Ok }
    };
    let time = data.value("late", Tick::from_seconds_f64(LATEST_SECONDS + 60.0));
    TimecodeField::new("Current time", Bind::bound(time).check(before_the_limit), drop_frame()).show(ui).edit.into()
}

#[story(TimecodeField<'static>, covers = [LongText], drive = Drive::Never)]
fn timecode_field_long(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    TimecodeField::new("Current time", Bind::bound(data.value("long", Tick::from_seconds_f64(LONG_SECONDS))), drop_frame()).show(ui).edit.into()
}

#[story(TimecodeField<'static>, covers = [], drive = Drive::Type(TYPED_TIME))]
fn timecode_field_from_sequence_start(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = Tick::from_seconds_f64(3_600.0);
    TimecodeField::new("In point", Bind::bound(data.value("time", start)), drop_frame()).min(start).show(ui).edit.into()
}

#[story(TimecodePanel<'static>, covers = [Default], drive = Drive::Never)]
fn timecode_panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = vec![
        TimecodeRow::new(TimecodeRowKind::CurrentTime).source(TimecodeSource::Program),
        TimecodeRow::new(TimecodeRowKind::Duration).source(TimecodeSource::Program),
        TimecodeRow::new(TimecodeRowKind::Remaining).source(TimecodeSource::Program).display(Some(TimeDisplay::Frames)),
    ];
    let text = TimecodeText::new(|row| match row.kind {
        TimecodeRowKind::CurrentTime => "00:01:02:03".to_owned(),
        TimecodeRowKind::Duration => "00:05:00:00".to_owned(),
        _ => "17,940".to_owned(),
    });
    TimecodePanel::new("Timecode", Bind::bound(data.value("settings", TimecodePanelSettings::new(rows))), text).show(ui).into()
}
