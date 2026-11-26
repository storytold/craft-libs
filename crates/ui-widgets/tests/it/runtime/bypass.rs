//! Bypass detection: a raw egui widget fails the pass-end walk, a component's own widgets pass it.

use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::testing::{Fixture, bypass, trial};
use egui::{Rect, pos2};

#[trial]
fn a_raw_egui_widget_next_to_a_component_is_a_finding() {
    let mut checked = false;
    let mut fixture = Fixture::new(move |ui| {
        Checkbox::new(Bind::bound(&mut checked)).label("Show grid").show(ui);
        let raw = Rect::from_min_max(pos2(0.0, 100.0), pos2(40.0, 120.0));
        craft_ui::testing::contract::raw_widget(ui, raw);
    });
    fixture.settle();
    // The raw widget shows twice: as an egui widget no component registered, and as an AccessKit node that declares
    // a click the runtime did not write. The component beside it shows in neither.
    let found = bypass::findings(&fixture);
    let widgets: Vec<_> = found.iter().filter(|finding| finding.starts_with("egui widget")).collect();
    let nodes: Vec<_> = found.iter().filter(|finding| finding.starts_with("AccessKit node")).collect();
    assert_eq!((widgets.len(), nodes.len()), (1, 1), "{found:?}");
    assert!(widgets[0].contains("raw_widget"), "{found:?}");
}
