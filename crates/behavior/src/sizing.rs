//! `Proposal`, `Metrics` and `ControlMetrics`: what a measure call is asked and what a style gives it to add.

use crate::flow::Insets;
use craft_theme::{BySize, ControlSize};
use emath::Vec2;

/// The question one measure call answers.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Proposal {
    /// The size the content wants, unwrapped.
    Ideal,
    /// The smallest size the content can take.
    Min,
    /// The height at this width.
    Width(f32),
}

/// What the pipeline adds around a component's measured content.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Metrics {
    /// Space between the component's edge and its content.
    pub padding: Insets,
    /// The least size of the component.
    pub min_size: Vec2,
    /// The wrap width of text.
    pub max_width: Option<f32>,
    /// Space between content parts.
    pub gap: f32,
    /// The least text width before eliding.
    pub min_text: f32,
    /// The width the component takes where its room allows, though its content needs less: its ideal measure is at
    /// least this wide, up to the room it is offered, and its least measure is its content's.
    pub ideal_width: f32,
}

impl Metrics {
    /// No padding, no minimum, no wrap width.
    pub const fn new() -> Self {
        Metrics { padding: Insets::new(0.0, 0.0, 0.0, 0.0), min_size: Vec2::ZERO, max_width: None, gap: 0.0, min_text: 0.0, ideal_width: 0.0 }
    }

    /// Sets the padding.
    pub const fn padding(mut self, padding: Insets) -> Self {
        self.padding = padding;
        self
    }

    /// Sets the least size.
    pub const fn min_size(mut self, min_size: Vec2) -> Self {
        self.min_size = min_size;
        self
    }

    /// Sets the wrap width.
    pub const fn max_width(mut self, width: f32) -> Self {
        self.max_width = Some(width);
        self
    }

    /// Sets the gap between content parts.
    pub const fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// Sets the least text width.
    pub const fn min_text(mut self, width: f32) -> Self {
        self.min_text = width;
        self
    }

    /// Sets the width the component takes where its room allows.
    pub const fn ideal_width(mut self, width: f32) -> Self {
        self.ideal_width = width;
        self
    }
}

/// The sizes a control takes from its `ControlSize`: height, icon side, horizontal padding and gap.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ControlMetrics {
    /// The control's height.
    pub height: f32,
    /// The side of an icon inside the control.
    pub icon: f32,
    /// The padding at the start and the end.
    pub pad_x: f32,
    /// The gap between content parts.
    pub gap: f32,
}

impl ControlMetrics {
    /// The entries of the four tables at `size`.
    pub fn at(size: ControlSize, height: &BySize<f32>, icon: &BySize<f32>, pad_x: &BySize<f32>, gap: &BySize<f32>) -> Self {
        ControlMetrics { height: height.get(size), icon: icon.get(size), pad_x: pad_x.get(size), gap: gap.get(size) }
    }

    /// The `Metrics` of a control of this height: horizontal padding, the height as the least height, the gap.
    pub const fn metrics(&self) -> Metrics {
        Metrics::new().padding(Insets::horizontal(self.pad_x)).min_size(Vec2::new(0.0, self.height)).gap(self.gap)
    }
}
