//! The suite's own rules: which states a type must show, and the runs a drag drive makes.

use craft_behavior::Mixed;
use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::testing::story::{Story, StoryData, StoryOutput};
use craft_ui::testing::suite::{measure_pure_case, required_states, values_case};
use craft_ui::testing::{Drive, Grip, Probe, Registration, Required, trial};
use craft_ui::text::TextField;
use craft_ui::value_drag::{Thumb, ThumbRow};
use egui::{Ui, vec2};

fn probe_of<T: 'static>() -> &'static Probe {
    let name = std::any::type_name::<T>();
    craft_ui::contract::registrations()
        .into_iter()
        .filter_map(Registration::probe)
        .find(|probe| probe.type_name() == name)
        .expect("the type is registered")
}

#[trial]
fn a_field_that_carries_a_value_needs_an_invalid_story() {
    assert!(required_states(probe_of::<TextField<'static>>()).contains(&Required::Invalid));
}

fn four_thumbs(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let thumbs = [Thumb::new("First"), Thumb::new("Second"), Thumb::new("Third"), Thumb::new("Fourth")];
    let value = data.value("levels", Mixed::each([30.0, 60.0, 190.0, 220.0]));
    ThumbRow::new(Bind::bound(value), 0.0..=255.0).thumbs(thumbs).label("Levels").show(ui).into()
}

#[trial]
fn a_drag_of_one_repeated_part_in_two_axes_also_runs_cancelled_by_escape() {
    let grip = Grip::part(Thumb::PART.at(2)).toward(vec2(-1.0, 1.0));
    let story = Story::new("third-thumb", four_thumbs).drive(Drive::Drag(grip));
    let runs = values_case(probe_of::<ThumbRow<'static, 4>>(), &story);
    assert_eq!(runs, Ok(vec!["drag", "drag with Escape"]));
}

fn counting_draws(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    *data.value("draws", 0u32) += 1;
    craft_ui::buttons::Button::new("Count").show(ui).into()
}

#[trial]
fn measure_pure_fails_a_story_that_counts_its_draws() {
    let probe = probe_of::<craft_ui::buttons::Button<'static>>();
    let failure = measure_pure_case(probe, &Story::new("draw-counter", counting_draws)).expect_err("counting draws is impure");
    assert!(failure.contains("draw-counter") && failure.contains("draws"), "{failure}");
}
