//! Where a number field puts its value and the face it sets it in: `centred` and `mono`, either of them off, on their
//! own and together, in measure, paint and editing.

use craft_behavior::Mixed;
use craft_i18n::Direction;
use craft_ui::contract::{Bind, Props as _, Show as _, Sizing};
use craft_ui::runtime::env::UnitKind;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::NumberField;
use egui::{FontFamily, Key, Modifiers, Rect, Shape};
use std::cell::RefCell;
use std::rc::Rc;

/// The texts a frame painted, each with where it lies and the face it is set in.
fn painted(shapes: &[Shape]) -> Vec<(String, Rect, FontFamily, f32)> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Text(text) => {
                let font = text.galley.job.sections.first()?.format.font_id.clone();
                let rect = text.galley.rect.translate(text.pos.to_vec2());
                Some((text.galley.job.text.clone(), rect, font.family, font.size))
            }
            _ => None,
        })
        .collect()
}

/// The painted run of `text` that lies inside `field`, from the frame's shapes.
fn run<'a>(runs: &'a [(String, Rect, FontFamily, f32)], field: Rect, text: &str) -> Option<&'a (String, Rect, FontFamily, f32)> {
    runs.iter().find(|run| run.0 == text && field.expand(1.0).contains_rect(run.1))
}

/// The rect of the value `text` a frame of `fixture` painted inside `field`.
fn value_in(fixture: &Fixture, field: Rect, text: &str) -> Rect {
    let found = fixture.texts().iter().find(|run| run.text == text && field.expand(1.0).contains_rect(run.rect));
    match found {
        Some(run) => run.rect,
        None => panic!("{field:?} paints no {text}: {:?}", fixture.texts().iter().map(|run| run.text.clone()).collect::<Vec<_>>()),
    }
}

/// The rect of the one run inside `field` that is not the value `text`: the field's unit.
fn beside_in(fixture: &Fixture, field: Rect, text: &str) -> Rect {
    let found = fixture.texts().iter().find(|run| run.text != text && field.expand(1.0).contains_rect(run.rect));
    match found {
        Some(run) => run.rect,
        None => panic!("{field:?} paints no unit beside {text}: {:?}", fixture.texts().iter().map(|run| run.text.clone()).collect::<Vec<_>>()),
    }
}

/// Two settled fields of one presentation: "5" and "100" over one range and of one width, so where the fields put them
/// says where that presentation puts values of other widths.
struct Pair {
    fixture: Fixture,
    fields: Vec<Rect>,
}

impl Pair {
    fn new(started: bool, direction: Option<Direction>) -> Pair {
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
        Pair { fixture, fields }
    }

    /// The rects of the two values, "5" and "100".
    fn values(&self) -> [Rect; 2] {
        [value_in(&self.fixture, self.fields[0], "5"), value_in(&self.fixture, self.fields[1], "100")]
    }
}

/// The presentation every number field has had: the value centred in the room the field leaves for it, in the monospace
/// face. A value of another width keeps its centre, and `centred(false)` starts it at the reading start instead, which in
/// a left-to-right interface is the field's left edge.
#[trial]
fn a_field_centres_its_value_in_the_mono_face_unless_told_to_start_it() {
    let mut centred = Pair::new(false, None);
    let started = Pair::new(true, None);
    let (c, s) = (centred.values(), started.values());
    assert!((c[0].center().x - c[1].center().x).abs() <= 0.5, "centred values of other widths share a centre: {c:?}");
    assert!((c[0].left() - c[1].left()).abs() > 2.0, "centred values of other widths start at other places: {c:?}");
    assert!((s[0].left() - s[1].left()).abs() <= 0.5, "started values share the reading start: {s:?}");
    assert!(s[1].left() < started.fields[1].center().x - 2.0, "a started value sits at the field's start, not its centre: {:?}", s[1]);
    // The default presentation sets the value in the monospace face: both fields' values are mono runs.
    let runs = painted(&centred.fixture.frame_shapes());
    let values: Vec<&(String, Rect, FontFamily, f32)> = runs.iter().filter(|run| run.0 == "5" || run.0 == "100").collect();
    assert_eq!(values.len(), 2, "the two fields paint their two values: {runs:?}");
    assert!(values.iter().all(|run| run.2 == FontFamily::Monospace), "a value is set in the mono face by default: {runs:?}");
}

/// `mono(false)` sets the value in the UI body face, and the face is the one the field measures its value in too: a
/// content-sized field is its value, its caret and its padding wide, so the room the field takes is the room its text
/// fills in either face.
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
    });
    fixture.settle();
    let fields = boxes.borrow().clone();
    let runs = painted(&fixture.frame_shapes());
    let family = |field: Rect| run(&runs, field, "111").map(|run| run.2.clone());
    assert_eq!(family(fields[0]), Some(FontFamily::Monospace), "a field sets its value in the mono face by default: {runs:?}");
    assert_eq!(family(fields[1]), Some(FontFamily::Proportional), "mono(false) sets the value in the UI body face: {runs:?}");
    let room = |field: Rect| {
        let value = value_in(&fixture, field, "111");
        assert!(field.expand(1.0).contains_rect(value), "the field is as wide as the value it measured: {value:?} in {field:?}");
        // The padding and the caret's room are the field's own in either face; the value is all that the face changes.
        field.width() - value.width()
    };
    let (mono, proportional) = (room(fields[0]), room(fields[1]));
    // The box and the text are laid out at the points the field's measure gives them, so a face the measure missed would
    // leave a different room around the value by the whole of an advance.
    assert!((mono - proportional).abs() <= 1.0, "the field measures the face it paints: {mono} beside {proportional}");
}

