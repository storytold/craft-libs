//! Labelled marks: the captions under a slider's marks sit apart and inside the slider, however long they are.

use craft_behavior::Mixed;
use craft_i18n::Direction;
use craft_ui::contract::{Bind, Props as _, Role, Show as _, ShowContainer as _};
use craft_ui::layout_components::Row;
use craft_ui::testing::{Fixture, trial};
use craft_ui::value_drag::{RangeSlider, Slider};
use egui::{Pos2, Rect, Vec2};

const SLACK: f32 = 0.5;

/// The two ends of a contract-or-expand slider, as long as they run in some languages.
const ENDS: [(f64, &str); 2] = [(-200.0, "Zusammenziehen"), (200.0, "Aufblasen")];

/// How a slider is placed: in a row in a room this wide, which shrinks it to its least width, or at this rect.
#[derive(Clone, Copy)]
enum Place {
    Row(f32),
    At(Rect),
}

/// Which slider shows the labels.
#[derive(Clone, Copy)]
enum Kind {
    Slider,
    Range,
}

/// The rect of the slider and the painted rect of each of `labels`, in their order, for `kind` with `labels` as its
/// labelled marks, placed by `place` in `direction`.
fn shown(kind: Kind, labels: &'static [(f64, &'static str)], place: Place, direction: Direction) -> (Rect, Vec<Rect>) {
    let range = labels.iter().map(|(value, _)| *value).fold(f64::INFINITY, f64::min)
        ..=labels.iter().map(|(value, _)| *value).fold(f64::NEG_INFINITY, f64::max);
    let mut fixture = Fixture::new(move |ui| {
        let show = |ui: &mut egui::Ui| match kind {
            Kind::Slider => {
                let mut value = 0.0;
                let slider = Slider::new(Bind::bound(&mut value), range.clone()).marks_labelled(labels.iter().copied()).label("Amount");
                match place {
                    Place::Row(_) => drop(slider.show(ui)),
                    Place::At(rect) => drop(slider.show_at(ui, rect)),
                }
            }
            Kind::Range => {
                let mut pair = (Mixed::Same(*range.start()), Mixed::Same(*range.end()));
                let slider = RangeSlider::new(Bind::bound(&mut pair), range.clone()).marks_labelled(labels.iter().copied()).label("Amount");
                match place {
                    Place::Row(_) => drop(slider.show(ui)),
                    Place::At(rect) => drop(slider.show_at(ui, rect)),
                }
            }
        };
        match place {
            Place::Row(_) => drop(Row::new().show(ui, |ui, _| show(ui))),
            Place::At(_) => show(ui),
        }
    });
    let room = match place {
        Place::Row(width) => Vec2::new(width, 400.0),
        Place::At(_) => Vec2::new(800.0, 400.0),
    };
    fixture.direction(direction).room(room).settle();
    let role = match kind {
        Kind::Slider => Role::SLIDER,
        Kind::Range => Role::GROUP,
    };
    let slider = fixture.by_label("Amount").or_else(|| fixture.by_role(role).into_iter().next()).expect("the slider").rect;
    let shapes = fixture.frame_shapes();
    let rects = labels
        .iter()
        .map(|(_, label)| {
            shapes
                .iter()
                .find_map(|shape| match shape {
                    egui::Shape::Text(text) if text.galley.text() == *label => Some(text.galley.rect.translate(text.pos.to_vec2())),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("the label {label:?} is painted"))
        })
        .collect();
    (slider, rects)
}

/// Every label lies inside the slider and clear of every other label.
fn assert_apart_inside(slider: Rect, labels: &[Rect], case: &str) {
    for label in labels {
        assert!(
            label.left() >= slider.left() - SLACK && label.right() <= slider.right() + SLACK,
            "{case}: the label {label:?} lies inside {slider:?}"
        );
    }
    for (i, a) in labels.iter().enumerate() {
        for b in &labels[i + 1..] {
            assert!(a.right() <= b.left() + SLACK || b.right() <= a.left() + SLACK, "{case}: the labels {a:?} and {b:?} overlap");
        }
    }
}

/// A row that shrinks a slider with two long end labels keeps it wide enough for both, so they lie apart and inside
/// it, in either reading direction, on a slider and on a range slider.
#[trial]
fn long_end_labels_widen_a_slider_a_row_shrinks() {
    for direction in [Direction::Ltr, Direction::Rtl] {
        for kind in [Kind::Slider, Kind::Range] {
            let (slider, labels) = shown(kind, &ENDS, Place::Row(60.0), direction);
            assert_apart_inside(slider, &labels, &format!("{direction:?}"));
            let (first, last) = if direction == Direction::Rtl { (labels[1], labels[0]) } else { (labels[0], labels[1]) };
            assert!(first.left() < last.left(), "{direction:?}: the labels keep the side of their marks: {labels:?}");
        }
    }
}

/// A label between the ends moves aside where centring it under its mark would cover a long end label, in either
/// reading direction.
#[trial]
fn a_middle_label_moves_clear_of_a_long_end_label() {
    const MARKS: [(f64, &str); 3] = [(0.0, "Fully contracted"), (50.0, "0"), (100.0, "Up")];
    let (_, wide) = shown(Kind::Slider, &MARKS, Place::At(Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::new(600.0, 40.0))), Direction::Ltr);
    let width: f32 = wide.iter().map(|label| label.width()).sum::<f32>() + 24.0;
    for direction in [Direction::Ltr, Direction::Rtl] {
        let rect = Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::new(width, 40.0));
        let (slider, labels) = shown(Kind::Slider, &MARKS, Place::At(rect), direction);
        assert_apart_inside(slider, &labels, &format!("{direction:?} at {width}"));
    }
}

