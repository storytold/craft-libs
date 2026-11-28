//! The transform fields and the transform bar: the box maths behind each field, typed values, gestures and the bar's
//! members.

use craft_behavior::geometry::quad::BoxState;
use craft_behavior::kurbo::{Point, Rect, Size};
use craft_behavior::{EditEvent, Mixed, RulerUnits};
use craft_draw::ui::transform::{FieldArrangement, SizeUnit, TransformField, TransformFields, TransformGroups, TransformRow, TransformSource};
use craft_ui::canvas::{CanvasRulers, RulerText};
use craft_ui::contract::Bind;
use craft_ui::runtime::env::UnitKind;
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
    relative: bool,
    events: Vec<EditEvent<TransformRow>>,
    sources: Vec<TransformSource>,
    submitted: Vec<(TransformField, craft_keymap::Mods)>,
}

struct Config {
    bar: bool,
    size_unit: SizeUnit,
    rulers: Option<CanvasRulers>,
    groups: TransformGroups,
    document: Option<Size>,
    relative: bool,
}

struct Harness {
    fixture: Fixture,
    host: Rc<RefCell<Host>>,
}

fn plain() -> Config {
    Config { bar: false, size_unit: SizeUnit::default(), rulers: None, groups: TransformGroups::ALL, document: None, relative: false }
}

