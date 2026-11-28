//! The curve editor's opt-in point keys: Ctrl+Tab and Ctrl+Shift+Tab step through the points with `tab_steps`, and
//! Delete on an end point deselects it with `end_delete_deselects`.

use super::curve::{Rig, rig_with};
use craft_ui::testing::input::key_press;
use craft_ui::testing::trial;
use egui::{Key, Modifiers};

const FOUR: [[f32; 2]; 4] = [[0.0, 0.0], [64.0, 64.0], [192.0, 192.0], [255.0, 255.0]];

/// Selects the point at `q` with a click on it, which also focuses the graph.
fn click_point(rig: &mut Rig, q: [f32; 2]) {
    rig.press_at(q);
    rig.release_at(q);
}

/// Ctrl+Tab steps right through the points and wraps; Ctrl+Shift+Tab steps left. The curve stays as it is.
#[trial]
fn tab_steps_selects_the_next_and_previous_point_round_the_curve() {
    let mut rig = rig_with(&FOUR, true);
    click_point(&mut rig, [64.0, 64.0]);
    assert_eq!(rig.selected.get(), Some(1));
    let mut seen = Vec::new();
    for _ in 0..3 {
        rig.fixture.key(egui::Key::Tab, egui::Modifiers::CTRL).idle(1);
        seen.push(rig.selected.get());
    }
    assert_eq!(seen, [Some(2), Some(3), Some(0)], "right, then round to the first");
    seen.clear();
    for _ in 0..3 {
        rig.fixture.key(egui::Key::Tab, egui::Modifiers::CTRL | egui::Modifiers::SHIFT).idle(1);
        seen.push(rig.selected.get());
    }
    assert_eq!(seen, [Some(3), Some(2), Some(1)], "round to the last, then left");
    assert_eq!(rig.points(), FOUR.to_vec(), "no point moved");
}

/// Without the option Ctrl+Tab leaves the selection.
#[trial]
fn without_tab_steps_ctrl_tab_leaves_the_selection() {
    let mut rig = rig_with(&FOUR, false);
    click_point(&mut rig, [64.0, 64.0]);
    rig.fixture.key(egui::Key::Tab, egui::Modifiers::CTRL).idle(1);
    assert_eq!(rig.selected.get(), Some(1));
}

/// Delete on an end point keeps the point; with the option it deselects it, without it the point stays selected.
/// Delete on an inner point removes it either way.
#[trial]
fn end_delete_deselects_an_end_point_and_inner_points_still_go() {
    for (point_keys, after) in [(false, Some(0)), (true, None)] {
        let mut rig = rig_with(&FOUR, point_keys);
        click_point(&mut rig, [0.0, 0.0]);
        assert_eq!(rig.selected.get(), Some(0));
        rig.fixture.key(egui::Key::Delete, egui::Modifiers::NONE).idle(1);
        assert_eq!(rig.points(), FOUR.to_vec(), "an end point stays");
        assert_eq!(rig.selected.get(), after, "end_delete_deselects: {point_keys}");
        click_point(&mut rig, [192.0, 192.0]);
        rig.fixture.key(egui::Key::Delete, egui::Modifiers::NONE).idle(1);
        assert_eq!(rig.points(), vec![[0.0, 0.0], [64.0, 64.0], [255.0, 255.0]], "an inner point goes");
    }
}

/// Delete and then Ctrl+Tab in one frame act in the order they were pressed: on an inner point the selected point goes,
/// not the next one, and on an end point Delete only deselects, so the Ctrl+Tab after it has no point to step from. A
/// shortcut the graph doesn't claim, pressed in the same frame, still reaches the app.
#[trial]
fn delete_then_ctrl_tab_in_one_frame_act_in_order_and_other_shortcuts_reach_the_app() {
    let inner_gone = vec![[0.0, 0.0], [192.0, 192.0], [255.0, 255.0]];
    for (at, after) in [([64.0, 64.0], inner_gone), ([0.0, 0.0], FOUR.to_vec())] {
        let mut rig = rig_with(&FOUR, true);
        click_point(&mut rig, at);
        rig.app_keys.borrow_mut().clear();
        let keys = [(Key::Delete, Modifiers::NONE), (Key::Tab, Modifiers::CTRL), (Key::S, Modifiers::CTRL)];
        rig.fixture.frame_through(keys.iter().map(|(key, mods)| key_press(*key, *mods)).collect()).idle(1);
        assert_eq!(rig.points(), after, "Delete acts on the point selected at {at:?}");
        assert_eq!(rig.selected.get(), None, "nothing is selected after the Delete");
        let app = rig.app_keys.borrow();
        assert!(app.contains(&Key::S) && !app.contains(&Key::Delete) && !app.contains(&Key::Tab), "the app gets only Ctrl+S: {app:?}");
    }
}