/// Labels that share one mark lie side by side, apart and inside the slider, in either reading direction.
#[trial]
fn labels_that_share_a_mark_lie_side_by_side() {
    const MARKS: [(f64, &str); 5] = [(0.0, "Start"), (50.0, "Alpha"), (50.0, "Beta"), (50.0, "Gamma"), (100.0, "End")];
    for direction in [Direction::Ltr, Direction::Rtl] {
        let (slider, labels) = shown(Kind::Slider, &MARKS, Place::Row(60.0), direction);
        assert_apart_inside(slider, &labels, &format!("{direction:?}"));
    }
}

/// Labels of nearby marks keep the order of their marks when they move apart, though the later one is far longer.
#[trial]
fn labels_of_nearby_marks_keep_the_order_of_their_marks() {
    const MARKS: [(f64, &str); 4] = [(0.0, "0"), (25.0, "Ab"), (30.0, "A much longer caption"), (100.0, "End")];
    let rect = Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::new(400.0, 40.0));
    for direction in [Direction::Ltr, Direction::Rtl] {
        let (slider, labels) = shown(Kind::Slider, &MARKS, Place::At(rect), direction);
        assert_apart_inside(slider, &labels, &format!("{direction:?}"));
        let (first, last) = if direction == Direction::Rtl { (labels[2], labels[1]) } else { (labels[1], labels[2]) };
        assert!(first.left() < last.left(), "{direction:?}: the labels keep the order of their marks: {labels:?}");
    }
}

/// Short end labels leave a slider as wide as an unlabelled one where the room allows.
#[trial]
fn short_end_labels_leave_the_slider_width_as_it_was() {
    const SHORT: [(f64, &str); 2] = [(0.0, "0"), (100.0, "100")];
    let (labelled, _) = shown(Kind::Slider, &SHORT, Place::Row(800.0), Direction::Ltr);
    let mut fixture = Fixture::new(|ui| {
        Row::new().show(ui, |ui, _| {
            let mut value = 0.0;
            Slider::new(Bind::bound(&mut value), 0.0..=100.0).label("Amount").show(ui);
        });
    });
    fixture.room(Vec2::new(800.0, 400.0)).settle();
    let bare = fixture.by_label("Amount").expect("the slider").rect;
    assert!((labelled.width() - bare.width()).abs() <= SLACK, "short labels keep the width: {labelled:?}, unlabelled {bare:?}");
}
