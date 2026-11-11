//! `ResetMark`: the dot the label step shows beside a side label while the value differs from its reset value.

use crate::style::ResetMarkLook;
use egui::{Pos2, Shape};

/// The reset mark painter.
pub struct ResetMark;

impl ResetMark {
    /// The dot centred on `center`.
    pub fn shapes(look: &ResetMarkLook, center: Pos2) -> Vec<Shape> {
        vec![Shape::circle_filled(center, look.diameter / 2.0, look.color.egui())]
    }
}