/// A started value keeps the field's unit where the field always put it: at the end, apart from the number, so a column
/// of fields still lines its units up.
#[trial]
fn a_started_value_keeps_its_unit_at_the_end_of_the_field() {
    let boxes: Rc<RefCell<Vec<Rect>>> = Rc::default();
    let b = boxes.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut v = b.borrow_mut();
        v.clear();
        for (at, value) in [5.0, 100.0].into_iter().enumerate() {
            let field = NumberField::new("N", Bind::given(Some(value))).range(0.0..=100.0).unit_kind(UnitKind::Percent).centred(false).id_salt(at);
            v.push(field.show(ui).response.rect);
        }
    });
    fixture.settle();
    let fields = boxes.borrow().clone();
    let numbers = [value_in(&fixture, fields[0], "5"), value_in(&fixture, fields[1], "100")];
    let units = [beside_in(&fixture, fields[0], "5"), beside_in(&fixture, fields[1], "100")];
    assert!((numbers[0].left() - numbers[1].left()).abs() <= 0.5, "started values share the reading start beside a unit: {numbers:?}");
    assert!((units[0].right() - units[1].right()).abs() <= 0.5, "the units end at one place from the field's end: {units:?}");
    assert!(numbers[1].right() < units[1].left(), "the unit stands apart from the number: {numbers:?} {units:?}");
}

/// A started value follows a right-to-left interface: the reading start is the field's right edge there, and a centred
/// value keeps the field's centre whatever the direction.
#[trial]
fn a_started_value_follows_a_right_to_left_interface() {
    let centred = Pair::new(false, Some(Direction::Rtl));
    let started = Pair::new(true, Some(Direction::Rtl));
    let (c, s) = (centred.values(), started.values());
    assert!((c[0].center().x - c[1].center().x).abs() <= 0.5, "centred values share a centre under right-to-left: {c:?}");
    assert!((s[0].right() - s[1].right()).abs() <= 0.5, "started values share the reading start, the right edge, under right-to-left: {s:?}");
    assert!(s[1].right() > started.fields[1].center().x + 2.0, "a started value sits at the field's right edge: {:?}", s[1]);
}

/// The caret sits beside the value wherever the field puts it: after the last digit at End and before the first at Home,
/// inside the field's own box, centred or started, mono or not. An editing field lays its text out as its measure did.
#[trial]
fn the_caret_follows_the_value_wherever_the_field_puts_it() {
    for (centred, mono) in [(true, true), (false, true), (false, false)] {
        let boxes: Rc<RefCell<Vec<Rect>>> = Rc::default();
        let b = boxes.clone();
        let mut fixture = Fixture::new(move |ui| {
            let mut v = b.borrow_mut();
            v.clear();
            v.push(
                NumberField::new("N", Bind::given(Some(100.0)))
                    .centred(centred)
                    .mono(mono)
                    .sizing(Sizing::Content)
                    .id_salt("n")
                    .show(ui)
                    .response
                    .rect,
            );
        });
        fixture.settle();
        let field = boxes.borrow()[0];
        let value = value_in(&fixture, field, "100");
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

/// A mixed field's dash and a disabled field's value are presented like any other value: `centred(false)` starts them at
/// the reading start too, so a field keeps its look when it holds no value or cannot be edited.
#[trial]
fn a_mixed_or_disabled_field_keeps_the_presentation_it_is_given() {
    let boxes: Rc<RefCell<Vec<Rect>>> = Rc::default();
    let b = boxes.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut v = b.borrow_mut();
        v.clear();
        let mut mixed = Mixed::<Option<f64>>::Mixed;
        v.push(NumberField::new("Mixed", Bind::bound_mixed(&mut mixed)).range(0.0..=100.0).centred(false).id_salt("mixed").show(ui).response.rect);
        v.push(
            NumberField::new("Off", Bind::given(Some(100.0))).range(0.0..=100.0).centred(false).enabled(false).id_salt("off").show(ui).response.rect,
        );
    });
    fixture.settle();
    let fields = boxes.borrow().clone();
    let dash = {
        let found = fixture.texts().iter().find(|run| fields[0].expand(1.0).contains_rect(run.rect)).map(|run| run.rect);
        match found {
            Some(rect) => rect,
            None => panic!("a mixed field paints its dash: {:?}", fixture.texts().iter().map(|run| run.text.clone()).collect::<Vec<_>>()),
        }
    };
    assert!(dash.left() < fields[0].center().x - 2.0, "the mixed dash starts at the field's start: {dash:?} in {:?}", fields[0]);
    let value = value_in(&fixture, fields[1], "100");
    assert!(value.left() < fields[1].center().x - 2.0, "a disabled value starts at the field's start: {value:?} in {:?}", fields[1]);
}
