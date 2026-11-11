//! `measure_slot`: a closure measured in an invisible sizing pass.

use crate::runtime::egui::{child, sizing};
use crate::runtime::env::Env;
use egui::{Ui, Vec2};

/// Runs `body` in an invisible sizing pass `width` wide and returns the size it took. Nothing is allocated in
/// `ui` and nothing is painted; the closure sees the env of `ui`.
pub fn measure_slot(ui: &mut Ui, width: f32, body: impl FnOnce(&mut Ui)) -> Vec2 {
    let rtl = Env::of(ui).direction.is_rtl();
    sizing::run_sized(ui, width, child::column_layout(rtl), body).1
}
