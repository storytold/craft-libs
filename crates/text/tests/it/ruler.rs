//! The text ruler's keys: the ruler is one Tab stop, Page Down moves between its markers and stops, and the arrows
//! move the focused one. Alt in a column border drag shows the column widths.

use craft_text::ui::ruler::{RulerItem, RulerValues, TabAlign, TabLeader, TabStop, TextRuler};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Props as _, Role, Show as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Which of `labels` holds the focus.
fn focused<'a>(fixture: &Fixture, labels: &[&'a str]) -> Option<&'a str> {
    let focused = craft_ui::testing::access::focused_key(fixture.tree());
    labels.iter().copied().find(|label| fixture.by_label(label).is_some_and(|target| target.node == focused))
}

#[trial]
fn the_ruler_is_one_tab_stop_whose_page_keys_pick_and_arrows_move() {
    let start = RulerValues { stops: vec![TabStop::new(72.0, TabAlign::Left, TabLeader::None)], ..Default::default() };
    let values = Rc::new(RefCell::new(start));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        Stack::new().show(ui, |ui, _| {
            Button::new("Before").show(ui);
            TextRuler::new("Ruler", Bind::bound(&mut *shown.borrow_mut()), 300.0).show(ui);
            Button::new("After").show(ui);
        });
    });
    fixture.idle(4);
    let labels = ["Before", "First Line Indent", "Hanging Indent", "After"];
    fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    assert_eq!(focused(&fixture, &labels), Some("Before"), "Tab reaches the first button");
    fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    assert_eq!(focused(&fixture, &labels), Some("First Line Indent"), "Tab enters the ruler at its first marker");
    fixture.key(Key::PageDown, Modifiers::NONE).idle(2);
    assert_eq!(focused(&fixture, &labels), Some("Hanging Indent"), "Page Down moves to the next marker");
    let before_move = values.borrow().clone();
    fixture.key(Key::ArrowRight, Modifiers::NONE).idle(2);
    assert_ne!(*values.borrow(), before_move, "Right moves the focused marker");
    assert_eq!(focused(&fixture, &labels), Some("Hanging Indent"), "Right keeps the focus on the marker");
    fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    assert_eq!(focused(&fixture, &labels), Some("After"), "Tab leaves the ruler: its markers and stops are one stop");
}

/// While a column border drag runs, holding Alt writes the widths of the columns between the borders.
#[trial]
fn alt_in_a_column_border_drag_shows_the_column_widths() {
    let values = Rc::new(RefCell::new(RulerValues::default()));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        TextRuler::new("Ruler", Bind::bound(&mut *shown.borrow_mut()), 300.0).columns(&[50.0, 150.0, 250.0]).show(ui);
    });
    fixture.settle();
    let border = fixture.by_label("Table Column Width").expect("a column border").center();
    fixture.drag_hold_at(border, Vec2::new(6.0, 0.0)).idle(2);
    let without = fixture.texts().len();
    fixture.hold(Modifiers::ALT).idle(2);
    let with = fixture.texts().len();
    assert!(with > without, "Alt adds the widths to the strip ({without} runs without, {with} with)");
    fixture.hold(Modifiers::NONE).idle(2);
    assert_eq!(fixture.texts().len(), without, "letting Alt go hides them");
    fixture.release_at(border + Vec2::new(6.0, 0.0)).settle();
}

/// The centres of the left stops' sliders, left to right.
fn stop_centres(fixture: &Fixture) -> Vec<egui::Pos2> {
    let mut found: Vec<egui::Pos2> = fixture
        .by_role(Role::SLIDER)
        .into_iter()
        .filter(|target| fixture.tree().name_of(target.node).is_some_and(|name| name.starts_with("Left Tab at")))
        .map(|target| target.center())
        .collect();
    found.sort_by(|a, b| a.x.total_cmp(&b.x));
    found
}

#[trial]
fn a_command_drag_moves_every_stop_and_keeps_their_spacing_within_the_column() {
    let id = egui::Id::new("ruler");
    let stops = [72.0, 144.0, 216.0].map(|at| TabStop::new(at, TabAlign::Left, TabLeader::None)).to_vec();
    let values = Rc::new(RefCell::new(RulerValues { stops, ..Default::default() }));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        TextRuler::new("Ruler", Bind::bound(&mut *shown.borrow_mut()), 300.0).id(id).show(ui);
    });
    fixture.settle();
    let grabbed = stop_centres(&fixture)[1];
    let by = Vec2::new(600.0, 0.0);
    fixture.hold(Modifiers::COMMAND);
    fixture.drag_hold_at(grabbed, by).idle(2);
    let drag = TextRuler::dragging(fixture.ctx(), id).expect("the drag is running");
    assert_eq!(drag.item, RulerItem::Stop(1), "the grabbed stop keeps its index");
    assert!((drag.position - 228.0).abs() < 0.01, "the highest stop stops at the column width: {}", drag.position);
    fixture.release_at(grabbed + by).settle();
    fixture.hold(Modifiers::NONE).idle(2);
    let moved: Vec<f32> = values.borrow().stops.iter().map(|stop| stop.position).collect();
    assert_eq!(moved.len(), 3);
    for (got, want) in moved.iter().zip([156.0, 228.0, 300.0]) {
        assert!((got - want).abs() < 0.01, "every stop moved by one offset: {moved:?}");
    }
    assert_eq!(TextRuler::dragging(fixture.ctx(), id), None, "no drag after the release");
}

#[trial]
fn dragging_is_none_off_the_ruler_after_escape_and_once_the_ruler_stops_showing() {
    let id = egui::Id::new("ruler");
    let values = Rc::new(RefCell::new(RulerValues {
        stops: vec![TabStop::new(72.0, TabAlign::Left, TabLeader::None)],
        ..Default::default()
    }));
    let (shown, visible) = (values.clone(), Rc::new(Cell::new(true)));
    let show = visible.clone();
    let mut fixture = Fixture::new(move |ui| {
        if show.get() {
            TextRuler::new("Ruler", Bind::bound(&mut *shown.borrow_mut()), 300.0).id(id).show(ui);
        }
    });
    fixture.settle();
    let from = stop_centres(&fixture)[0];
    fixture.drag_hold_at(from, Vec2::new(20.0, 0.0)).idle(2);
    assert!(TextRuler::dragging(fixture.ctx(), id).is_some(), "a drag on the ruler is reported");
    fixture.frame(vec![egui::Event::PointerMoved(from + Vec2::new(20.0, 200.0))]).idle(1);
    assert_eq!(TextRuler::dragging(fixture.ctx(), id), None, "off the ruler");
    fixture.release_at(from + Vec2::new(20.0, 200.0)).settle();

    let from = stop_centres(&fixture)[0];
    fixture.drag_hold_at(from, Vec2::new(20.0, 0.0)).idle(2);
    assert!(TextRuler::dragging(fixture.ctx(), id).is_some());
    fixture.key(egui::Key::Escape, Modifiers::NONE).idle(2);
    assert_eq!(TextRuler::dragging(fixture.ctx(), id), None, "after Escape");
    fixture.release_at(from + Vec2::new(20.0, 0.0)).settle();

    let from = stop_centres(&fixture)[0];
    fixture.drag_hold_at(from, Vec2::new(20.0, 0.0)).idle(2);
    assert!(TextRuler::dragging(fixture.ctx(), id).is_some());
    visible.set(false);
    fixture.idle(2);
    assert_eq!(TextRuler::dragging(fixture.ctx(), id), None, "once the ruler stops showing");
}
