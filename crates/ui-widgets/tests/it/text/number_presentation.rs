//! Where a number field puts its value and the role it sets it in: `centred` and `mono`, either of them off, on their own
//! and together, in measure, paint and editing.

use craft_behavior::Mixed;
use craft_i18n::Direction;
use craft_ui::contract::{Bind, Props as _, Show as _, Sizing};
use craft_ui::display::Text;
use craft_ui::runtime::env::UnitKind;
use craft_ui::style::TypeRole;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::NumberField;
use egui::{FontFamily, FontId, Key, Modifiers, Rect, Shape};
use std::cell::RefCell;
use std::rc::Rc;

/// The label of the reference run set in the monospace role.
const MONO_ROLE: &str = "mono role";
/// The label of the reference run set in the UI body role.
const BODY_ROLE: &str = "body role";

/// A text run a frame painted: its text, where it lies and the font it is set in.
struct Run {
    text: String,
    rect: Rect,
    font: FontId,
}

/// The runs the next frame paints, with their fonts.
fn painted_runs(fixture: &mut Fixture) -> Vec<Run> {
    fixture
        .frame_shapes()
        .into_iter()
        .filter_map(|shape| match shape {
            Shape::Text(text) => {
                let font = text.galley.job.sections.first()?.format.font_id.clone();
                Some(Run { text: text.galley.job.text.clone(), rect: text.galley.rect.translate(text.pos.to_vec2()), font })
            }
            _ => None,
        })
        .collect()
}

/// The run showing `text` inside `field`.
fn run_in<'a>(runs: &'a [Run], field: Rect, text: &str) -> Option<&'a Run> {
    runs.iter().find(|run| run.text == text && field.expand(1.0).contains_rect(run.rect))
}

/// The run showing `text` inside `field`, or a panic naming what the frame painted.
fn shown<'a>(runs: &'a [Run], field: Rect, text: &str) -> &'a Run {
    match run_in(runs, field, text) {
        Some(run) => run,
        None => panic!("{field:?} paints no {text}: {:?}", runs.iter().map(|run| run.text.clone()).collect::<Vec<_>>()),
    }
}

/// The one run inside `field` (the dash of a mixed field).
fn only(runs: &[Run], field: Rect) -> &Run {
    match runs.iter().find(|run| field.expand(1.0).contains_rect(run.rect)) {
        Some(run) => run,
        None => panic!("{field:?} paints nothing: {:?}", runs.iter().map(|run| run.text.clone()).collect::<Vec<_>>()),
    }
}

/// The unit a field paints: the one run inside it that is not the value `text`.
fn unit_of<'a>(runs: &'a [Run], field: Rect, text: &str) -> &'a Run {
    match runs.iter().find(|run| run.text != text && field.expand(1.0).contains_rect(run.rect)) {
        Some(run) => run,
        None => panic!("{field:?} paints no unit beside {text}: {:?}", runs.iter().map(|run| run.text.clone()).collect::<Vec<_>>()),
    }
}

/// How far `run` sits from the edge the text reads from: its reading start when `start`, else its reading end; the left
/// edge in a left-to-right interface, the right one under right-to-left.
fn inset(run: Rect, field: Rect, start: bool, rtl: bool) -> f32 {
    if start == rtl { field.right() - run.right() } else { run.left() - field.left() }
}

/// Two settled fields of one presentation: "5" and "100" over one range and of one width, so where the fields put them
/// says where that presentation puts values of other widths.
fn twins(started: bool, direction: Option<Direction>) -> (Vec<Rect>, Fixture) {
    let boxes: Rc<RefCell<Vec<Rect>>> = Rc::default();
    let b = boxes.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut v = b.borrow_mut();
        v.clear();
        for (at, value) in [5.0, 100.0].into_iter().enumerate() {
            v.push(NumberField::new("N", Bind::given(Some(value))).range(0.0..=100.0).centred(!started).id_salt(at).show(ui).response.rect);
        }
    });
    if let Some(direction) = direction {
        fixture.direction(direction);
    }
    fixture.settle();
    let fields = boxes.borrow().clone();
    (fields, fixture)
}

