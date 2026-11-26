//! The kit's window driver gives focus and close requests to one child of several.

use craft_ui_test_kit::windows::Windows;
use egui::{ViewportBuilder, ViewportId, ViewportInfo};
use std::cell::RefCell;
use std::collections::BTreeMap;

/// What each of three children saw in a frame: (close requested, focused).
fn run(kit: &Windows) -> BTreeMap<&'static str, (bool, bool)> {
    let seen = RefCell::new(BTreeMap::new());
    kit.frame(Vec::new(), Vec::new(), |ui| {
        for name in ["a", "b", "c"] {
            ui.ctx().show_viewport_immediate(ViewportId::from_hash_of(name), ViewportBuilder::default(), |ui, _| {
                let (close, focused) = ui.ctx().input(|i| (i.viewport().close_requested(), i.viewport().focused == Some(true)));
                seen.borrow_mut().insert(name, (close, focused));
            });
        }
    });
    seen.into_inner()
}

#[test]
fn one_of_three_children_gets_the_close_and_the_focus() {
    let kit = Windows::new();
    let (a, b, c) = (ViewportId::from_hash_of("a"), ViewportId::from_hash_of("b"), ViewportId::from_hash_of("c"));
    run(&kit);
    kit.focus(b);
    kit.close(b);
    let seen = run(&kit);
    assert!(seen["b"].0 && !seen["a"].0 && !seen["c"].0, "only b is asked to close");
    assert!(seen["b"].1 && !seen["a"].1 && !seen["c"].1, "only b is focused");
    assert!(!run(&kit)["b"].0, "a close request comes once");
    kit.focus(c);
    let seen = run(&kit);
    assert!(seen["c"].1 && !seen["b"].1 && !seen["a"].1);
    assert_eq!(kit.facts(a).focused, Some(false));
    let scale = kit.facts(a).native_pixels_per_point;
    kit.child_info(ViewportInfo { native_pixels_per_point: Some(3.0), ..ViewportInfo::default() });
    assert_ne!(kit.facts(a).native_pixels_per_point, scale, "the shared facts still reach a child after a focus");
    assert_eq!(kit.facts(c).focused, Some(true), "and the focus outlasts them");
}
