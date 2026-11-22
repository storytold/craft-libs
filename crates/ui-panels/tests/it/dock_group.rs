//! A docked panel group.

use craft_ui::buttons::Button;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::docking::{DockGroup, PanelKey, PanelTitle, Titles};
use craft_ui::testing::{Fixture, trial};
use egui::{Pos2, Rect, Vec2};
use std::cell::Cell;
use std::rc::Rc;

// A minimized group shows its header alone: its body does not run, so it leaves no node and nothing a press could
// reach under the groups below it. Expanded, the same body shows.
#[trial]
fn a_minimized_group_runs_no_body() {
    let body = |minimized: bool| {
        let runs = Rc::new(Cell::new(0));
        let counted = Rc::clone(&runs);
        let panels = vec![PanelTitle::new(PanelKey::new(1), "Color")];
        let mut fixture = Fixture::new(move |ui| {
            let group = DockGroup::new(1usize, Titles::from_content(panels.clone()), panels[0].key);
            let group = if minimized { group.minimized() } else { group };
            let counted = Rc::clone(&counted);
            group.show_at(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(240.0, 200.0)), move |ui, _| {
                counted.set(counted.get() + 1);
                Button::new("Body button").show(ui);
            });
        });
        fixture.idle(4);
        let found = fixture.by_label("Body button").is_some();
        (runs.get(), found)
    };
    let (runs, found) = body(false);
    assert!(runs > 0 && found, "an expanded group shows its body: {runs} runs, node {found}");
    assert_eq!(body(true), (0, false), "a minimized group runs no body and leaves no node");
}
