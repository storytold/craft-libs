//! [`ProjectMask`]: a layer's mask (`maskActions` plus `maskEnabled` and `maskInitial`). The mask is a
//! grey effect stack of its own: painted strokes (in a group), fills with generators, filters, levels,
//! colour selection, anchors and references.

use super::effect::ProjectEffect;

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectMask {
    pub enabled: bool,
    /// The value the mask starts from before its effects.
    pub initial: MaskInitial,
    /// Bottom → top.
    pub effects: Vec<ProjectEffect>,
}

/// `maskInitial`. 0 = black and 1 = white are Inferred: masks painted with white strokes mostly start
/// from 0 (256 of 290), and layers without a mask almost all have 1 (1962 of 2001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskInitial {
    Black,
    White,
    Other(u64),
}

impl MaskInitial {
    pub fn from_raw(value: u64) -> MaskInitial {
        match value {
            0 => MaskInitial::Black,
            1 => MaskInitial::White,
            n => MaskInitial::Other(n),
        }
    }

    pub fn name(self) -> String {
        match self {
            MaskInitial::Black => "black".into(),
            MaskInitial::White => "white".into(),
            MaskInitial::Other(n) => format!("initial {n}"),
        }
    }
}
