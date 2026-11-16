//! Number-field alignment and type roles agree across measurement, resting paint and editing.

use craft_behavior::Mixed;
use craft_i18n::Direction;
use craft_ui::contract::{Bind, Props as _, Show as _, Sizing};
use craft_ui::display::Text;
use craft_ui::primitives::HAlign;
use craft_ui::runtime::env::UnitKind;
use craft_ui::style::TypeRole;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::NumberField;
use egui::{FontId, Key, Modifiers, Rect, Shape};
use std::cell::RefCell;
use std::rc::Rc;

const ALIGNS: [HAlign; 3] = [HAlign::Start, HAlign::Center, HAlign::End];
const ROLES: [TypeRole; 3] = [TypeRole::Mono, TypeRole::Body, TypeRole::Caption];

struct Run {
    text: String,
    rect: Rect,
    font: FontId,
}

fn painted_runs(fixture: &mut Fixture) -> Vec<Run> {
    fixture
        .frame_shapes()
        .into_iter()
        .filter_map(|shape| match shape {
            Shape::Text(text) => Some(Run {
                text: text.galley.job.text.clone(),
                rect: text.galley.rect.translate(text.pos.to_vec2()),
                font: text.galley.job.sections.first()?.format.font_id.clone(),
            }),
            _ => None,
        })
        .collect()
}

fn shown<'a>(runs: &'a [Run], field: Rect, text: &str) -> &'a Run {
    runs.iter()
        .find(|run| run.text == text && field.expand(1.0).contains_rect(run.rect))
        .unwrap_or_else(|| panic!("{field:?} paints no {text}: {:?}", runs.iter().map(|run| &run.text).collect::<Vec<_>>()))
}

fn edge(run: Rect, field: Rect, align: HAlign, rtl: bool) -> f32 {
    match (align, rtl) {
        (HAlign::Center, _) => run.center().x - field.center().x,
        (HAlign::Start, false) | (HAlign::End, true) => run.left() - field.left(),
        (HAlign::Start, true) | (HAlign::End, false) => field.right() - run.right(),
    }
}

#[trial]
fn alignment_places_values_at_the_reading_edges_and_keeps_units_separate() {
    for direction in [Direction::Ltr, Direction::Rtl] {
        let rtl = direction == Direction::Rtl;
        for with_unit in [false, true] {
            let boxes: Rc<RefCell<Vec<Rect>>> = Rc::default();
            let b = boxes.clone();
            let mut fixture = Fixture::new(move |ui| {
                let mut boxes = b.borrow_mut();
                boxes.clear();
                for (at, align) in ALIGNS.into_iter().enumerate() {
                    for (which, value) in [5.0, 100.0].into_iter().enumerate() {
                        let mut field = NumberField::new("N", Bind::given(Some(value))).range(0.0..=100.0);
                        // The centre case also checks the default, not only its setter.
                        if align != HAlign::Center {
                            field = field.align(align);
                        }
                        if with_unit {
                            field = field.unit_kind(UnitKind::Percent);
                        }
                        boxes.push(field.id_salt((at, which)).show(ui).response.rect);
                    }
                }
            });
            fixture.direction(direction);
            fixture.settle();
            let boxes = boxes.borrow().clone();
            let runs = painted_runs(&mut fixture);
            let mut unit_ends = Vec::new();
            for (at, align) in ALIGNS.into_iter().enumerate() {
                let fields = &boxes[at * 2..at * 2 + 2];
                let values = [shown(&runs, fields[0], "5"), shown(&runs, fields[1], "100")];
                let positions = [edge(values[0].rect, fields[0], align, rtl), edge(values[1].rect, fields[1], align, rtl)];
                assert!((positions[0] - positions[1]).abs() <= 0.5, "{align:?} aligns values of different widths under {direction:?}: {positions:?}");
                if with_unit {
                    for (field, value) in fields.iter().zip(values) {
                        let unit = runs
                            .iter()
                            .find(|run| run.text != value.text && field.expand(1.0).contains_rect(run.rect))
                            .expect("a unit beside the value");
                        unit_ends.push(edge(unit.rect, *field, HAlign::End, rtl));
                        let gap = if rtl { value.rect.left() - unit.rect.right() } else { unit.rect.left() - value.rect.right() };
                        assert!(gap > 0.0, "the unit stays apart from a {align:?} value under {direction:?}");
                    }
                }
            }
            // Different edge choices must actually move the short value, not all silently centre it.
            let offsets: Vec<_> = [0, 2, 4].into_iter().map(|at| shown(&runs, boxes[at], "5").rect.left() - boxes[at].left()).collect();
            assert!(
                if rtl {
                    offsets[0] > offsets[1] + 2.0 && offsets[1] > offsets[2] + 2.0
                } else {
                    offsets[0] + 2.0 < offsets[1] && offsets[1] + 2.0 < offsets[2]
                },
                "the three alignments differ under {direction:?}: {offsets:?}"
            );
            assert!(unit_ends.iter().all(|end| (end - unit_ends[0]).abs() <= 0.5), "changing alignment leaves the unit in place: {unit_ends:?}");
        }
        let boxes: Rc<RefCell<Vec<Rect>>> = Rc::default();
        let b = boxes.clone();
        let mut fixture = Fixture::new(move |ui| {
            let mut boxes = b.borrow_mut();
            boxes.clear();
            for (at, align) in ALIGNS.into_iter().enumerate() {
                let mut mixed = Mixed::<Option<f64>>::Mixed;
                boxes.push(
                    NumberField::new("Mixed", Bind::bound_mixed(&mut mixed))
                        .range(0.0..=100.0)
                        .align(align)
                        .id_salt((at, "mixed"))
                        .show(ui)
                        .response
                        .rect,
                );
                boxes.push(
                    NumberField::new("Off", Bind::given(Some(100.0)))
                        .range(0.0..=100.0)
                        .align(align)
                        .enabled(false)
                        .id_salt((at, "off"))
                        .show(ui)
                        .response
                        .rect,
                );
            }
        });
        fixture.direction(direction);
        fixture.settle();
        let boxes = boxes.borrow().clone();
        let runs = painted_runs(&mut fixture);
        for (at, align) in ALIGNS.into_iter().enumerate() {
            let mixed = runs.iter().find(|run| boxes[at * 2].expand(1.0).contains_rect(run.rect)).expect("a mixed dash");
            let disabled = shown(&runs, boxes[at * 2 + 1], "100");
            assert!(
                (edge(mixed.rect, boxes[at * 2], align, rtl) - edge(disabled.rect, boxes[at * 2 + 1], align, rtl)).abs() <= 0.5,
                "mixed and disabled values keep {align:?} under {direction:?}"
            );
        }
        let offsets: Vec<_> = [0, 2, 4]
            .into_iter()
            .map(|at| runs.iter().find(|run| boxes[at].expand(1.0).contains_rect(run.rect)).unwrap().rect.left() - boxes[at].left())
            .collect();
        assert!(
            (offsets[0] - offsets[1]).abs() > 2.0 && (offsets[1] - offsets[2]).abs() > 2.0,
            "mixed values really follow all three alignments: {offsets:?}"
        );
    }
}

