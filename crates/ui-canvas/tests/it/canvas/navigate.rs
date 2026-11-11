//! Navigation rules (wheel, page keys, flick, autoscroll), the zoom motion and the momentary tool order.

use craft_behavior::{Wheel, WheelPhase, WheelUnit};
use craft_keymap::Mods;
use craft_ui::canvas::{Momentary, MomentaryCommands, MomentaryTool, NavigationOptions, TouchpadGlide};
use craft_ui::runtime::commands::FirePhase;
use craft_ui::testing::trial;
use egui::Vec2;

#[trial]
fn a_touchpad_glide_stops_in_the_pass_the_edge_clamp_leaves_the_view_where_it_was() {
    const STEP: f64 = 0.016;
    let mut glide = TouchpadGlide::new(true);
    let mut now = 0.0;
    glide.read(&Wheel::new(Vec2::new(30.0, 0.0), Mods::NONE, WheelUnit::Point, WheelPhase::Start), now, true);
    for _ in 0..4 {
        now += STEP;
        glide.read(&Wheel::new(Vec2::new(30.0, 0.0), Mods::NONE, WheelUnit::Point, WheelPhase::Move), now, true);
    }
    glide.read(&Wheel::new(Vec2::ZERO, Mods::NONE, WheelUnit::Point, WheelPhase::End), now, true);
    now += STEP;
    assert!(glide.step(now).x > 0.0, "a fast lift glides");
    glide.moved(true);
    now += STEP;
    assert!(glide.step(now).x > 0.0, "a glide whose view moved goes on");
    glide.moved(false);
    now += STEP;
    assert_eq!(glide.step(now), Vec2::ZERO, "a pass that left the view where it was ends the glide");
}

#[trial]
fn space_is_the_hand_and_cmd_or_alt_added_in_any_order_switch_to_zoom() {
    let mut momentary = Momentary::new(NavigationOptions::default());
    assert_eq!(momentary.tool(Mods::NONE, false), None);
    momentary.fired(MomentaryCommands::Hand, FirePhase::Press);
    assert_eq!(momentary.tool(Mods::NONE, false), Some(MomentaryTool::Hand));
    assert_eq!(momentary.tool(Mods::CMD, false), Some(MomentaryTool::ZoomIn));
    assert_eq!(momentary.tool(Mods::ALT, false), Some(MomentaryTool::ZoomOut));
    momentary.fired(MomentaryCommands::ZoomIn, FirePhase::Press);
    assert_eq!(momentary.tool(Mods::CMD, false), Some(MomentaryTool::ZoomIn));
    let both = Mods { cmd: true, alt: true, ..Mods::NONE };
    assert_eq!(momentary.tool(both, false), Some(MomentaryTool::ZoomOut), "Alt added to a held Zoom In zooms out");
    assert_eq!(momentary.tool(Mods::NONE, true), Some(MomentaryTool::ZoomOut), "a drag keeps the tool it began with");
    momentary.fired(MomentaryCommands::ZoomIn, FirePhase::Release);
    assert_eq!(momentary.tool(Mods::NONE, false), None);
}
