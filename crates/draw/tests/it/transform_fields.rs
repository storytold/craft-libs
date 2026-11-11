//! The transform fields and the transform bar: the box maths behind each field, typed values, gestures and the bar's
//! members.

use craft_behavior::EditEvent;
use craft_behavior::geometry::quad::BoxState;
use craft_behavior::kurbo::{Point, Rect};
use craft_draw::ui::transform::{BarGroups, SizeUnit, TransformFields, TransformRow};
use craft_ui::canvas::RulerText;
use craft_ui::contract::Bind;
use craft_ui::testing::{Fixture, Target, trial};
use egui::{Key, Modifiers};
use std::cell::RefCell;
use std::rc::Rc;

/// The box before the transform, in document units.
const START: Rect = Rect::new(0.0, 0.0, 200.0, 100.0);

struct Host {
    row: TransformRow,
    linked: bool,
    enabled: bool,
    events: Vec<EditEvent<TransformRow>>,
}

struct Config {
    bar: bool,
    size_unit: SizeUnit,
    units: Option<RulerText>,
    groups: BarGroups,
}

struct Harness {
    fixture: Fixture,
    host: Rc<RefCell<Host>>,
}

fn plain() -> Config {
    Config { bar: false, size_unit: SizeUnit::default(), units: None, groups: BarGroups::ALL }
}

fn harness_of(row: TransformRow, config: Config) -> Harness {
    let host = Rc::new(RefCell::new(Host { row, linked: true, enabled: true, events: Vec::new() }));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        let mut host = shown.borrow_mut();
        let Host { row, linked, enabled, events } = &mut *host;
        let mut fields =
            TransformFields::new("Transform", Bind::bound(row), START, Bind::bound(linked)).size_unit(config.size_unit.clone()).enabled(*enabled);
        if config.bar {
            fields = fields.bar().groups(config.groups);
        }
        if let Some(units) = config.units {
            fields = fields.units(units, units);
        }
        let output = fields.show(ui);
        events.extend(output.events);
    });
    fixture.settle();
    Harness { fixture, host }
}

fn harness() -> Harness {
    harness_of(TransformRow::new(BoxState::from_rect(START), 0.0), plain())
}

impl Harness {
    fn row(&self) -> TransformRow {
        self.host.borrow().row
    }

    fn parts(&self) -> craft_behavior::geometry::quad::Parts {
        let row = self.row();
        row.state.parts(START, row.skew_v)
    }

    fn target(&self, label: &str) -> Target {
        self.fixture.by_label(label).unwrap_or_else(|| panic!("the fields show {label}"))
    }

    /// Replaces the text of the field named `label` with `text` and commits it with Enter.
    fn type_into(&mut self, label: &str, text: &str) {
        let field = self.target(label);
        self.fixture.click(&field);
        self.fixture.key(Key::A, Modifiers::COMMAND);
        self.fixture.type_text(text);
        self.fixture.key(Key::Enter, Modifiers::NONE);
        self.fixture.idle(3);
    }
}

/// Hostile text: a negative width flips the box, a huge one stops at the limit, zero collapses it without a
/// non-finite value.
#[trial]
fn hostile_typed_lengths_flip_clamp_or_collapse() {
    let mut h = harness();
    h.type_into("Width", "-50");
    assert!(h.parts().w < 0.0, "{:?}", h.parts());
    h.type_into("Width", "99999999999");
    assert!(h.parts().w.is_finite() && h.parts().w.abs() <= 10_000.0 + 1e-6, "{:?}", h.parts());
    h.type_into("Width", "0");
    let row = h.row();
    assert!(row.state.quad.iter().all(|p| p.x.is_finite() && p.y.is_finite()), "{row:?}");
}

/// Typing 50 into W of a distorted box digit by digit gives the box typing it at once gives.
#[trial]
fn typing_50_into_w_of_a_distorted_box_equals_typing_50_once_from_the_start_box() {
    let quad = [Point::new(0.0, 0.0), Point::new(200.0, 0.0), Point::new(220.0, 100.0), Point::new(-20.0, 100.0)];
    let row = TransformRow::new(BoxState::new(quad, Point::new(100.0, 50.0)), 0.0);
    let mut once = harness_of(row, plain());
    once.type_into("Width", "50");
    let mut by_digit = harness_of(row, plain());
    let field = by_digit.target("Width");
    by_digit.fixture.click(&field);
    by_digit.fixture.key(Key::A, Modifiers::COMMAND);
    by_digit.fixture.type_text("5");
    by_digit.fixture.type_text("0");
    by_digit.fixture.key(Key::Enter, Modifiers::NONE);
    by_digit.fixture.idle(3);
    let (a, b) = (once.row(), by_digit.row());
    assert!(a.state.quad.iter().zip(&b.state.quad).all(|(p, q)| p.distance(*q) < 1e-6), "{a:?} {b:?}");
}
