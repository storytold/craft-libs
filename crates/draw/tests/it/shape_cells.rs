//! The shape cells: the arrowhead rows behind the popover and the Shape panel.

use craft_behavior::EditEvent;
use craft_color::ui::Paint;
use craft_draw::ui::{Arrowheads, ArrowheadsPopover, LiveShape, ShapeAppearance, ShapeChange, ShapePanel};
use craft_ui::contract::Bind;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::RangeAlert;
use egui::{Key, Modifiers, vec2};
use std::cell::RefCell;
use std::rc::Rc;

fn start_value() -> Arrowheads {
    let mut arrows = Arrowheads::new();
    arrows.start = true;
    arrows.end = true;
    arrows.width = 6.0;
    arrows.length = 12.0;
    arrows.concavity = 25.0;
    arrows
}

/// A scrub of Width is one gesture, `Begin` to `Commit`, and every value it carries is the whole `from` with only the
/// width replaced; the gesture ends on another width.
fn assert_width_scrub<T>(events: &[EditEvent<T>], whole: impl Fn(&T) -> Option<Arrowheads>, from: Arrowheads, source: &str) {
    assert!(matches!(events.first(), Some(EditEvent::Begin { .. })), "{source}: the scrub begins");
    assert!(matches!(events.last(), Some(EditEvent::Commit { .. })), "{source}: the scrub commits");
    let mut last = None;
    for event in events {
        let Some(value) = event.value() else { continue };
        let arrows = whole(value).unwrap_or_else(|| panic!("{source}: every event carries the whole Arrowheads"));
        assert_eq!((arrows.start, arrows.end), (from.start, from.end), "{source}: the ends stay");
        assert_eq!((arrows.length, arrows.concavity), (from.length, from.concavity), "{source}: length and concavity stay");
        last = Some(arrows);
    }
    let last = last.unwrap_or_else(|| panic!("{source}: the scrub reports a value"));
    assert_ne!(last.width, from.width, "{source}: the width moved");
}

struct PopoverHost {
    value: Arrowheads,
    open: bool,
    events: Vec<EditEvent<Arrowheads>>,
}

struct PanelHost {
    shape: LiveShape,
    look: ShapeAppearance,
    events: Vec<EditEvent<ShapeChange>>,
}

#[trial]
fn an_arrowhead_row_scrub_reports_the_whole_value_from_the_popover_and_the_panel() {
    // The open popover.
    let host = Rc::new(RefCell::new(PopoverHost { value: start_value(), open: false, events: Vec::new() }));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        let mut held = shown.borrow_mut();
        let PopoverHost { value, open, events } = &mut *held;
        let out = ArrowheadsPopover::new(Bind::given(*value), Bind::bound(open), 72.0).show(ui);
        for event in out.edits {
            if let Some(next) = event.value() {
                *value = *next;
            }
            events.push(event);
        }
    });
    fixture.settle();
    host.borrow_mut().open = true;
    fixture.idle(3);
    let width = fixture.by_label("Width").expect("the open popover shows the Width field");
    fixture.drag(&width, vec2(40.0, 0.0));
    assert_width_scrub(&host.borrow().events, |value| Some(*value), start_value(), "popover");

    // The panel on a line with arrowheads.
    let host = Rc::new(RefCell::new(PanelHost {
        shape: LiveShape::arrow_line(3.0, start_value()),
        look: ShapeAppearance::new(Paint::None, Paint::None, 2.0),
        events: Vec::new(),
    }));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        let mut held = shown.borrow_mut();
        let PanelHost { shape, look, events } = &mut *held;
        let out = ShapePanel::new("Shape properties", shape, look, 72.0).show(ui);
        for event in out.events {
            if let Some(ShapeChange::Arrowheads(next)) = event.value() {
                *shape = LiveShape::arrow_line(3.0, *next);
            }
            events.push(event);
        }
    });
    fixture.settle();
    let width = fixture.by_label("Width").expect("the panel shows the arrowhead Width field");
    fixture.drag(&width, vec2(40.0, 0.0));
    let whole = |change: &ShapeChange| match change {
        ShapeChange::Arrowheads(arrows) => Some(*arrows),
        _ => None,
    };
    assert_width_scrub(&host.borrow().events, whole, start_value(), "panel");
}

/// Types `text` over the field named `label` and commits it with Enter.
fn type_into(fixture: &mut Fixture, label: &str, text: &str) {
    let field = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} field"));
    fixture.click(&field).settle();
    fixture.key(Key::A, Modifiers::COMMAND);
    if text.is_empty() {
        fixture.key(Key::Backspace, Modifiers::NONE);
    } else {
        fixture.type_text(text);
    }
    fixture.key(Key::Enter, Modifiers::NONE).settle();
}

struct AlertingPopover {
    value: Arrowheads,
    open: bool,
    alerts: Vec<RangeAlert>,
}

struct AlertingPanel {
    shape: LiveShape,
    look: ShapeAppearance,
    alerts: Vec<RangeAlert>,
}

#[trial]
fn a_concavity_typed_past_its_range_keeps_the_end_and_reports_it_once_from_the_popover_and_the_panel() {
    let host = Rc::new(RefCell::new(AlertingPopover { value: start_value(), open: false, alerts: Vec::new() }));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        let mut held = shown.borrow_mut();
        let AlertingPopover { value, open, alerts } = &mut *held;
        let out = ArrowheadsPopover::new(Bind::given(*value), Bind::bound(open), 72.0).show(ui);
        if let Some(next) = out.edits.iter().rev().find_map(|event| event.value()) {
            *value = *next;
        }
        alerts.extend(out.out_of_range);
    });
    fixture.settle();
    host.borrow_mut().open = true;
    fixture.idle(3);
    type_into(&mut fixture, "Concavity", "80");
    assert_eq!(host.borrow().value.concavity, 50.0, "popover: a concavity past 50 keeps 50");
    assert_eq!(host.borrow().alerts.len(), 1, "popover: the clamp is reported once");
    type_into(&mut fixture, "Concavity", "-20");
    assert_eq!(host.borrow().value.concavity, -20.0, "popover: a concavity below 0 is in range");
    assert_eq!(host.borrow().alerts.len(), 1, "popover: a value in range reports nothing");

    let host = Rc::new(RefCell::new(AlertingPanel {
        shape: LiveShape::arrow_line(3.0, start_value()),
        look: ShapeAppearance::new(Paint::None, Paint::None, 2.0),
        alerts: Vec::new(),
    }));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        let mut held = shown.borrow_mut();
        let AlertingPanel { shape, look, alerts } = &mut *held;
        let out = ShapePanel::new("Shape properties", shape, look, 72.0).show(ui);
        if let Some(ShapeChange::Arrowheads(next)) = out.events.iter().rev().find_map(|event| event.value()) {
            *shape = LiveShape::arrow_line(3.0, *next);
        }
        alerts.extend(out.out_of_range);
    });
    fixture.settle();
    type_into(&mut fixture, "Concavity", "80");
    let mut kept = start_value();
    kept.concavity = 50.0;
    assert_eq!(host.borrow().shape, LiveShape::arrow_line(3.0, kept), "panel: 80 keeps 50");
    assert_eq!(host.borrow().alerts.len(), 1, "panel: the clamp is reported once");
}
