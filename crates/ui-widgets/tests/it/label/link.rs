//! A link side label is a link to assistive technology, reports its click to the control's host and leaves focus alone;
//! a drag on it still scrubs, and a plain label beside it still focuses its control.

use super::kit::{Dial, Field, Seen};
use super::{ROW_A, START};
use craft_ui::contract::{LinkLabelUnderline, Props as _, Show as _, ShowContainer as _, SideLabel};
use craft_ui::testing::{Fixture, trial};
use egui::{Pos2, Rect, Ui, Vec2, pos2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const ROW_B: Rect = Rect { min: pos2(10.0, 60.0), max: pos2(310.0, 90.0) };
const DRAG: f32 = 40.0;

/// One row of the trial: its value, what its dial saw and whether its label ever reported a link click.
#[derive(Clone)]
struct Row {
    value: Rc<Cell<f64>>,
    seen: Rc<RefCell<Seen>>,
    linked: Rc<Cell<bool>>,
}

impl Row {
    fn new() -> Self {
        Row { value: Rc::new(Cell::new(START)), seen: Rc::default(), linked: Rc::default() }
    }

    fn show(&self, ui: &mut Ui, salt: &'static str, side: SideLabel, at: Rect) {
        let mut value = self.value.get();
        Field::new("Opacity", side).id_salt(salt).show_at(ui, at, |ui, _| {
            Dial::new(&mut value, &self.seen).show(ui);
        });
        self.value.set(value);
        let link = self.seen.borrow().label.as_ref().is_some_and(|label| label.link);
        self.linked.set(self.linked.get() || link);
    }

    fn label_at(&self) -> Pos2 {
        let seen = self.seen.borrow();
        seen.label.as_ref().map(|label| label.rect.center()).unwrap_or_else(|| panic!("the dial ran inside a label scope"))
    }
}

#[trial]
fn a_link_label_reports_its_click_to_the_host_and_still_scrubs() {
    let (link, plain) = (Row::new(), Row::new());
    let shown = (link.clone(), plain.clone());
    let mut fixture = Fixture::new(move |ui| {
        shown.0.show(ui, "link", SideLabel::new().link().scrub(true), ROW_A);
        shown.1.show(ui, "plain", SideLabel::new().scrub(true), ROW_B);
    });
    fixture.settle();
    let links = fixture.by_role(craft_ui::runtime::access::Role::LINK);
    assert!(
        links.len() == 1 && links[0].rect.contains(link.label_at()),
        "the link label is a link to assistive technology, the plain one is not: {links:?}"
    );

    fixture.click_at(link.label_at());
    assert!(link.linked.get(), "a click on a link label reports to the host");
    assert_ne!(fixture.focused(), link.seen.borrow().id, "a link click leaves focus alone");

    fixture.click_at(plain.label_at());
    assert!(!plain.linked.get(), "a plain label reports no link");
    assert_eq!(fixture.focused(), plain.seen.borrow().id, "a click on a plain label focuses its control");

    fixture.drag_at(link.label_at(), Vec2::new(DRAG, 0.0));
    assert!(link.value.get() > START, "a drag on a link label scrubs: {}", link.value.get());
}

/// `Fixture::by_label` gives the control a link label names, not the link that carries the same name.
#[trial]
fn by_label_gives_the_control_a_link_label_names() {
    let row = Row::new();
    let shown = row.clone();
    let mut fixture = Fixture::new(move |ui| shown.show(ui, "link", SideLabel::new().link(), ROW_A));
    fixture.settle();
    let dial = row.seen.borrow().id.expect("the dial ran");
    let control = fixture.node_of(dial).expect("the dial is in the access tree").node;
    assert_eq!(fixture.by_label("Opacity").map(|found| found.node), Some(control), "the label's name finds its control");
}

/// Regression: egui strokes a link's underline on its text box's bottom edge, which the clip of a label rect no taller
/// than that box cut away, so the link showed no underline. Under `LinkLabelUnderline::Inside` the label draws the
/// underline under its name, inside its rect.
#[trial]
fn an_inside_underline_is_painted_under_the_name_inside_the_clip() {
    let row = Row::new();
    let shown = row.clone();
    let mut fixture = Fixture::new(move |ui| shown.show(ui, "link", SideLabel::new().link(), ROW_A));
    fixture.root_value(LinkLabelUnderline::Inside).settle();
    let shapes = fixture.frame_clipped_shapes();
    let (clip, name) = shapes
        .iter()
        .find_map(|(clip, shape)| match shape {
            egui::Shape::Text(text) if text.galley.text() == "Opacity" => Some((*clip, shape.visual_bounding_rect())),
            _ => None,
        })
        .expect("the link label paints its name");
    let line = shapes
        .iter()
        .filter(|(at, shape)| *at == clip && !matches!(shape, egui::Shape::Text(_)))
        .map(|(_, shape)| shape.visual_bounding_rect())
        .find(|rect| rect.is_finite() && rect.height() <= 2.0 && rect.width() >= name.width() / 2.0)
        .expect("the label draws its underline");
    assert!(line.center().y > name.center().y, "the underline {line:?} lies under the name {name:?}");
    assert!(clip.y_range().contains(line.top()) && clip.y_range().contains(line.bottom()), "the underline {line:?} lies inside the clip {clip:?}");
}
