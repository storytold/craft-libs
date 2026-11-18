//! The slider-based adjustment editors, driven through the public API as a host would show them: one binding per
//! editor, one gesture per slider drag, the Set of a check box, a preset or a button, and the requests the colour wells
//! file.

use std::cell::RefCell;
use std::rc::Rc;

use craft_imaging::ui::adjust::{AdjustSession, CmykChannelMixer, CmykChannelMixerEditor, HueRangeValue};
use craft_ui::contract::{Bind, Role};
use craft_ui::testing::{Fixture, Target, access, trial};
use egui::{Key, Modifiers, vec2};

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
    cmyk_mixer_with(start, None)
}

/// [`cmyk_mixer`], with `monochrome_tip` as the reason the Monochrome box gives.
fn cmyk_mixer_with(start: CmykChannelMixer, monochrome_tip: Option<&'static str>) -> (Fixture, Rc<RefCell<CmykChannelMixer>>) {
    let value = Rc::new(RefCell::new(start));
    let shown = Rc::clone(&value);
    let mut fixture = Fixture::new(move |ui| {
        let mut held = shown.borrow_mut();
        let mut editor = CmykChannelMixerEditor::new("Channel Mixer", Bind::bound(&mut *held));
        if let Some(tip) = monochrome_tip {
            editor = editor.monochrome_tip(tip);
        }
        let _ = editor.show(ui);
    });
    fixture.settle();
    (fixture, value)
}

/// The editor's sliders in order: Cyan, Magenta, Yellow, Black, Constant.
fn sliders(fixture: &Fixture) -> Vec<Target> {
    fixture.by_role(Role::SLIDER)
}

/// Picks the output `name` in the Output Channel select.
fn choose_output(fixture: &mut Fixture, name: &str) {
    let select = fixture.by_role(Role::COMBO_BOX).first().copied().expect("the Output Channel select");
    fixture.click(&select).settle();
    // The sliders carry the ink names too, so the entry is looked up among the select's options.
    let tree = fixture.tree();
    let option = [Role::LIST_BOX_OPTION, Role::MENU_ITEM, Role::MENU_ITEM_RADIO, Role::LIST_ITEM]
        .into_iter()
        .flat_map(|role| fixture.by_role(role))
        .find(|option| tree.name_of(option.node) == Some(name))
        .unwrap_or_else(|| panic!("no {name} entry in the Output Channel select"));
    fixture.click(&option).settle();
}

/// The row of output `out` (Cyan, Magenta, Yellow, Black).
fn ink_row(value: &CmykChannelMixer, out: usize) -> [f64; 5] {
    [value.cyan, value.magenta, value.yellow, value.black][out]
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
    assert_eq!(rows.len(), 5, "the four ink sources and Constant show again");
    assert!(rows.iter().all(|slider| !fixture.tree().is_disabled_key(slider.node)), "the ink rows take edits again");
}

/// Picking another output moves the sliders to that ink's row: an edit changes that row and leaves the others.
#[trial]
fn cmyk_mixer_output_channel_picks_the_row_the_sliders_edit() {
    let (mut fixture, value) = cmyk_mixer(CmykChannelMixer::default());
    for (out, name) in [(1, "Magenta"), (2, "Yellow"), (3, "Black")] {
        choose_output(&mut fixture, name);
        let rows = sliders(&fixture);
        assert_eq!(rows.len(), 5, "{name}: four ink sources and Constant");
        let before = *value.borrow();
        // The Cyan source of the picked output, at 0 in its identity row.
        let source = rows[0];
        fixture.drag(&source, vec2(source.rect.width() / 8.0, 0.0)).settle();
        let after = *value.borrow();
        assert_ne!(ink_row(&after, out)[0], ink_row(&before, out)[0], "{name}'s Cyan source moves");
        for other in (0..4).filter(|other| *other != out) {
            assert_eq!(ink_row(&after, other), ink_row(&before, other), "an edit of {name} leaves output {other}");
        }
    }
    assert_eq!(value.borrow().cyan, CmykChannelMixer::default().cyan, "the Cyan row was never edited");
}

/// An edit of one source keeps the fractions of the rest of its row (an imported value need not be whole percent).
#[trial]
fn cmyk_mixer_edit_keeps_the_fractions_of_the_rest_of_the_row() {
    let start = CmykChannelMixer { cyan: [80.0, 19.5, 0.0, 0.0, 0.5], ..CmykChannelMixer::default() };
    let (mut fixture, value) = cmyk_mixer(start);
    let cyan = sliders(&fixture)[0];
    fixture.drag(&cyan, vec2(-cyan.rect.width() / 8.0, 0.0)).settle();
    let after = value.borrow().cyan;
    assert_ne!(after[0], 80.0, "the Cyan source moves");
    assert_eq!(after[0], after[0].round(), "the edited source ends on a whole percent");
    assert_eq!(after[1..], start.cyan[1..], "the untouched sources and Constant keep their fractions");
}

/// A Total over 100 % that whole percent would show as 100 % shows its hundredths, so the readout agrees with its
/// warning; a Total of exactly 100 % does not warn.
#[trial]
fn cmyk_mixer_total_shows_the_excess_it_warns_of() {
    let painted = |fixture: &Fixture, part: &str| fixture.texts().iter().any(|run| run.text.contains(part));
    let (fixture, _) = cmyk_mixer(CmykChannelMixer { cyan: [80.25, 20.0, 0.0, 0.0, 0.0], ..CmykChannelMixer::default() });
    assert!(painted(&fixture, "over 100 percent"), "a Total of 100.25 % warns");
    let texts: Vec<&str> = fixture.texts().iter().map(|run| run.text.as_str()).collect();
    assert!(painted(&fixture, "100.25"), "the readout shows the excess it warns of: {texts:?}");
    let (fixture, _) = cmyk_mixer(CmykChannelMixer::default());
    assert!(!painted(&fixture, "over 100 percent"), "a Total of exactly 100 % does not warn");
    // 67.89 + 28.35 + 3.76 adds up to a hair over 100 in floating point.
    let (fixture, _) = cmyk_mixer(CmykChannelMixer { cyan: [67.89, 28.35, 3.76, 0.0, 0.0], ..CmykChannelMixer::default() });
    assert!(!painted(&fixture, "over 100 percent"), "sources that add up to 100 % to the hundredth do not warn");
}

/// Given a reason, the disabled Monochrome box stays a Tab stop, so the keyboard reaches the reason it gives.
#[trial]
fn cmyk_mixer_monochrome_reason_reaches_the_keyboard() {
    let (mut fixture, _) = cmyk_mixer_with(CmykChannelMixer::default(), Some("A CMYK document has no Monochrome"));
    let monochrome = fixture.by_label("Monochrome").expect("the Monochrome box");
    assert!(fixture.tree().is_disabled_key(monochrome.node), "Monochrome is off and cannot be turned on");
    let reached = (0..24).any(|_| {
        fixture.key(Key::Tab, Modifiers::NONE).settle();
        access::focused_key(fixture.tree()) == monochrome.node
    });
    assert!(reached, "Tab reaches the disabled box that says why");
}
