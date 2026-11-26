//! A length field keeps the document's unit and shows the user's.

use craft_behavior::NumberCodec;
use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::runtime::env::{Env, LengthUnits, UnitKind};
use craft_ui::testing::input::key_press;
use craft_ui::testing::pointer::click;
use craft_ui::testing::{Frames, trial};
use craft_ui::text::NumberField;
use egui::{Event, Key, Modifiers, Pos2, Ui};
use std::cell::{Cell, RefCell};

/// A length the field starts from, in the document's points.
const START: f64 = 72.0;
/// How far two lengths may differ and still count as the same.
const SAME: f64 = 1e-6;

/// A field over points that shows inches, driven by `drive` after it holds focus.
fn in_inches(start: f64, drive: impl FnOnce(&mut Frames, &mut dyn FnMut(&mut Ui))) -> f64 {
    in_lengths(LengthUnits::new(UnitKind::Points, UnitKind::Inches, START), start, drive)
}

/// A field over `lengths`, driven by `drive` after it holds focus.
fn in_lengths(lengths: LengthUnits, start: f64, drive: impl FnOnce(&mut Frames, &mut dyn FnMut(&mut Ui))) -> f64 {
    let mut frames = Frames::new();
    let value = RefCell::new(Some(start));
    let at = Cell::new(Pos2::ZERO);
    let mut show = |ui: &mut Ui| {
        let mut guard = value.borrow_mut();
        at.set(NumberField::length("Width", Bind::bound(&mut *guard), lengths).id_salt("width").show(ui).response.rect.center());
    };
    frames.idle(3, &mut show);
    click(&mut frames, at.get(), &mut show);
    drive(&mut frames, &mut show);
    frames.idle(3, &mut show);
    value.into_inner().unwrap_or_default()
}

fn type_and_commit(text: &'static str) -> impl FnOnce(&mut Frames, &mut dyn FnMut(&mut Ui)) {
    move |frames, show| {
        frames.frame(vec![key_press(Key::A, Modifiers::COMMAND)], &mut *show);
        frames.frame(vec![Event::Text(text.into())], &mut *show);
        frames.idle(1, &mut *show);
        frames.frame(vec![key_press(Key::Enter, Modifiers::NONE)], &mut *show);
    }
}

/// Typed units convert to the document's unit, a bare number is in the shown unit, and an arrow steps by the shown
/// unit's step.
#[trial]
fn a_length_field_types_and_steps_in_the_shown_unit() {
    let one = in_inches(START, type_and_commit("1 in"));
    let metric = in_inches(START, type_and_commit("25.4 mm"));
    assert!((metric - one).abs() < SAME, "25.4 mm is one inch: {metric} against {one}");

    let bare = in_inches(START, type_and_commit("2"));
    let spelled = in_inches(START, type_and_commit("2 in"));
    assert!((bare - spelled).abs() < SAME, "a bare number is in the shown unit: {bare} against {spelled}");

    let stepped = in_inches(one, |frames, show| {
        frames.frame(vec![key_press(Key::ArrowUp, Modifiers::NONE)], show);
    });
    let tenth_up = in_inches(START, type_and_commit("1.1"));
    assert!((stepped - tenth_up).abs() < SAME, "an arrow moves a tenth of an inch: {stepped} against {tenth_up}");
}

/// A Feet & Inches length is written, typed and stepped as feet, as a Feet length is, and only its name tells the two
/// apart.
#[trial]
fn a_feet_and_inches_length_counts_feet() {
    let feet = LengthUnits::new(UnitKind::Points, UnitKind::Feet, START);
    let feet_inches = LengthUnits::new(UnitKind::Points, UnitKind::FeetInches, START);
    for text in ["1.5", "18 in", "1.5 ft"] {
        let plain = in_lengths(feet, START, type_and_commit(text));
        let both = in_lengths(feet_inches, START, type_and_commit(text));
        assert!((both - plain).abs() < SAME && both != START, "{text} reads as feet: {both} against {plain}");
    }
    let stepped = |lengths: LengthUnits| {
        in_lengths(lengths, START, |frames, show| {
            frames.frame(vec![key_press(Key::ArrowUp, Modifiers::NONE)], show);
        })
    };
    assert!((stepped(feet_inches) - stepped(feet)).abs() < SAME, "an arrow steps both by the same tenth of a foot");

    let mut frames = Frames::new();
    frames.frame(vec![], |ui| {
        let units = Env::of(ui).units();
        assert_eq!(units.show(UnitKind::FeetInches, 1.5), units.show(UnitKind::Feet, 1.5), "both write feet");
        assert_ne!(units.name(UnitKind::FeetInches), units.name(UnitKind::Feet), "a list of units tells them apart");
        for kind in [UnitKind::Feet, UnitKind::FeetInches] {
            let reader = units.length_reader(LengthUnits::new(UnitKind::Points, kind, START));
            let named = reader.read_tagged("1.5 ft", None).ok().and_then(|(_, tag)| tag).and_then(UnitKind::from_tag);
            assert_eq!(named, Some(kind), "a typed ft names the field's own unit");
        }
    });
}

