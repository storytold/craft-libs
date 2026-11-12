//! Stories of the swatch pair.

use super::{PairWell, SwatchPair, Well};
use craft_behavior::Mixed;
use craft_theme::Srgba;
use craft_ui::contract::{Bind, Show as _, Validity};
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::{Key, Modifiers, Ui};

/// Tab arrives at the active well; the arrow moves to the other well and makes it active.
fn next_well(fixture: &mut Fixture) {
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::ArrowRight, Modifiers::NONE);
    fixture.idle(2);
}

fn wells(ring: bool) -> [PairWell; 2] {
    [
        PairWell::new("Foreground colour", Srgba::rgb(0.1, 0.3, 0.8)),
        PairWell::new("Background colour", Srgba { r: 1.0, g: 1.0, b: 1.0, a: 0.7 }).ring(ring),
    ]
}

/// The host's side of a pair: the active well and the two colours.
#[derive(Clone, Debug, PartialEq)]
struct Colors {
    active: Well,
    colors: [Srgba; 2],
}

/// A pair editing two colours as a host does: swap exchanges them, the default button sets black and white, and the
/// popup's picker edits the well it opened on. With `ring` the secondary well is a ring, as DesignCraft's stroke.
fn editing(ui: &mut Ui, data: &mut StoryData, names: [&'static str; 2], start: [Srgba; 2], ring: bool) -> StoryOutput {
    let defaults = [Srgba::rgb(0.0, 0.0, 0.0), Srgba::rgb(1.0, 1.0, 1.0)];
    let state = data.value("colors", Colors { active: Well::Primary, colors: start });
    let wells = [PairWell::new(names[0], state.colors[0]), PairWell::new(names[1], state.colors[1]).ring(ring)];
    let shown =
        SwatchPair::new("Colours", wells, Bind::bound(&mut state.active)).swap_button("Swap colours").default_button("Default colours").show(ui);
    if shown.swap {
        state.colors.swap(0, 1);
    }
    if shown.defaults {
        state.colors = defaults;
    }
    if let Some(picked) = shown.events.value() {
        state.colors[shown.well as usize] = picked.color;
    }
    let active = names[state.active as usize];
    let last = data.note("last", String::new());
    if let Some(event) = shown.events.last() {
        *last = format!("{:?}", event.clone().map(|picked| picked.color.to_hex()));
    }
    let readout = format!("Active: {active}\nLast event: {last}");
    StoryOutput::from(shown).readout(readout)
}

#[story(SwatchPair<'static>, covers = [Default], drive = Drive::Custom(next_well))]
fn swatch_pair(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let black_white = [Srgba::rgb(0.0, 0.0, 0.0), Srgba::rgb(1.0, 1.0, 1.0)];
    editing(ui, data, ["Foreground colour", "Background colour"], black_white, false)
}

#[story(SwatchPair<'static>, covers = [], drive = Drive::Never)]
fn swatch_pair_ring(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    editing(ui, data, ["Fill colour", "Stroke colour"], [Srgba::rgb(0.2, 0.55, 0.8), Srgba::rgb(0.9, 0.35, 0.2)], true)
}

#[story(SwatchPair<'static>, covers = [], drive = Drive::Never)]
fn swatch_pair_host_picker(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    SwatchPair::new("Colours", wells(false), Bind::bound(data.value("active", Well::Primary))).popover(false).click_activates(false).show(ui).into()
}

#[story(SwatchPair<'static>, covers = [Mixed], drive = Drive::Never)]
fn swatch_pair_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let wells = [PairWell::new("Fill", Mixed::<Srgba>::Mixed), PairWell::new("Stroke", Srgba::rgb(0.1, 0.3, 0.8)).ring(true)];
    SwatchPair::new("Fill and stroke", wells, Bind::bound(data.value("active", Well::Primary))).show(ui).into()
}

/// Opens the picker of the focused well with Enter.
fn open_picker(fixture: &mut Fixture) {
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.idle(4);
}

#[story(SwatchPair<'static>, covers = [Open], drive = Drive::Custom(open_picker))]
fn swatch_pair_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    SwatchPair::new("Colours", wells(false), Bind::bound(data.value("active", Well::Primary))).show(ui).into()
}

#[story(SwatchPair<'static>, covers = [Invalid], drive = Drive::Never)]
fn swatch_pair_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_secondary =
        |well: &Well| if *well == Well::Secondary { Validity::Refuse(Message::new("Only the foreground can be edited")) } else { Validity::Ok };
    SwatchPair::new("Colours", wells(false), Bind::bound(data.value("active", Well::Secondary)).check(refuse_secondary)).show(ui).into()
}
