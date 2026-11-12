//! The number keys of a tool's opacity: a digit key fires its `OpacityDigit` command, which sets the opacity, and
//! Shift with it the second value.

use craft_behavior::DigitKeys;
use craft_ui::canvas::OpacityDigit;
use craft_ui::runtime::frame::take_output;
use craft_ui::testing::input::key_press;
use craft_ui::testing::{Frames, trial};
use egui::{Key, Modifiers, Ui};

#[trial]
fn a_digit_key_sets_the_opacity_and_shift_with_it_the_second_value() {
    let mut show = |_: &mut Ui| {};
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    let mut keys = DigitKeys::default();
    let mut press = |frames: &mut Frames, key: Key, modifiers: Modifiers| {
        frames.frame(vec![key_press(key, modifiers)], &mut show);
        let now = frames.ctx.input(|input| input.time);
        let entries: Vec<_> = take_output(&frames.ctx).fired.iter().filter_map(|fired| OpacityDigit::entry(&mut keys, fired, now)).collect();
        assert_eq!(entries.len(), 1, "{key:?} with {modifiers:?} makes one entry");
        (entries[0].percent, entries[0].second)
    };
    assert_eq!(press(&mut frames, Key::Num4, Modifiers::NONE), (40.0, false));
    assert_eq!(press(&mut frames, Key::Num5, Modifiers::NONE), (45.0, false), "a second digit at once makes the exact value");
    assert_eq!(press(&mut frames, Key::Num0, Modifiers::SHIFT), (100.0, true));
}