/// Four settled fields with a unit: "5" and "100" started, then "5" and "100" centred, all over one range and of one
/// width, so each presentation is the other's twin.
fn unit_twins(direction: Option<Direction>) -> (Vec<Rect>, Fixture) {
    let boxes: Rc<RefCell<Vec<Rect>>> = Rc::default();
    let b = boxes.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut v = b.borrow_mut();
        v.clear();
        for (at, (value, centred)) in [(5.0, false), (100.0, false), (5.0, true), (100.0, true)].into_iter().enumerate() {
            let field = NumberField::new("N", Bind::given(Some(value))).range(0.0..=100.0).unit_kind(UnitKind::Percent).centred(centred);
            v.push(field.id_salt(at).show(ui).response.rect);
        }
    });
    if let Some(direction) = direction {
        fixture.direction(direction);
    }
    fixture.settle();
    let fields = boxes.borrow().clone();
    (fields, fixture)
}

/// The presentation every number field has had: the value centred in the room the field leaves for it, in the monospace
/// role. A value of another width keeps its centre; `centred(false)` starts it at the logical reading start instead, the
/// left edge in a left-to-right interface and the right edge under right-to-left.
#[trial]
fn a_field_centres_its_value_in_the_mono_face_unless_told_to_start_it() {
    for direction in [None, Some(Direction::Rtl)] {
        let rtl = direction.is_some();
        let what = if rtl { "under right-to-left" } else { "under left-to-right" };
        let (centred_fields, mut centred) = twins(false, direction);
        let (started_fields, mut started) = twins(true, direction);
        let (centred_runs, started_runs) = (painted_runs(&mut centred), painted_runs(&mut started));
        let c = [shown(&centred_runs, centred_fields[0], "5"), shown(&centred_runs, centred_fields[1], "100")];
        let s = [shown(&started_runs, started_fields[0], "5"), shown(&started_runs, started_fields[1], "100")];
        assert!(
            (c[0].rect.center().x - c[1].rect.center().x).abs() <= 0.5,
            "centred values of other widths share a centre {what}: {:?}",
            (c[0].rect, c[1].rect)
        );
        assert!(
            (c[0].rect.left() - c[1].rect.left()).abs() > 2.0,
            "centred values of other widths start at other places {what}: {:?}",
            (c[0].rect, c[1].rect)
        );
        let (s5, s100) = (inset(s[0].rect, started_fields[0], true, rtl), inset(s[1].rect, started_fields[1], true, rtl));
        assert!((s5 - s100).abs() <= 0.5, "started values share the logical reading start {what}: {s5} beside {s100}");
        // The centred twin of the same field says how far in a centred value sits: a started one is at the near edge, not
        // merely left of where a wider centred value would reach.
        let c5 = inset(c[0].rect, centred_fields[0], true, rtl);
        assert!(s5 < c5 / 2.0, "a started value sits at the reading start, not at a centred offset {what}: {s5} beside {c5}");
        // The default presentation sets the value in the monospace role.
        assert!(
            c.iter().chain(&s).all(|run| run.font.family == FontFamily::Monospace),
            "a value is set in the monospace role by default {what}: {:?}",
            (c[0].font.clone(), s[0].font.clone())
        );
    }
}