#[trial]
fn a_number_field_measures_and_paints_its_chosen_type_role() {
    let boxes: Rc<RefCell<Vec<Rect>>> = Rc::default();
    let b = boxes.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut boxes = b.borrow_mut();
        boxes.clear();
        boxes.push(NumberField::new("Default", Bind::given(Some(111.0))).sizing(Sizing::Content).id_salt("default").show(ui).response.rect);
        for (at, role) in ROLES.into_iter().enumerate() {
            boxes.push(NumberField::new("N", Bind::given(Some(111.0))).role(role).sizing(Sizing::Content).id_salt(at).show(ui).response.rect);
            Text::new(format!("{role:?} reference")).role(role).show(ui);
        }
    });
    fixture.settle();
    let boxes = boxes.borrow().clone();
    let runs = painted_runs(&mut fixture);
    let default = shown(&runs, boxes[0], "111");
    let mut rooms = Vec::new();
    for (at, role) in ROLES.into_iter().enumerate() {
        let field = boxes[at + 1];
        let value = shown(&runs, field, "111");
        let reference = runs.iter().find(|run| run.text == format!("{role:?} reference")).expect("a reference run");
        assert_eq!(value.font, reference.font, "{role:?} sets both face and size");
        rooms.push(field.width() - value.rect.width());
    }
    assert_eq!(default.font, shown(&runs, boxes[1], "111").font, "the default stays in the mono role");
    assert_ne!(shown(&runs, boxes[1], "111").font, shown(&runs, boxes[2], "111").font, "the role choice changes the actual painted font");
    assert!(
        rooms.iter().all(|room| (room - rooms[0]).abs() <= 1.0),
        "measurement uses the painted role, leaving the same padding and caret room: {rooms:?}"
    );
}

#[trial]
fn editing_keeps_the_caret_beside_the_aligned_value_in_either_reading_direction() {
    for direction in [Direction::Ltr, Direction::Rtl] {
        for align in ALIGNS {
            for role in ROLES {
                let boxes: Rc<RefCell<Vec<Rect>>> = Rc::default();
                let b = boxes.clone();
                let mut fixture = Fixture::new(move |ui| {
                    let field = NumberField::new("N", Bind::given(Some(100.0))).align(align).role(role).id_salt("n").show(ui);
                    *b.borrow_mut() = vec![field.response.rect];
                });
                fixture.direction(direction);
                fixture.settle();
                let field = boxes.borrow()[0];
                let runs = painted_runs(&mut fixture);
                let value = shown(&runs, field, "100").rect;
                assert!(fixture.focused().is_none(), "compare editing with resting paint");
                fixture.key(Key::Tab, Modifiers::NONE);
                fixture.settle();
                fixture.key(Key::End, Modifiers::NONE);
                let end = fixture.ime().expect("a caret in edit").caret;
                fixture.key(Key::Home, Modifiers::NONE);
                let home = fixture.ime().expect("a caret in edit").caret;
                assert!(
                    (end.min.x - value.right()).abs() <= 1.5 && (home.min.x - value.left()).abs() <= 1.5,
                    "editing and paint agree for {align:?}, {role:?}, {direction:?}: {value:?}, {home:?}, {end:?}"
                );
                assert!(field.expand(1.5).contains_rect(end) && field.expand(1.5).contains_rect(home), "the caret stays in its field");
            }
        }
    }
}
