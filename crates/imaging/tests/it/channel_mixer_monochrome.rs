//! Channel Mixer's Monochrome check box (photocraft#14412): unticking it hands the Gray mix to the Red, Green and Blue
//! outputs, so the image stays gray until an output is edited.

use std::cell::RefCell;
use std::rc::Rc;

use craft_imaging::ui::adjust::{ChannelMixer, ChannelMixerEditor};
use craft_ui::contract::{Bind, Role};
use craft_ui::testing::{Fixture, trial};
use egui::vec2;

/// The Yellow Filter black-and-white mix the reference was measured with: Red 34, Green 66, Blue 0, Constant 0.
const YELLOW_FILTER: [f64; 4] = [34.0, 66.0, 0.0, 0.0];

/// The Channel Mixer editor over `start`, with the value its host holds.
fn mixer(start: ChannelMixer) -> (Fixture, Rc<RefCell<ChannelMixer>>) {
    let host = Rc::new(RefCell::new(start));
    let held = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        let mut value = held.borrow_mut();
        let _ = ChannelMixerEditor::new("Channel Mixer", Bind::bound(&mut *value)).show(ui);
    });
    fixture.room(vec2(400.0, 520.0));
    fixture.settle();
    (fixture, host)
}

/// Clicks the Monochrome check box.
fn toggle_monochrome(fixture: &mut Fixture) {
    let check = fixture
        .by_role(Role::CHECK_BOX)
        .into_iter()
        .find(|target| fixture.tree().name_of(target.node) == Some("Monochrome"))
        .expect("the Monochrome check box");
    fixture.click(&check).settle();
}

/// The Red, Green, Blue and Constant sliders' values as the dialog shows them.
fn shown_sources(fixture: &Fixture) -> Vec<(String, f64)> {
    let tree = fixture.tree();
    fixture
        .by_role(Role::SLIDER)
        .iter()
        .filter_map(|slider| {
            let name = tree.name_of(slider.node)?.to_owned();
            let value = tree.numeric_of(slider.node)?.value?;
            Some((name, value))
        })
        .collect()
}

/// Picks `output` in the Output Channel dropdown.
fn pick_output(fixture: &mut Fixture, output: &str) {
    let dropdown = fixture.by_role(Role::COMBO_BOX).into_iter().next().expect("the Output Channel dropdown");
    fixture.click(&dropdown).settle();
    let entry = [Role::LIST_BOX_OPTION, Role::MENU_ITEM_RADIO, Role::MENU_ITEM]
        .into_iter()
        .flat_map(|role| fixture.by_role(role))
        .find(|target| fixture.tree().name_of(target.node) == Some(output))
        .unwrap_or_else(|| panic!("the {output} entry"));
    fixture.click(&entry).settle();
}

/// The shown sources of the Yellow Filter mix.
fn yellow_filter_shown() -> Vec<(String, f64)> {
    ["Red", "Green", "Blue", "Constant"].iter().zip(YELLOW_FILTER).map(|(name, value)| ((*name).to_owned(), value)).collect()
}

/// Monochrome off from a gray mix of 34 / 66 / 0: each output takes 34 / 66 / 0, Constant 0, and the dialog shows it
/// for every output channel.
#[trial]
fn monochrome_off_gives_each_output_the_gray_mix() {
    let (mut fixture, host) = mixer(ChannelMixer { gray: Some(YELLOW_FILTER), ..ChannelMixer::default() });
    toggle_monochrome(&mut fixture);
    assert_eq!(*host.borrow(), ChannelMixer::new(YELLOW_FILTER, YELLOW_FILTER, YELLOW_FILTER, None), "the gray mix on every output");
    for output in ["Red", "Green", "Blue"] {
        if output != "Red" {
            pick_output(&mut fixture, output);
        }
        assert_eq!(shown_sources(&fixture), yellow_filter_shown(), "the {output} output shows the gray mix");
    }
}

/// Monochrome on from Default starts the Gray row at 40 / 40 / 20 and leaves the colour outputs as they were.
#[trial]
fn monochrome_on_from_default_starts_at_40_40_20() {
    let (mut fixture, host) = mixer(ChannelMixer::default());
    toggle_monochrome(&mut fixture);
    assert_eq!(*host.borrow(), ChannelMixer { gray: Some([40.0, 40.0, 20.0, 0.0]), ..ChannelMixer::default() });
}