/// `mono(false)` sets the value in the UI body role, and the field measures the role it paints: a content-sized field is
/// its value, its caret and its padding wide in either role, so the room it takes is the room its text fills. The
/// reference runs are the roles' own fonts, size included, so this pins the whole painted font and not only the family.
#[trial]
fn the_mono_choice_sets_the_face_the_field_paints_and_measures() {
    let boxes: Rc<RefCell<Vec<Rect>>> = Rc::default();
    let b = boxes.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut v = b.borrow_mut();
        v.clear();
        for (at, mono) in [true, false].into_iter().enumerate() {
            v.push(NumberField::new("N", Bind::given(Some(111.0))).mono(mono).sizing(Sizing::Content).id_salt(at).show(ui).response.rect);
        }
        Text::new(MONO_ROLE).role(TypeRole::Mono).show(ui);
        Text::new(BODY_ROLE).role(TypeRole::Body).show(ui);
    });
    fixture.settle();
    let fields = boxes.borrow().clone();
    let runs = painted_runs(&mut fixture);
    let mono = shown(&runs, fields[0], "111");
    let proportional = shown(&runs, fields[1], "111");
    let reference = |text: &str| runs.iter().find(|run| run.text == text).map(|run| run.font.clone());
    let (mono_role, body_role) = (reference(MONO_ROLE), reference(BODY_ROLE));
    assert!(
        mono_role.is_some() && body_role.is_some() && mono_role != body_role,
        "the two roles are set in other fonts: {mono_role:?} {body_role:?}"
    );
    assert_eq!(Some(mono.font.clone()), mono_role, "a field sets its value in the monospace role, face and size");
    assert_eq!(Some(proportional.font.clone()), body_role, "mono(false) sets the value in the UI body role, face and size");
    let room = |field: Rect, run: &Run| {
        assert!(field.expand(1.0).contains_rect(run.rect), "the field is as wide as the value it measured: {:?} in {field:?}", run.rect);
        // The padding and the caret's room are the field's own in either role; the value is all that the role changes.
        field.width() - run.rect.width()
    };
    let (measured, painted) = (room(fields[0], mono), room(fields[1], proportional));
    // The value "111" is what this unbounded field measures (the current value, not a fixed sample), and its digits are
    // set at clearly different advances in the two roles, so a measure that named the face itself leaves the room apart.
    assert!((measured - painted).abs() <= 1.0, "the field measures the role it paints: {measured} beside {painted}");
}

/// A started value keeps the field's unit where the field always put it: at the logical reading end, apart from the
/// number, and at the same inset the value keeps from the reading start. The centred twin of each value shows the unit
/// is the same either way.
#[trial]
fn a_started_value_keeps_its_unit_at_the_reading_end() {
    for direction in [None, Some(Direction::Rtl)] {
        let rtl = direction.is_some();
        let what = if rtl { "under right-to-left" } else { "under left-to-right" };
        let (fields, mut fixture) = unit_twins(direction);
        let runs = painted_runs(&mut fixture);
        let texts = ["5", "100", "5", "100"];
        let numbers: Vec<&Run> = fields.iter().zip(texts).map(|(field, text)| shown(&runs, *field, text)).collect();
        let units: Vec<&Run> = fields.iter().zip(texts).map(|(field, text)| unit_of(&runs, *field, text)).collect();
        // Started values share the reading start, and it is the inset the unit keeps from the other end.
        let (s5, s100) = (inset(numbers[0].rect, fields[0], true, rtl), inset(numbers[1].rect, fields[1], true, rtl));
        assert!((s5 - s100).abs() <= 0.5, "started values share the reading start beside a unit {what}: {s5} beside {s100}");
        let unit_end = inset(units[0].rect, fields[0], false, rtl);
        // The field's horizontal padding is symmetric at this preset, so the value keeps at the reading start the inset the
        // unit keeps at the reading end: coverage of the default preset's padding, not a claim that every theme's is.
        assert!((s5 - unit_end).abs() <= 0.5, "the value starts at the inset the unit keeps from the reading end {what}: {s5} beside {unit_end}");
        // The unit is the same whichever the presentation: all four fields end it at one inset.
        let ends: Vec<f32> = units.iter().zip(&fields).map(|(unit, field)| inset(unit.rect, *field, false, rtl)).collect();
        assert!(ends.iter().all(|end| (end - ends[0]).abs() <= 0.5), "the units end at one place {what}: {ends:?}");
        // The centred twins sit further in, and share their centre.
        let (c5, c100) = (inset(numbers[2].rect, fields[2], true, rtl), inset(numbers[3].rect, fields[3], true, rtl));
        assert!(c5 > s5 + 2.0 && c100 > s100 + 2.0, "a centred value sits further in than a started one {what}: {c5} {c100} beside {s5} {s100}");
        assert!(
            (numbers[2].rect.center().x - numbers[3].rect.center().x).abs() <= 0.5,
            "centred values share a centre {what}: {:?}",
            (numbers[2].rect, numbers[3].rect)
        );
        for (number, unit) in numbers.iter().zip(&units) {
            let apart = if rtl { number.rect.left() - unit.rect.right() } else { unit.rect.left() - number.rect.right() };
            assert!(apart > 0.0, "the unit stands apart from the number {what}: {:?} beside {:?}", number.rect, unit.rect);
        }
    }
}

