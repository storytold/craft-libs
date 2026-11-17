//! The slider-based adjustment editors, driven through the public API as a host would show them: one binding per
//! editor, one gesture per slider drag, the Set of a check box, a preset or a button, and the requests the colour wells
//! file.

use std::cell::RefCell;
use std::rc::Rc;

use craft_imaging::ui::adjust::{AdjustSession, CmykChannelMixer, CmykChannelMixerEditor, HueRangeValue};
use craft_ui::contract::{Bind, Role};
use craft_ui::testing::{Fixture, Target, trial};
use egui::vec2;

// ---------------------------------------------------------------------------------------------
// Hue/Saturation

/// Dragging a range thumb across 0 degrees keeps the four bounds increasing.
#[trial]
fn range_thumbs_wrap_through_zero() {
    // The Magentas' end thumb dragged from 345 to 20 degrees.
    let moved = HueRangeValue::move_bound(HueRangeValue::neutral(5).bounds, 3, 20.0);
    assert_eq!(moved, [255.0, 285.0, 315.0, 380.0]);
    // The Reds' start thumb dragged from 315 (-45) to 340 degrees, past the second thumb's room.
    let moved = HueRangeValue::move_bound(HueRangeValue::neutral(0).bounds, 0, 340.0);
    assert_eq!(moved, [-20.0, -15.0, 15.0, 45.0]);
    assert!(moved.windows(2).all(|pair| pair[0] <= pair[1]));
}

// ---------------------------------------------------------------------------------------------
// The slider table and the session

/// Tracked, edited, Reset: the start settings; Reset again: the defaults; a pass without tracking or another target
/// takes the committed settings anew.
#[trial]
fn reset_returns_to_the_values_found_when_edited_else_defaults() {
    let mut session = AdjustSession::new();
    session.track(7, 1, &10);
    session.track(7, 2, &20);
    assert_eq!(session.start(), None, "nothing is offered before an edit");
    session.edited();
    session.track(7, 3, &30);
    assert_eq!(session.start(), Some(&10), "following passes keep the settings found first");
    assert_eq!(session.reset(|| 0), 10, "edited: the start settings");
    assert!(!session.is_edited());
    assert_eq!(session.reset(|| 0), 0, "a second Reset gives the defaults");

    session.edited();
    session.track(7, 9, &90);
    assert!(!session.is_edited());
    session.edited();
    assert_eq!(session.start(), Some(&90));

    session.track(8, 10, &5);
    assert!(!session.is_edited());
    session.edited();
    assert_eq!(session.start(), Some(&5));
}

// ---------------------------------------------------------------------------------------------
// Channel Mixer of a CMYK document

/// A CMYK Channel Mixer editor over `start`, and the value it holds after each pass.
fn cmyk_mixer(start: CmykChannelMixer) -> (Fixture, Rc<RefCell<CmykChannelMixer>>) {
    let value = Rc::new(RefCell::new(start));
    let shown = Rc::clone(&value);
    let mut fixture = Fixture::new(move |ui| {
        let mut held = shown.borrow_mut();
        let _ = CmykChannelMixerEditor::new("Channel Mixer", Bind::bound(&mut *held)).show(ui);
    });
    fixture.settle();
    (fixture, value)
}

/// The editor's sliders in order: Cyan, Magenta, Yellow, Black, Constant.
fn sliders(fixture: &Fixture) -> Vec<Target> {
    fixture.by_role(Role::SLIDER)
}

/// The shown output is Cyan; each of its five sliders edits its own place in the Cyan row and nothing else.
#[trial]
fn cmyk_mixer_sliders_edit_the_shown_ink_row() {
    let (mut fixture, value) = cmyk_mixer(CmykChannelMixer::default());
    let rows = sliders(&fixture);
    assert_eq!(rows.len(), 5, "four ink sources and Constant");
    let start = CmykChannelMixer::default();
    for (i, slider) in rows.iter().enumerate() {
        let before = *value.borrow();
        fixture.drag(slider, vec2(-slider.rect.width() / 8.0, 0.0)).settle();
        let after = *value.borrow();
        assert_ne!(after.cyan[i], before.cyan[i], "slider {i} moves its place in the Cyan row");
        assert_eq!(after.cyan[i], after.cyan[i].round(), "slider {i} ends on a whole percent");
        let others: Vec<usize> = (0..5).filter(|j| *j != i).collect();
        assert!(others.iter().all(|j| after.cyan[*j] == before.cyan[*j]), "slider {i} leaves the rest of the row: {after:?}");
        assert_eq!((after.magenta, after.yellow, after.black, after.gray), (start.magenta, start.yellow, start.black, None));
    }
}

/// Monochrome cannot be turned on; an imported one shows its Gray row disabled and only turns off, keeping the inks.
#[trial]
fn cmyk_mixer_monochrome_only_turns_off() {
    let (mut fixture, value) = cmyk_mixer(CmykChannelMixer::default());
    let monochrome = fixture.by_label("Monochrome").expect("the Monochrome box");
    assert!(fixture.tree().is_disabled_key(monochrome.node), "Monochrome is off and cannot be turned on");
    fixture.click(&monochrome).settle();
    assert_eq!(*value.borrow(), CmykChannelMixer::default(), "a click on the disabled box changes nothing");

    let inks = CmykChannelMixer::new(
        [80.0, 10.0, 0.0, 0.0, 5.0],
        [0.0, 100.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 90.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 100.0, 0.0],
        None,
    );
    let imported = CmykChannelMixer { gray: Some([30.0, 30.0, 30.0, 10.0, 0.0]), ..inks };
    let (mut fixture, value) = cmyk_mixer(imported);
    let rows = sliders(&fixture);
    assert_eq!(rows.len(), 5);
    assert!(rows.iter().all(|slider| fixture.tree().is_disabled_key(slider.node)), "Gray's sources and Constant are disabled");
    let first = rows[0];
    fixture.drag(&first, vec2(first.rect.width() / 4.0, 0.0)).settle();
    assert_eq!(*value.borrow(), imported, "a disabled row takes no edit");
    let monochrome = fixture.by_label("Monochrome").expect("the Monochrome box");
    assert!(!fixture.tree().is_disabled_key(monochrome.node), "an imported Monochrome can be turned off");
    fixture.click(&monochrome).settle();
    assert_eq!(*value.borrow(), inks, "turning Monochrome off drops the Gray row and keeps the ink rows");
    let rows = sliders(&fixture);
    assert!(rows.iter().all(|slider| !fixture.tree().is_disabled_key(slider.node)), "the ink rows take edits again");
}