fn harness_of(row: TransformRow, config: Config) -> Harness {
    let host = Rc::new(RefCell::new(Host {
        row,
        linked: true,
        enabled: true,
        relative: config.relative,
        events: Vec::new(),
        sources: Vec::new(),
        submitted: Vec::new(),
    }));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        let mut host = shown.borrow_mut();
        let Host { row, linked, enabled, relative, events, sources, submitted } = &mut *host;
        let mut fields =
            TransformFields::new("Transform", Bind::bound(row), START, Bind::bound(linked)).size_unit(config.size_unit.clone()).enabled(*enabled);
        if config.bar {
            fields = fields.bar().groups(config.groups);
        }
        if let Some(rulers) = config.rulers {
            fields = fields.rulers(rulers);
        }
        if let Some(document) = config.document {
            fields = fields.document(document);
        }
        if config.relative {
            fields = fields.relative(Bind::bound(relative));
        }
        let output = fields.show(ui);
        events.extend(output.events);
        sources.extend(output.sources);
        submitted.extend(output.submitted);
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

/// A typed percentage in X and Y is read against the document's sides, in W against the box's.
#[trial]
fn x_and_y_read_a_typed_percent_against_the_document() {
    let config = Config { document: Some(Size::new(1000.0, 400.0)), ..plain() };
    let mut h = harness_of(TransformRow::new(BoxState::from_rect(START), 0.0), config);
    h.type_into("X position", "50%");
    h.type_into("Y position", "50%");
    h.type_into("Width", "50%");
    let row = h.row();
    assert!((row.state.pivot.x - 500.0).abs() < 1e-6 && (row.state.pivot.y - 200.0).abs() < 1e-6, "{row:?}");
    let width = row.state.quad[0].distance(row.state.quad[1]);
    assert!((width - 100.0).abs() < 1e-6, "{row:?}");
}

/// With the Δ toggle on, X and Y read and move the reference point's offset.
#[trial]
fn relative_x_and_y_read_and_move_by_the_reference_points_offset() {
    let relative = || Config { bar: true, relative: true, ..plain() };
    let start = TransformRow::new(BoxState::from_rect(START), 0.0);
    let mut h = harness_of(start, relative());
    // A fresh box reads 0: typing 0 moves nothing, where an absolute X would move the box to the origin.
    h.type_into("X position", "0");
    assert_eq!(h.row().state, start.state, "a fresh box reads an offset of 0");
    h.type_into("X position", "10");
    let moved = h.row();
    assert!((moved.state.pivot.x - start.state.pivot.x - 10.0).abs() < 1e-9, "{moved:?}");
    assert!(moved.state.quad.iter().zip(&start.state.quad).all(|(p, q)| (p.x - q.x - 10.0).abs() < 1e-9 && (p.y - q.y).abs() < 1e-9), "{moved:?}");
    h.type_into("X position", "10");
    assert_eq!(h.row().state, moved.state, "the offset now reads 10, so typing 10 moves nothing");

    // A fractional Y that the field cannot hold stays exactly where it was when only X is typed.
    let shifted = BoxState::new(start.state.quad.map(|p| Point::new(p.x, p.y + 0.1)), Point::new(start.state.pivot.x, start.state.pivot.y + 0.1));
    let mut h = harness_of(TransformRow::new(shifted, 0.0), relative());
    h.type_into("X position", "20");
    let after = h.row();
    assert!(after.state.quad.iter().zip(&shifted.quad).all(|(p, q)| p.y == q.y), "{after:?}");
    assert_eq!(after.state.pivot.y, shifted.pivot.y);

    // A box flattened to zero height still reads a finite offset.
    let flat =
        BoxState::new([Point::new(0.0, 50.0), Point::new(200.0, 50.0), Point::new(200.0, 50.0), Point::new(0.0, 50.0)], Point::new(100.0, 50.0));
    let mut h = harness_of(TransformRow::new(flat, 0.0), relative());
    h.type_into("X position", "5");
    let row = h.row();
    assert!(row.state.pivot.x.is_finite() && row.state.quad.iter().all(|p| p.x.is_finite() && p.y.is_finite()), "{row:?}");
}

/// An Alt+Enter in W with the link on reports W and Alt, and scales H by W's factor.
#[trial]
fn alt_enter_in_w_with_the_link_on_reports_w_alt_and_scales_h() {
    let mut h = harness();
    let start = h.parts();
    let field = h.target("Width");
    h.fixture.click(&field);
    h.fixture.key(Key::A, Modifiers::COMMAND);
    h.fixture.type_text("50");
    h.fixture.key(Key::Enter, Modifiers::ALT);
    h.fixture.idle(3);
    let host = h.host.borrow();
    assert!(!host.events.is_empty());
    assert_eq!(host.sources.len(), host.events.len(), "one source per event");
    assert!(host.sources.iter().all(|source| *source == TransformSource::Field(TransformField::W)), "{:?}", host.sources);
    assert_eq!(host.submitted.len(), 1, "{:?}", host.submitted);
    assert_eq!(host.submitted[0].0, TransformField::W);
    assert!(host.submitted[0].1.alt, "{:?}", host.submitted);
    drop(host);
    let after = h.parts();
    assert!(after.w != start.w, "W changed");
    assert!((after.h / start.h - after.w / start.w).abs() < 1e-9, "H follows W's factor: {start:?} {after:?}");
}

/// X and Y count from the rulers' zero; a relative offset ignores it.
#[trial]
fn x_and_y_count_from_the_rulers_zero_and_a_relative_offset_ignores_it() {
    let px = RulerText::new(RulerUnits::pixels(), UnitKind::Pixels);
    let rulers = Some(CanvasRulers::new(px, px, Point::new(40.0, 10.0)));
    let start = TransformRow::new(BoxState::from_rect(START), 0.0);
    let mut h = harness_of(start, Config { rulers, ..plain() });
    // X reads the pivot less the zero (60): typing what it reads moves nothing.
    h.type_into("X position", "60");
    h.type_into("Y position", "40");
    assert_eq!(h.row().state, start.state, "typing the value the field reads moves nothing");
    h.type_into("X position", "70");
    h.type_into("Y position", "45");
    let moved = h.row().state;
    assert!((moved.pivot.x - start.state.pivot.x - 10.0).abs() < 1e-9 && (moved.pivot.y - start.state.pivot.y - 5.0).abs() < 1e-9, "{moved:?}");

    let mut h = harness_of(start, Config { bar: true, relative: true, rulers, ..plain() });
    h.type_into("X position", "0");
    assert_eq!(h.row().state, start.state, "a relative offset reads 0 whatever the zero");
    h.type_into("X position", "10");
    assert!((h.row().state.pivot.x - start.state.pivot.x - 10.0).abs() < 1e-9);
}

/// The texts the fields paint for a row binding that reads `read`; `None` is a binding with nothing in it.
fn texts_of(read: Option<Mixed<TransformRow>>) -> Vec<String> {
    let mut linked = true;
    let mut fixture = Fixture::new(move |ui| {
        let row = match read {
            Some(read) => Bind::given(read),
            None => Bind::kept(),
        };
        TransformFields::new("Transform", row, START, Bind::bound(&mut linked)).show(ui);
    });
    fixture.settle();
    fixture.texts().iter().map(|painted| painted.text.clone()).collect()
}

/// A row that reads nothing shows blank fields; a mixed row shows the start box.
#[trial]
fn a_row_that_reads_no_value_shows_blank_fields_and_a_mixed_row_shows_the_start_box() {
    let has_number = |texts: &[String]| texts.iter().any(|text| text.chars().any(|c| c.is_ascii_digit()));
    let start = TransformRow::new(BoxState::from_rect(START), 0.0);
    let blank = texts_of(None);
    let mixed = texts_of(Some(Mixed::Mixed));
    assert!(!has_number(&blank), "{blank:?}");
    assert!(has_number(&mixed), "{mixed:?}");
    assert_eq!(mixed, texts_of(Some(Mixed::Same(start))));
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

/// A host that passes the box as it is now as the start box, so the start box moves with a scrub's own values, keeps
/// the scrub running to its release: the field reads it against the start box it began with. Guards against a field
/// that read the running scrub's value against each pass's start box, so the host's write-back looked like an outside
/// change and cancelled it (`HostChanged`).
#[trial]
fn a_start_box_that_moves_with_a_scrub_keeps_the_scrub_running() {
    let host = Rc::new(RefCell::new((TransformRow::new(BoxState::from_rect(START), 0.0), Vec::new())));
    let shown = Rc::clone(&host);
    let mut linked = true;
    let mut fixture = Fixture::new(move |ui| {
        let (row, events) = &mut *shown.borrow_mut();
        let start = Rect::from_points(row.state.quad[0], row.state.quad[2]);
        events.extend(TransformFields::new("Transform", Bind::bound(row), start, Bind::bound(&mut linked)).show(ui).events);
    });
    fixture.record_paint().settle();
    let glyph = fixture.texts().iter().find(|painted| painted.text == "W").map(|painted| painted.rect.center()).expect("W's caption shows");
    fixture.drag_at(glyph, egui::vec2(40.0, 0.0));
    let (row, events) = &*host.borrow();
    assert!(events.iter().any(|event| matches!(event, EditEvent::Update { .. })), "the scrub ran: {events:?}");
    assert!(!events.iter().any(|event| matches!(event, EditEvent::Cancel { .. })), "the scrub was not cancelled: {events:?}");
    assert!(events.iter().any(|event| matches!(event, EditEvent::Commit { .. })), "the release committed it: {events:?}");
    assert!(row.state.quad[0].distance(row.state.quad[1]) > START.width(), "the scrub widened the box: {row:?}");
}

/// The position-first panel form stands the reference point before the X and W, Y and H rows and the link after them
/// centred across the rows, both clear of every field, in a wide room, a narrow one and a host sized from its content.
#[trial]
fn the_position_first_link_stands_between_the_width_and_height_rows() {
    use craft_ui::contract::ShowContainer as _;
    for (width, content) in [(260.0, false), (400.0, false), (400.0, true)] {
        let mut row = TransformRow::new(BoxState::from_rect(START), 0.0);
        let mut linked = true;
        let mut fixture = Fixture::new(move |ui| {
            let mut show = |ui: &mut egui::Ui| {
                TransformFields::new("Transform", Bind::bound(&mut row), START, Bind::bound(&mut linked))
                    .arrangement(FieldArrangement::PositionFirst)
                    .show(ui)
            };
            if content {
                craft_ui::layout_components::Stack::new().show(ui, |ui, _| show(ui));
            } else {
                show(ui);
            }
        });
        fixture.room(egui::vec2(width, 400.0)).settle();
        let rect = |label: &str| fixture.by_label(label).unwrap_or_else(|| panic!("the fields show {label}")).rect;
        let link = rect("Maintain aspect ratio");
        let reference = rect("Reference point");
        let fields = ["X position", "Width", "Y position", "Height"].map(rect);
        let dump = format!("in {width} (content: {content}): reference {reference:?}, link {link:?}, fields {fields:?}");
        assert!(
            fields.iter().all(|field| !field.intersects(link) && !field.intersects(reference)),
            "no field overlaps the link or the point: {dump}"
        );
        let [x, w, _, h] = fields;
        assert!(
            reference.right() <= x.left() && w.right() <= link.left() && link.right() <= width,
            "point, fields, link in order, inside the room: {dump}"
        );
        assert!(h.top() >= w.bottom(), "H is under W: {dump}");
        let middle = (w.top() + h.bottom()) / 2.0;
        assert!((link.center().y - middle).abs() <= 1.0, "the link is centred across the two rows: {dump}");
    }
}