/// The caret sits beside the value wherever the field puts it: after the last digit at End and before the first at Home,
/// inside the field's own box, in all four presentations. The field is its ordinary cell width rather than sized to its
/// content, so a value placed by the wrong alignment moves across the whole band (tens of points), not within the caret's
/// own room. This compares the editable caret with the resting painted value.
#[trial]
fn the_caret_follows_the_value_wherever_the_field_puts_it() {
    for (centred, mono) in [(true, true), (true, false), (false, true), (false, false)] {
        let boxes: Rc<RefCell<Vec<Rect>>> = Rc::default();
        let b = boxes.clone();
        let mut fixture = Fixture::new(move |ui| {
            let mut v = b.borrow_mut();
            v.clear();
            v.push(NumberField::new("N", Bind::given(Some(100.0))).centred(centred).mono(mono).id_salt("n").show(ui).response.rect);
        });
        fixture.settle();
        let field = boxes.borrow()[0];
        let runs = painted_runs(&mut fixture);
        let value = shown(&runs, field, "100").rect;
        assert!(fixture.focused().is_none(), "the resting paint is the unfocused field's: {field:?}");
        fixture.key(Key::Tab, Modifiers::NONE);
        fixture.settle();
        fixture.key(Key::End, Modifiers::NONE);
        let end = fixture.ime().expect("a field in edit reports its caret").caret;
        fixture.key(Key::Home, Modifiers::NONE);
        let home = fixture.ime().expect("a field in edit reports its caret").caret;
        let which = format!("centred {centred}, mono {mono}");
        assert!((end.min.x - value.right()).abs() <= 1.5, "the caret at End sits after the value {value:?}: {end:?} ({which})");
        assert!((home.min.x - value.left()).abs() <= 1.5, "the caret at Home sits before the value {value:?}: {home:?} ({which})");
        assert!(
            field.expand(1.5).contains_rect(end) && field.expand(1.5).contains_rect(home),
            "the caret stays in the field {field:?}: {end:?} {home:?} ({which})"
        );
    }
}

/// A value wider than the room the field gives it: the text is drawn from the field's inner edge and overflows the box,
/// and in edit the field scrolls so that Home and End both keep the caret inside it. egui's single-line editor clamps
/// either alignment to the inner edge once the text is wider than the room (`builder.rs`, `align_size_within_rect(...)
/// .intersect(inner_rect)`), so this trial covers the overflow and the scrolling and cannot tell the two settings apart;
/// both are exercised for that shared path only.
#[trial]
fn a_long_value_overflows_its_room_and_keeps_its_caret_in_view() {
    const LONG: f64 = 123_456_789_012_345.0;
    const LONG_TEXT: &str = "123456789012345";
    for centred in [false, true] {
        let boxes: Rc<RefCell<Vec<Rect>>> = Rc::default();
        let b = boxes.clone();
        let mut fixture = Fixture::new(move |ui| {
            let mut v = b.borrow_mut();
            v.clear();
            v.push(NumberField::new("N", Bind::given(Some(LONG))).centred(centred).id_salt("long").show(ui).response.rect);
        });
        fixture.settle();
        let field = boxes.borrow()[0];
        let runs = painted_runs(&mut fixture);
        // This fixture shows one field, so the value is found by text alone: a value wider than its room is painted
        // beyond the field's box, and no containment test would find it.
        let value = match runs.iter().find(|run| run.text == LONG_TEXT) {
            Some(run) => run.rect,
            None => panic!("the field paints no {LONG_TEXT}: {:?}", runs.iter().map(|run| run.text.clone()).collect::<Vec<_>>()),
        };
        // The overflow is what the trial is for: the ordinary layout keeps the field at one number cell, so the value is
        // wider than the field it is drawn across (were the field grown to fit, this would fail rather than test nothing).
        assert!(value.width() > field.width(), "the value is wider than the field it overflows (centred {centred}): {value:?} in {field:?}");
        // The clamped text begins at the field's inner edge: not left of it, and not at a placed offset inside it.
        let start = inset(value, field, true, false);
        assert!(
            start >= -0.5 && start < field.width() / 4.0,
            "an overflowing value is drawn from the field's inner edge (centred {centred}): {start} in {field:?}"
        );
        // In edit the value is wider than the room, so the field scrolls: the caret stays inside the box at both ends.
        fixture.key(Key::Tab, Modifiers::NONE);
        fixture.settle();
        fixture.key(Key::Home, Modifiers::NONE);
        let home = fixture.ime().expect("a field in edit reports its caret").caret;
        fixture.key(Key::End, Modifiers::NONE);
        let end = fixture.ime().expect("a field in edit reports its caret").caret;
        assert!(field.expand(1.5).contains_rect(home), "the caret at Home stays in the field (centred {centred}): {home:?} in {field:?}");
        assert!(field.expand(1.5).contains_rect(end), "the field scrolls so the caret at End stays in it (centred {centred}): {end:?} in {field:?}");
        assert!(end.min.x > home.min.x, "End is past Home (centred {centred}): {home:?} {end:?}");
    }
}

