//! The slider-based adjustment editors, driven through the public API as a host would show them: one binding per
//! editor, one gesture per slider drag, the Set of a check box, a preset or a button, and the requests the colour wells
//! file.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::Mixed;
use craft_imaging::ui::adjust::{AdjustSession, HueRangeValue, HueSaturation, HueSaturationEditor};
use craft_theme::Srgba;
use craft_ui::contract::{Bind, Role};
use craft_ui::testing::{Fixture, trial};
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

/// What the Hue/Saturation editor's host sees: the value, the range shown and what the editor asked the engine last pass.
struct HueHost {
    value: HueSaturation,
    shown: Option<usize>,
    asked: Vec<(HueSaturation, Srgba)>,
}

/// The Hue/Saturation editor over `start`, with an engine that changes nothing and notes what it was asked.
fn hue_saturation(start: HueSaturation) -> (Fixture, Rc<RefCell<HueHost>>) {
    let host = Rc::new(RefCell::new(HueHost { value: start, shown: None, asked: Vec::new() }));
    let held = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        let mut host = held.borrow_mut();
        let HueHost { value, shown, asked } = &mut *host;
        let noted = RefCell::new(Vec::new());
        let engine = |value: &HueSaturation, color: Srgba| {
            noted.borrow_mut().push((*value, color));
            color
        };
        *shown = HueSaturationEditor::new("Hue/Saturation", Bind::bound(value)).adjusted(&engine).show(ui).shown_range;
        *asked = noted.into_inner();
    });
    fixture.room(vec2(400.0, 640.0));
    fixture.settle();
    (fixture, host)
}

/// The range swatches in order, each with whether it is checked.
fn range_swatches(fixture: &Fixture) -> Vec<(String, bool)> {
    let tree = fixture.tree();
    fixture
        .by_role(Role::RADIO_BUTTON)
        .iter()
        .map(|swatch| (tree.name_of(swatch.node).unwrap_or_default().to_owned(), tree.toggled_of(swatch.node) == Some(Mixed::Same(true))))
        .collect()
}

/// The painted degree texts above the Hue row's title: the centre label, if one shows.
fn centre_labels(fixture: &Fixture) -> Vec<String> {
    let texts = fixture.texts();
    let hue = texts.iter().find(|text| text.text == "Hue").expect("the Hue row's title").rect;
    texts.iter().filter(|text| text.text.contains('°') && text.rect.bottom() <= hue.top() + 0.5).map(|text| text.text.clone()).collect()
}

fn click(fixture: &mut Fixture, label: &str) {
    let swatch = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} swatch"));
    fixture.click(&swatch).settle();
}

const RANGES: [&str; 7] = ["Master", "Reds", "Yellows", "Greens", "Cyans", "Blues", "Magentas"];

/// Seven round swatches pick the range, Master selected at first; there is no range dropdown.
#[trial]
fn seven_swatches_replace_the_dropdown_with_master_selected() {
    let (fixture, _) = hue_saturation(HueSaturation::default());
    let want: Vec<(String, bool)> = RANGES.iter().enumerate().map(|(i, name)| ((*name).to_owned(), i == 0)).collect();
    assert_eq!(range_swatches(&fixture), want);
    assert!(fixture.by_role(Role::COMBO_BOX).is_empty(), "no range dropdown");
}

/// A click on a swatch shows that range, checks only its swatch and puts the range's centre above the Hue row; Master
/// has no centre.
#[trial]
fn a_click_selects_that_range_and_shows_its_centre() {
    let (mut fixture, host) = hue_saturation(HueSaturation::default());
    assert!(centre_labels(&fixture).is_empty(), "no centre for Master");
    click(&mut fixture, "Blues");
    assert_eq!(host.borrow().shown, Some(4));
    assert!(range_swatches(&fixture).iter().all(|(name, on)| *on == (name == "Blues")), "{:?}", range_swatches(&fixture));
    assert_eq!(centre_labels(&fixture), ["240°"]);
    click(&mut fixture, "Reds");
    assert_eq!(centre_labels(&fixture), ["0°"]);
    click(&mut fixture, "Master");
    assert_eq!(host.borrow().shown, None);
    assert!(centre_labels(&fixture).is_empty(), "no centre for Master");
}

/// A range's centre is the middle of its fully-on span, within one turn; a Hue shift shows where it moves the centre.
#[trial]
fn centre_labels_name_the_range_centre_and_where_its_hue_moves_it() {
    let centres: Vec<f64> = [0, 1, 4, 5].map(|r| HueRangeValue::neutral(r).centre()).to_vec();
    assert_eq!(centres, [0.0, 60.0, 240.0, 300.0]);
    assert_eq!(HueRangeValue::new(0.0, 0.0, 0.0, [190.0, 220.0, 240.0, 270.0]).centre(), 230.0);
    for (shift, shown) in [(30.0, "0° / 30°"), (-30.0, "0° / 330°")] {
        let mut start = HueSaturation::default();
        start.ranges[0].hue = shift;
        let (mut fixture, _) = hue_saturation(start);
        click(&mut fixture, "Reds");
        assert_eq!(centre_labels(&fixture), [shown]);
    }
}

/// Colorize shows one read-only swatch of the colorize colour and no range swatches.
#[trial]
fn colorize_collapses_the_row_into_one_swatch() {
    let (fixture, _) = hue_saturation(HueSaturation { hue: 120.0, saturation: 25.0, colorize: true, ..HueSaturation::default() });
    assert!(range_swatches(&fixture).is_empty(), "no range swatches");
    let tree = fixture.tree();
    let wells = fixture.by_role(Role::COLOR_WELL).iter().filter(|well| tree.name_of(well.node) == Some("Colorize Color")).count();
    assert_eq!(wells, 1);
}

/// An edited range's swatch shows its centre before and after that range alone: the engine is asked about the centre
/// under a value holding only that range. Back at neutral (what Reset writes) the swatch shows one colour again.
#[trial]
fn an_edited_range_shows_before_and_after() {
    let mut start = HueSaturation { hue: 90.0, ..HueSaturation::default() };
    start.ranges[0].hue = 30.0;
    let (mut fixture, host) = hue_saturation(start);
    let mut only_reds = HueSaturation::default();
    only_reds.ranges[0].hue = 30.0;
    let red = |color: &Srgba| color.r > 0.99 && color.g < 0.01 && color.b < 0.01;
    assert!(host.borrow().asked.iter().any(|(value, color)| *value == only_reds && red(color)), "the Reds' centre under the Reds alone");
    let reds = fixture.by_label("Reds").expect("the Reds swatch");
    assert!(fixture.tree().value_of(reds.node).is_some_and(|value| value.contains(" / ")), "before and after");

    host.borrow_mut().value = HueSaturation::default();
    fixture.settle();
    let shown = host.borrow().value;
    assert!(host.borrow().asked.iter().all(|(value, _)| *value == shown), "no range is asked about alone");
    let reds = fixture.by_label("Reds").expect("the Reds swatch");
    assert!(fixture.tree().value_of(reds.node).is_some_and(|value| !value.contains(" / ")), "one colour");
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
