//! `AutoScrollMark`: the mark a latched middle-button auto-scroll shows at its anchor: a ring on the panel fill, a dot
//! and two arrowheads in text ink.

use crate::style::{StyleColor, UiTokens};
use egui::{Pos2, Shape};

/// The auto-scroll mark's colours and sizes.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct AutoScrollMarkLook {
    /// The ring colour.
    pub ring: StyleColor,
    /// The fill inside the ring.
    pub fill: StyleColor,
    /// The ink of the dot and the arrowheads.
    pub ink: StyleColor,
    /// The ring's line width.
    pub ring_width: f32,
    /// The mark's diameter.
    pub diameter: f32,
}

pub(crate) fn auto_scroll_mark_look(tokens: &UiTokens) -> AutoScrollMarkLook {
    let _ = tokens;
    todo!("CV-C9")
}

/// The auto-scroll mark painter.
pub struct AutoScrollMark;

impl AutoScrollMark {
    /// The mark centred on `anchor`, from the look's tokens.
    pub fn shapes(look: &AutoScrollMarkLook, anchor: Pos2) -> Vec<Shape> {
        let _ = (look, anchor);
        todo!("CV-C9")
    }
}