/// A mixed field's dash and a disabled field's value are presented like any other value, and each has its centred twin
/// in the same fixture: the twin proves it is centred, and the started ones must sit at the start the twin rules out.
#[trial]
fn a_mixed_or_disabled_field_keeps_the_presentation_it_is_given() {
    let boxes: Rc<RefCell<Vec<Rect>>> = Rc::default();
    let b = boxes.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut v = b.borrow_mut();
        v.clear();
        let mut mixed = Mixed::<Option<f64>>::Mixed;
        v.push(
            NumberField::new("Mixed", Bind::bound_mixed(&mut mixed))
                .range(0.0..=100.0)
                .centred(false)
                .id_salt("mixed_started")
                .show(ui)
                .response
                .rect,
        );
        let mut mixed = Mixed::<Option<f64>>::Mixed;
        v.push(NumberField::new("Mixed", Bind::bound_mixed(&mut mixed)).range(0.0..=100.0).id_salt("mixed_centred").show(ui).response.rect);
        v.push(
            NumberField::new("Off", Bind::given(Some(100.0)))
                .range(0.0..=100.0)
                .centred(false)
                .enabled(false)
                .id_salt("off_started")
                .show(ui)
                .response
                .rect,
        );
        v.push(NumberField::new("Off", Bind::given(Some(100.0))).range(0.0..=100.0).enabled(false).id_salt("off_centred").show(ui).response.rect);
    });
    fixture.settle();
    let fields = boxes.borrow().clone();
    let runs = painted_runs(&mut fixture);
    let (dash_started, dash_centred) = (only(&runs, fields[0]), only(&runs, fields[1]));
    let (value_started, value_centred) = (shown(&runs, fields[2], "100"), shown(&runs, fields[3], "100"));
    // Each twin really is centred: the run's centre is its field's.
    assert!(
        (dash_centred.rect.center().x - fields[1].center().x).abs() <= 0.5,
        "the centred twin centres the mixed dash {:?} in {:?}",
        dash_centred.rect,
        fields[1]
    );
    assert!(
        (value_centred.rect.center().x - fields[3].center().x).abs() <= 0.5,
        "the centred twin centres the disabled value {:?} in {:?}",
        value_centred.rect,
        fields[3]
    );
    // The started ones sit at the field's start the twin rules out: both at the same inset, under half the twin's.
    let (dash_in, value_in) = (dash_started.rect.left() - fields[0].left(), value_started.rect.left() - fields[2].left());
    let (dash_twin, value_twin) = (dash_centred.rect.left() - fields[1].left(), value_centred.rect.left() - fields[3].left());
    assert!((dash_in - value_in).abs() <= 0.5, "a mixed field's dash starts where a value starts: {dash_in} beside {value_in}");
    assert!(dash_in < dash_twin / 2.0, "the mixed dash starts at the start, not at a centred offset: {dash_in} beside {dash_twin}");
    assert!(value_in < value_twin / 2.0, "a disabled value starts at the start, not at a centred offset: {value_in} beside {value_twin}");
}
