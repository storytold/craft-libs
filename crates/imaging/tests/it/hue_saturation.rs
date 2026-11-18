//! Hue/Saturation's range swatches: the seven swatches pick the range, a range shows its centre, and Colorize turns
//! the row into one swatch.

use craft_imaging::ui::adjust::{HueRangeValue, HueSaturation, HueSaturationEditor};
use craft_ui::contract::Bind;
use craft_ui::testing::{Fixture, trial};
use egui::Vec2;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct Rig {
    fixture: Fixture,
    shown: Rc<Cell<Option<usize>>>,
}

/// The editor of `start` with the range swatches, keeping the range it reports.
fn rig(start: HueSaturation, swatches: bool) -> Rig {
    let value = Rc::new(RefCell::new(start));
    let shown = Rc::new(Cell::new(None));
    let (v, s) = (value.clone(), shown.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut now = *v.borrow();
        let mut editor = HueSaturationEditor::new("Hue/Saturation", Bind::bound(&mut now)).range(s.get());
        if swatches {
            editor = editor.range_swatches();
        }
        s.set(editor.show(ui).shown_range);
        *v.borrow_mut() = now;
    });
    fixture.room(Vec2::new(480.0, 900.0)).idle(20);
    Rig { fixture, shown }
}

fn shows(fixture: &Fixture, text: &str) -> bool {
    fixture.texts().iter().any(|painted| painted.text == text)
}

const NAMES: [&str; 7] = ["Master", "Reds", "Yellows", "Greens", "Cyans", "Blues", "Magentas"];

#[trial]
fn seven_swatches_pick_the_range_and_a_range_shows_its_centre() {
    let mut rig = rig(HueSaturation::default(), true);
    let lefts: Vec<f32> = NAMES.iter().map(|name| rig.fixture.by_label(name).unwrap_or_else(|| panic!("a {name} swatch")).rect.left()).collect();
    assert!(lefts.windows(2).all(|pair| pair[0] < pair[1]), "Master, then Reds … Magentas, left to right: {lefts:?}");
    assert!(!shows(&rig.fixture, "0°"), "Master has no centre");
    let blues = rig.fixture.by_label("Blues").expect("the Blues swatch");
    rig.fixture.click(&blues).idle(10);
    assert_eq!(rig.shown.get(), Some(4), "Blues is range 4");
    assert!(shows(&rig.fixture, "240°"), "the Blues centre");
    let reds = rig.fixture.by_label("Reds").expect("the Reds swatch");
    rig.fixture.click(&reds).idle(10);
    assert_eq!(rig.shown.get(), Some(0));
    assert!(shows(&rig.fixture, "0°"), "the Reds centre");
}

#[trial]
fn a_shifted_range_shows_both_centres() {
    let mut start = HueSaturation::default();
    start.ranges[0] = HueRangeValue { hue: 30.0, ..HueRangeValue::neutral(0) };
    let mut rig = rig(start, true);
    let reds = rig.fixture.by_label("Reds").expect("the Reds swatch");
    rig.fixture.click(&reds).idle(10);
    assert!(shows(&rig.fixture, "0° / 30°"));
}

#[trial]
fn colorize_turns_the_row_into_one_swatch() {
    let rig = rig(HueSaturation { hue: 210.0, saturation: 40.0, colorize: true, ..HueSaturation::default() }, true);
    for name in NAMES {
        assert!(rig.fixture.by_label(name).is_none(), "no {name} swatch while Colorize is on");
    }
}

#[trial]
fn without_the_option_the_range_is_a_select() {
    let rig = rig(HueSaturation::default(), false);
    assert!(rig.fixture.by_label("Blues").is_none(), "no swatches; the select lists the ranges when opened");
    assert!(rig.fixture.by_label("Range").is_some(), "the Range select");
}
