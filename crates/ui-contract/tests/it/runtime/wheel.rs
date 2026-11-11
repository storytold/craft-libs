//! The wheel service: the begin hook splits the pass's wheel input among last pass's takers.

use craft_behavior::{Sense, WheelTaker};
use craft_ui::testing::pointer;
use craft_ui::testing::sensing::{ProbeRoot, show};
use craft_ui::testing::wheel::{register, take};
use craft_ui::testing::{Frames, trial};
use egui::{Pos2, Rect, Vec2};

/// Skeleton: `frames_state::slider_takes_the_wheel_after_a_press_until_the_pointer_leaves`, second half: a press
/// arms a value control for the wheel until the pointer leaves it.
#[trial]
fn a_press_arms_a_gated_taker_until_the_pointer_leaves() {
    let panel = ProbeRoot::id("armed_panel");
    let slider = ProbeRoot::id("armed_slider");
    let panel_rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0));
    let slider_rect = Rect::from_min_size(Pos2::new(50.0, 50.0), Vec2::new(100.0, 20.0));
    let probe = ProbeRoot::new("armed_slider", slider_rect, Sense::CLICK | Sense::DRAG);
    let read = std::cell::Cell::new((None, None));
    let mut show_frame = |ui: &mut egui::Ui| {
        show(ui, std::slice::from_ref(&probe));
        register(ui.ctx(), WheelTaker::new(panel, panel_rect).vertical(true));
        register(ui.ctx(), WheelTaker::new(slider, slider_rect).vertical(true).gated(true));
        read.set((take(ui.ctx(), panel), take(ui.ctx(), slider)));
    };
    let scroll = Some(Vec2::new(0.0, -30.0));
    let on_slider = Pos2::new(70.0, 60.0);
    let mut frames = Frames::new();
    frames.idle(2, &mut show_frame);
    frames.frame(vec![pointer::moved(on_slider)], &mut show_frame);
    frames.frame(vec![pointer::wheel(-30.0)], &mut show_frame);
    assert_eq!(read.get(), (scroll, None), "before a press the panel takes the wheel");
    frames.frame(vec![pointer::press(on_slider)], &mut show_frame);
    frames.frame(vec![pointer::release(on_slider)], &mut show_frame);
    frames.frame(vec![pointer::wheel(-30.0)], &mut show_frame);
    assert_eq!(read.get(), (None, scroll), "after a press the slider takes it");
    frames.frame(vec![pointer::moved(Pos2::new(300.0, 300.0))], &mut show_frame);
    frames.frame(vec![pointer::wheel(-30.0)], &mut show_frame);
    assert_eq!(read.get(), (scroll, None), "once the pointer leaves, the panel takes it again");
}
