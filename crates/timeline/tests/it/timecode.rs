//! The timecode field: a drag scrubs a field that is not in edit, and the arrows on the selected text start from its
//! end. The Timecode panel: each row's menu opens from that row and acts on it.

use craft_timeline::ui::{TimecodeField, TimecodePanel, TimecodePanelSettings, TimecodeRow, TimecodeRowKind, TimecodeSource, TimecodeText};
use craft_timeline::{FrameRate, Tick, TimeFormat};
use craft_ui::contract::{Bind, Role, Show as _};
use craft_ui::testing::{Fixture, Story, StoryData, StoryOutput, pointer, trial};
use egui::{Key, Modifiers, Ui, Vec2};

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

#[trial]
fn left_on_the_selected_timecode_starts_from_its_last_digit() {
    let mut fixture = fixture();
    let target = fixture.by_role(Role::SPIN_BUTTON).first().copied().expect("the field");
    fixture.click(&target);
    fixture.settle();
    fixture.key(Key::ArrowLeft, Modifiers::NONE);
    fixture.key(Key::Backspace, Modifiers::NONE);
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    let edited = time(&mut fixture);
    assert!(edited != start() && edited > Tick::from_seconds_f64(1.0), "Backspace took a digit of the frames, not nothing at the start");
}

fn rows() -> TimecodePanelSettings {
    TimecodePanelSettings::new(
        [TimecodeRowKind::CurrentTime, TimecodeRowKind::Duration, TimecodeRowKind::Remaining]
            .map(|kind| TimecodeRow::new(kind).source(TimecodeSource::Program))
            .to_vec(),
    )
}

fn panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let text = TimecodeText::new(|row| format!("{:?}", row.kind));
    TimecodePanel::new("Timecode", Bind::bound(data.value("settings", rows())), text).show(ui).into()
}

fn kinds(fixture: &mut Fixture) -> Vec<TimecodeRowKind> {
    fixture.data().value("settings", rows()).rows.iter().map(|row| row.kind).collect()
}

fn open_menu_by_key(fixture: &mut Fixture, readout: &str, key: Key, modifiers: Modifiers) {
    let target = fixture.by_label(readout).expect("the readout");
    fixture.click(&target);
    fixture.settle();
    fixture.key(key, modifiers);
    fixture.settle();
}

#[trial]
fn a_rows_menu_opens_from_a_secondary_click_or_the_menu_keys_and_acts_on_that_row() {
    use TimecodeRowKind::{CurrentTime, Duration, MediaTime, Remaining};
    let mut fixture = Fixture::story(&Story::new("timecode panel", panel));
    fixture.settle();
    let second = "Duration \u{b7} Program: Duration";
    let target = fixture.by_label(second).expect("the second readout");
    fixture.click(&target);
    fixture.settle();
    assert!(fixture.by_label("Remove Row").is_none(), "a primary click opens no menu");
    assert_eq!(kinds(&mut fixture), [CurrentTime, Duration, Remaining], "a primary click changes nothing");

    let at = target.center();
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::secondary(at, true)]);
    fixture.frame(vec![pointer::secondary(at, false)]);
    fixture.settle();
    let remove = fixture.by_label("Remove Row").expect("a secondary click on the row opens its menu");
    fixture.click(&remove);
    fixture.settle();
    assert_eq!(kinds(&mut fixture), [CurrentTime, Remaining], "Remove Row removed the row the menu opened from");

    open_menu_by_key(&mut fixture, "Remaining \u{b7} Program: Remaining", Key::F10, Modifiers::SHIFT);
    let media = fixture.by_label("Media Time").expect("Shift+F10 on the focused row opens its menu");
    fixture.click(&media);
    fixture.settle();
    assert_eq!(kinds(&mut fixture), [CurrentTime, MediaTime], "the menu Shift+F10 opened acted on its row");

    open_menu_by_key(&mut fixture, "Current Time \u{b7} Program: CurrentTime", Key::Menu, Modifiers::NONE);
    let duration = fixture.by_label("Duration").expect("the Menu key on the focused row opens its menu");
    fixture.click(&duration);
    fixture.settle();
    assert_eq!(kinds(&mut fixture), [Duration, MediaTime], "the menu the Menu key opened acted on its row");
}
