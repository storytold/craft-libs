//! `StateLayer`: the colour the resolver chose, over a shape.

use crate::style::StyleColor;
use egui::epaint::RectShape;
use egui::{CornerRadius, Rect, Shape, Stroke, StrokeKind};

/// How a state layer paints its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum StateLayerKind {
    /// A fill.
    #[default]
    Fill,
    /// An outline inside the rect (thumbnails, tiles).
    Outline,
}

/// The state layer painter.
pub struct StateLayer;

impl StateLayer {
    /// The shape of `color` over `rect`; an outline is `width` wide.
    pub fn shapes(kind: StateLayerKind, color: StyleColor, rect: Rect, radius: CornerRadius, width: f32) -> Vec<Shape> {
        let color = color.egui();
        match kind {
            StateLayerKind::Fill => vec![Shape::Rect(RectShape::filled(rect, radius, color))],
            StateLayerKind::Outline => vec![Shape::Rect(RectShape::stroke(rect, radius, Stroke::new(width, color), StrokeKind::Inside))],
        }
    }
}
