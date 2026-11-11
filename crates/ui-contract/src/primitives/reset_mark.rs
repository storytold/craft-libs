//! `ResetMark`: the mark the label step shows beside a side label: a dot while the value differs from its reset value,
//! a ring while it is at it.

use crate::style::ResetMarkLook;
use egui::{Pos2, Shape};

/// The reset mark painter.
pub struct ResetMark;

impl ResetMark {
    /// The mark centred on `center`: the dot while the value `differs`, else the ring.
    pub fn shapes(look: &ResetMarkLook, center: Pos2, differs: bool) -> Vec<Shape> {
        let radius = look.diameter / 2.0;
        if differs {
            vec![Shape::circle_filled(center, radius, look.color.egui())]
        } else {
            vec![Shape::circle_stroke(center, radius - look.ring_width / 2.0, egui::Stroke::new(look.ring_width, look.ring.egui()))]
        }
    }
}