/// With bare numbers in points, a bare number in a picas field reads as points; a spelled unit still reads as written.
#[trial]
fn a_bare_number_reads_in_the_lengths_bare_unit() {
    let picas = LengthUnits::new(UnitKind::Points, UnitKind::Picas, START);
    let in_points = picas.bare_in(UnitKind::Points);
    let bare = in_lengths(in_points, START, type_and_commit("12"));
    let spelled = in_lengths(in_points, START, type_and_commit("12 pt"));
    assert!((bare - spelled).abs() < SAME, "a bare 12 is 12 points: {bare} against {spelled}");
    let in_picas = in_lengths(picas, START, type_and_commit("12"));
    assert!(in_picas > bare, "without it a bare 12 is 12 picas: {in_picas} against {bare}");
    let picas_spelled = in_lengths(in_points, START, type_and_commit("1 pica"));
    let picas_plain = in_lengths(picas, START, type_and_commit("1"));
    assert!((picas_spelled - picas_plain).abs() < SAME, "a spelled pica is still a pica");
}

/// Any length field reads picas and points written together (`2p6`) as their sum, and a percentage after `*` as a
/// share of the quantity before it.
#[trial]
fn picas_with_points_and_a_percentage_factor_read_as_print_apps_write_them() {
    let mm = LengthUnits::new(UnitKind::Points, UnitKind::Millimetres, START);
    let together = in_lengths(mm, START, type_and_commit("2p6"));
    let summed = in_lengths(mm, START, type_and_commit("2 pica + 6 pt"));
    assert!((together - summed).abs() < SAME && together != START, "2p6 is two picas and six points: {together} against {summed}");
    let share = in_lengths(mm, START, type_and_commit("3cm*50%"));
    let half = in_lengths(mm, START, type_and_commit("1.5cm"));
    assert!((share - half).abs() < SAME && share != START, "3cm*50% is half of 3 cm: {share} against {half}");
}

/// With `NumberCodec::share_of_start`, a lone percentage typed key by key is that share of the length the edit began
/// on, applied on Enter: the numbers typed before the `%` apply as typed, and the share does not read them.
#[trial]
fn a_lone_percentage_is_that_share_of_the_length_the_edit_began_on() {
    let mm = LengthUnits::new(UnitKind::Points, UnitKind::Millimetres, START);
    let mut frames = Frames::new();
    let value = RefCell::new(Some(START));
    let at = Cell::new(Pos2::ZERO);
    let mut show = |ui: &mut Ui| {
        let codec = NumberCodec::new(Env::of(ui).units().length_reader(mm), 2).share_of_start();
        let mut guard = value.borrow_mut();
        let field = NumberField::length("Width", Bind::bound(&mut *guard), mm).codec(codec).id_salt("width");
        at.set(field.show(ui).response.rect.center());
    };
    let half = |v: Option<f64>| v.is_some_and(|v| (v - START / 2.0).abs() < SAME);
    frames.idle(3, &mut show);
    click(&mut frames, at.get(), &mut show);
    frames.frame(vec![key_press(Key::A, Modifiers::COMMAND)], &mut show);
    for typed in ["5", "0", "%"] {
        frames.frame(vec![Event::Text(typed.into())], &mut show);
        frames.idle(1, &mut show);
    }
    let typed = *value.borrow();
    assert!(typed != Some(START) && !half(typed), "the numbers apply as typed, the share waits for Enter: {typed:?}");
    frames.frame(vec![key_press(Key::Enter, Modifiers::NONE)], &mut show);
    frames.idle(3, &mut show);
    let committed = value.into_inner();
    assert!(half(committed), "50% is half the length the edit began on: {committed:?}");
}
