//! `ContourField`: the thumbnail asks for the Contour Editor, the rest of the field opens the presets.

use std::cell::RefCell;
use std::rc::Rc;

use craft_imaging::layer_style::Contour;
use craft_imaging::ui::layer_style::{ContourField, ContourKey, ContourRequest, ContourWords, contour_rows};
use craft_ui::contract::{Bind, Role};
use craft_ui::patterns::presets::PresetState;
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, pos2};

/// A field over Linear, recording the editor requests it reports.
fn rig() -> (Fixture, Rc<RefCell<Vec<ContourRequest>>>) {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let seen = requests.clone();
    let mut contour = Contour::linear();
    let mut state = PresetState::<ContourKey>::new();
    let mut fixture = Fixture::new(move |ui| {
        let words = Env::of(ui).words::<ContourWords>();
        let rows = contour_rows(&Contour::presets(), &[], &words, None);
        let out = ContourField::new("Contour", Bind::bound(&mut contour), &rows, Bind::bound(&mut state)).show(ui);
        seen.borrow_mut().extend(out.edit_contour);
    });
    fixture.settle();
    (fixture, requests)
}

fn field(fixture: &Fixture) -> Rect {
    fixture.by_role(Role::COMBO_BOX).first().expect("the contour field").rect
}

/// The browser is open while its presets are in the tree; the closed field names only Linear.
fn browser_open(fixture: &Fixture) -> bool {
    fixture.by_label("Cone").is_some()
}

#[trial]
fn the_thumbnail_asks_for_the_editor_and_the_arrow_opens_the_presets() {
    let (mut fixture, requests) = rig();
    let rect = field(&fixture);
    fixture.click_at(pos2(rect.left() + rect.height() / 2.0, rect.center().y));
    fixture.settle();
    assert_eq!(requests.borrow().len(), 1, "a click on the thumbnail asks for the Contour Editor");
    assert_eq!(requests.borrow()[0].contour, Contour::linear());
    assert!(!browser_open(&fixture), "a click on the thumbnail opens no browser");

    fixture.click_at(pos2(rect.right() - rect.height() / 2.0, rect.center().y));
    fixture.settle();
    assert!(browser_open(&fixture), "a click on the arrow opens the presets");
    assert_eq!(requests.borrow().len(), 1, "the arrow asks for no editor");
}
