//! Stories of the swatch pair.

use super::{PairWell, SwatchPair, Well};
use crate::ui::{GradientPreview, Paint};
use craft_behavior::Mixed;
use craft_theme::Srgba;
use craft_ui::contract::{Bind, Role, Show as _, ShowContainer as _, Tracked, Validity};
use craft_ui::layout_components::Row;
use craft_ui::rails::{LastUsed, ToolRail, ToolSlot};
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
    let (state, state_at) = data.value_at("colors", Colors { active: Well::Primary, colors: start });
    let wells = [PairWell::new(names[0], state.colors[0]), PairWell::new(names[1], state.colors[1]).ring(ring)];
    let shown = SwatchPair::<()>::new("Colours", wells, state_at.bind(&mut state.active))
        .swap_button("Swap colours")
        .default_button("Default colours")
        .show(ui);
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

#[story(SwatchPair<'static, u8>, covers = [Default], drive = Drive::Custom(next_well))]
fn swatch_pair(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let black_white = [Srgba::rgb(0.0, 0.0, 0.0), Srgba::rgb(1.0, 1.0, 1.0)];
    editing(ui, data, ["Foreground colour", "Background colour"], black_white, false)
}

#[story(SwatchPair<'static, u8>, covers = [], drive = Drive::Never)]
fn swatch_pair_ring(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    editing(ui, data, ["Fill colour", "Stroke colour"], [Srgba::rgb(0.2, 0.55, 0.8), Srgba::rgb(0.9, 0.35, 0.2)], true)
}

#[story(SwatchPair<'static, u8>, covers = [], drive = Drive::Never)]
fn swatch_pair_host_picker(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    SwatchPair::<()>::new("Colours", wells(false), data.bind("active", Well::Primary)).popover(false).click_activates(false).show(ui).into()
}

#[story(SwatchPair<'static, u8>, covers = [Mixed], drive = Drive::Never)]
fn swatch_pair_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let wells = [PairWell::new("Fill", Mixed::<Srgba>::Mixed), PairWell::new("Stroke", Srgba::rgb(0.1, 0.3, 0.8)).ring(true)];
    SwatchPair::<()>::new("Fill and stroke", wells, data.bind("active", Well::Primary)).show(ui).into()
}

/// Opens the picker of the focused well with Enter.
fn open_picker(fixture: &mut Fixture) {
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.idle(4);
}

#[story(SwatchPair<'static, u8>, covers = [Open], drive = Drive::Custom(open_picker))]
fn swatch_pair_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    SwatchPair::<()>::new("Colours", wells(false), data.bind("active", Well::Primary)).show(ui).into()
}

#[story(SwatchPair<'static, u8>, covers = [Invalid], drive = Drive::Never)]
fn swatch_pair_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_secondary =
        |well: &Well| if *well == Well::Secondary { Validity::Refuse(Message::new("Only the foreground can be edited")) } else { Validity::Ok };
    SwatchPair::<()>::new("Colours", wells(false), data.bind("active", Well::Secondary).check(refuse_secondary)).show(ui).into()
}

/// The pair in a tool rail's footer, as PhotoCraft's colour chips: the gap round the front well takes the rail's chrome.
#[story(SwatchPair<'static, u8>, covers = [], drive = Drive::Never)]
fn swatch_pair_tool_rail(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (active, active_at) = data.value_at("active", Well::Primary);
    let (slots, last): (Tracked<Vec<ToolSlot<u8>>>, Tracked<LastUsed<u8>>) = (Tracked::default(), Tracked::default());
    let wells = [PairWell::new("Foreground colour", Srgba::rgb(0.0, 0.0, 0.0)), PairWell::new("Background colour", Srgba::rgb(1.0, 1.0, 1.0))];
    let mut shown = None;
    ToolRail::new("Tools", &slots, &0, &last).show_rail(ui, |ui, _| {
        shown = Some(
            SwatchPair::<()>::new("Colours", wells.clone(), active_at.bind(&mut *active))
                .popover(false)
                .swap_button("Swap colours")
                .default_button("Default colours")
                .show(ui),
        );
    });
    shown.map(StoryOutput::from).unwrap_or_default()
}

/// Wells that show a gradient and No Color, as squares and as rings.
#[story(SwatchPair<'static, u8>, covers = [], drive = Drive::Never)]
fn swatch_pair_paints(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let ramp = || Paint::Gradient(GradientPreview::new(vec![(0.0, Srgba::rgb(0.9, 0.3, 0.2)), (1.0, Srgba::rgb(0.2, 0.4, 0.9))]));
    Row::new().show(ui, |ui, _| {
        for ring in [false, true] {
            let wells = [PairWell::paint("Fill", ramp()).ring(ring), PairWell::paint("Stroke", Paint::None).ring(ring)];
            let name = if ring { "Ring paints" } else { "Paints" };
            let _ = SwatchPair::<()>::new(name, wells, Bind::given(Well::Primary)).show(ui);
        }
    });
    StoryOutput::default()
}

/// Drags the first pair's primary well onto the second pair's.
fn drag_across(fixture: &mut Fixture) {
    let wells = fixture.by_role(Role::RADIO_BUTTON);
    let (from, to) = (wells[0].rect.center(), wells[2].rect.center());
    fixture.drag_at(from, to - from);
}

/// Two pairs whose wells drag a payload and take the other pair's.
#[story(SwatchPair<'static, u8>, covers = [], drive = Drive::Custom(drag_across))]
fn swatch_pair_drags(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let dropped = data.note("dropped", String::new());
    Row::new().show(ui, |ui, _| {
        for first in [1u8, 3] {
            let wells = [PairWell::new("Source", Srgba::rgb(0.1, 0.3, 0.8)), PairWell::new("Target", Srgba::rgb(0.9, 0.35, 0.2))];
            let shown = SwatchPair::<()>::new(format!("Pair {first}"), wells, Bind::given(Well::Primary))
                .drags([Some(first), Some(first + 1)])
                .accepts(|_, _| true)
                .show(ui);
            if let Some((well, payload)) = shown.dropped {
                *dropped = format!("{payload} on {well:?}");
            }
        }
    });
    let readout = format!("Dropped: {dropped}");
    StoryOutput::default().readout(readout)
}
