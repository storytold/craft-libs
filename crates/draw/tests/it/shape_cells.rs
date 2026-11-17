//! The shape cells: the arrowhead rows behind the popover and the Shape panel, the panel's rows by shape, and the
//! Rectangle's gear.

use craft_behavior::EditEvent;
use craft_color_ui::ui::Paint;
use craft_draw::ui::{
    Arrowheads, ArrowheadsPopover, BoxConstraint, BoxConstraints, BoxConstraintsPopover, LiveShape, ShapeAppearance, ShapeBar, ShapeChange,
    ShapeKind, ShapePanel, ShapeToolSettings,
};
use craft_ui::bars::{Bar, BarKind};
use craft_ui::contract::{Bind, ShowContainer as _};
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
fn a_concavity_or_stroke_width_typed_past_its_range_keeps_the_end_and_reports_it_once() {
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

    // The Shape bar's stroke width (StrokeCells, which the Pen tools' bar shares) reports its clamp the same way.
    let host = Rc::new(RefCell::new(AlertingPanel {
        shape: LiveShape::arrow_line(3.0, start_value()),
        look: ShapeAppearance::new(Paint::None, Paint::None, 2.0),
        alerts: Vec::new(),
    }));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        let mut held = shown.borrow_mut();
        let AlertingPanel { look, alerts, .. } = &mut *held;
        let settings = ShapeToolSettings::default();
        let out =
            Bar::new("Shape options", BarKind::Options).show(ui, |ui, _| ShapeBar::new(ShapeKind::Rectangle, &settings, look, 72.0).show(ui)).inner;
        let Some(out) = out else { return };
        if let Some(ShapeChange::StrokeWidth(width)) = out.events.iter().rev().find_map(|event| event.value()) {
            look.stroke_width = *width;
        }
        alerts.extend(out.out_of_range);
    });
    fixture.settle();
    type_into(&mut fixture, "Stroke width", "400");
    assert_eq!(host.borrow().look.stroke_width, 288.0, "bar: a stroke width past 288 px keeps 288");
    assert_eq!(host.borrow().alerts.len(), 1, "bar: the clamp is reported once");
    type_into(&mut fixture, "Stroke width", "12");
    assert_eq!(host.borrow().look.stroke_width, 12.0, "bar: a stroke width in range");
    assert_eq!(host.borrow().alerts.len(), 1, "bar: a value in range reports nothing");
}

#[trial]
fn a_triangle_shows_its_size_and_one_corner_radius() {
    let host = Rc::new(RefCell::new(PanelHost {
        shape: LiveShape::triangle(240.0, 200.0, 0.0),
        look: ShapeAppearance::new(Paint::None, Paint::None, 2.0),
        events: Vec::new(),
    }));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        let mut held = shown.borrow_mut();
        let PanelHost { shape, look, events } = &mut *held;
        events.extend(ShapePanel::new("Shape properties", shape, look, 72.0).show(ui).events);
    });
    fixture.settle();
    for shown in ["Width", "Height", "Corner radius"] {
        assert!(fixture.by_label(shown).is_some(), "a triangle shows {shown}");
    }
    for absent in ["Sides", "Star ratio", "Top Left"] {
        assert!(fixture.by_label(absent).is_none(), "a triangle has no {absent} field");
    }
    type_into(&mut fixture, "Corner radius", "12");
    let radius = host.borrow().events.iter().rev().find_map(|event| event.value().cloned());
    assert_eq!(radius, Some(ShapeChange::CornerRadius(12.0)), "the radius is one radius for every corner");
}

struct GearHost {
    value: BoxConstraints,
    open: bool,
    alerts: Vec<RangeAlert>,
}

/// A fixture with the gear open over `value`.
fn gear(value: BoxConstraints) -> (Fixture, Rc<RefCell<GearHost>>) {
    let host = Rc::new(RefCell::new(GearHost { value, open: false, alerts: Vec::new() }));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        let mut held = shown.borrow_mut();
        let GearHost { value, open, alerts } = &mut *held;
        let out = BoxConstraintsPopover::new(Bind::given(*value), Bind::bound(open), 72.0).show(ui);
        if let Some(next) = out.edits.iter().rev().find_map(|event| event.value()) {
            *value = *next;
        }
        alerts.extend(out.out_of_range);
    });
    fixture.settle();
    host.borrow_mut().open = true;
    fixture.idle(3);
    (fixture, host)
}

fn disabled(fixture: &Fixture, label: &str) -> bool {
    let field = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} field"));
    fixture.tree().is_disabled_key(field.node)
}

#[trial]
fn the_gear_enables_only_the_chosen_sizes_and_escape_keeps_the_pick() {
    let (mut fixture, host) = gear(BoxConstraints::default());
    for field in ["Fixed Size W", "Fixed Size H", "Proportional W", "Proportional H"] {
        assert!(disabled(&fixture, field), "Unconstrained: {field} is off");
    }
    let fixed = fixture.by_label("Fixed Size").expect("the Fixed Size radio");
    fixture.click(&fixed).settle();
    assert_eq!(host.borrow().value.constraint, BoxConstraint::FixedSize, "the radio picks Fixed Size");
    assert!(!disabled(&fixture, "Fixed Size W") && !disabled(&fixture, "Fixed Size H"), "Fixed Size: its fields are on");
    assert!(disabled(&fixture, "Proportional W") && disabled(&fixture, "Proportional H"), "Fixed Size: Proportional's stay off");
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert!(!host.borrow().open, "Escape closes the gear");
    assert_eq!(host.borrow().value.constraint, BoxConstraint::FixedSize, "Escape keeps the pick");
}

#[trial]
fn a_fixed_size_typed_past_its_range_keeps_the_end_and_reports_it_once_and_an_emptied_field_is_empty() {
    let start = BoxConstraints { constraint: BoxConstraint::FixedSize, fixed: (Some(100.0), Some(50.0)), ..Default::default() };
    let (mut fixture, host) = gear(start);
    type_into(&mut fixture, "Fixed Size W", "400000");
    assert_eq!(host.borrow().value.fixed, (Some(300_000.0), Some(50.0)), "a width past 300,000 keeps 300,000");
    assert_eq!(host.borrow().alerts.len(), 1, "the clamp is reported once");
    type_into(&mut fixture, "Fixed Size W", "");
    assert_eq!(host.borrow().value.fixed, (None, Some(50.0)), "an emptied field is empty");
    assert_eq!(host.borrow().alerts.len(), 1, "an emptied field reports nothing");
}
