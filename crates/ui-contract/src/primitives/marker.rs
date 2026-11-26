//! `Marker`: the mark that shows where a colour sits on a colour field, wheel, disc or gradient line.

use crate::style::MarkerLook;
use egui::{Color32, Pos2, Shape, Stroke};

/// How large a marker is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum MarkerSize {
    /// A marker of the look's radius.
    #[default]
    Regular,
    /// The look's large radius: the base colour of a set, or a line's end.
    Large,
}

/// A colour marker centred on `at`: the colour it marks (when filled), a ring and a halo ring just outside it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[must_use]
pub struct Marker {
    at: Pos2,
    fill: Option<Color32>,
    size: MarkerSize,
    /// A radius in place of the size's.
    radius: Option<f32>,
    selected: bool,
}

impl Marker {
    /// A ring that shows the field under it.
    pub const fn hollow(at: Pos2) -> Self {
        Marker { at, fill: None, size: MarkerSize::Regular, radius: None, selected: false }
    }

    /// A disc of `color`; a translucent colour sits on the look's backing.
    pub const fn filled(at: Pos2, color: Color32) -> Self {
        Marker { at, fill: Some(color), size: MarkerSize::Regular, radius: None, selected: false }
    }

    /// Drawn at `size`.
    pub const fn size(mut self, size: MarkerSize) -> Self {
        self.size = size;
        self
    }

    /// Drawn at `radius` in place of its size's: a ring round a picked mark, such as a selected blur pin's ring.
    pub const fn radius(mut self, radius: f32) -> Self {
        self.radius = Some(radius);
        self
    }

    /// The ring in the look's selected colour while `selected`.
    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// The colour it marks, if filled.
    pub(crate) const fn fill(&self) -> Option<Color32> {
        self.fill
    }

    /// The same marker filled with `color`.
    pub(crate) const fn with_fill(mut self, color: Color32) -> Self {
        self.fill = Some(color);
        self
    }

    /// The radius of a marker of `size`, out to the inner edge of its ring.
    pub fn size_radius(look: &MarkerLook, size: MarkerSize) -> f32 {
        match size {
            MarkerSize::Regular => look.radius,
            MarkerSize::Large => look.large,
        }
    }

    /// How far a marker of `size` reaches from its centre, halo included.
    pub fn reach(look: &MarkerLook, size: MarkerSize) -> f32 {
        Self::size_radius(look, size) + look.ring_width + look.halo_width
    }

    /// The shapes, bottom first.
    pub fn shapes(&self, look: &MarkerLook) -> Vec<Shape> {
        let radius = self.radius.unwrap_or_else(|| Self::size_radius(look, self.size));
        let mut shapes = Vec::with_capacity(4);
        if let Some(color) = self.fill {
            if !color.is_opaque() {
                shapes.push(Shape::circle_filled(self.at, radius, look.backing.egui()));
            }
            shapes.push(Shape::circle_filled(self.at, radius, color));
        }
        let ring = if self.selected { look.selected } else { look.ring };
        shapes.push(Shape::circle_stroke(self.at, radius, Stroke::new(look.ring_width, ring.egui())));
        // A circle strokes outside its radius: the halo starts where the ring ends.
        let halo = radius + look.ring_width;
        shapes.push(Shape::circle_stroke(self.at, halo, Stroke::new(look.halo_width, look.halo.egui())));
        shapes
    }
}
