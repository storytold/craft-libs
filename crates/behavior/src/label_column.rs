//! `LabelColumn`: label-then-control geometry for one row.

use crate::flow::Flow;
use emath::{Align, Pos2, Rect, Vec2};

/// The label column of a row: a width token, and the row width under which the label moves above the control.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct LabelColumn {
    /// The width of the label column.
    pub width: f32,
    /// Rows narrower than this stack the label above the control.
    pub stack_below: f32,
    /// Space between the label and the control.
    pub gap: f32,
}

/// What `LabelColumn::place` decided for one row.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct LabelPlacement {
    /// Where the label is drawn.
    pub label: Rect,
    /// The rect left for the control.
    pub control: Rect,
    /// Whether the label sits above the control.
    pub stacked: bool,
}

impl LabelColumn {
    /// A column `width` wide that stacks below `stack_below`, with `gap` between label and control.
    pub const fn new(width: f32, stack_below: f32, gap: f32) -> Self {
        LabelColumn { width, stack_below, gap }
    }

    /// Places a label of `label` size whose baseline is `label_baseline` below its top, and the control in the
    /// rest of `flow`'s rect. `control_baseline` is the distance from the control's top to its first baseline
    /// as measured last pass; with it the label's baseline lines up with the control's. `None` (the first
    /// appearance) aligns the tops.
    pub fn place(&self, flow: Flow, label: Vec2, label_baseline: f32, control_baseline: Option<f32>) -> LabelPlacement {
        let row = flow.rect();
        if row.width() < self.stack_below {
            let label_rect = flow.align(Vec2::new(label.x.min(row.width()), label.y), Align::Min, Align::Min);
            let control = Rect::from_min_max(Pos2::new(row.left(), row.top() + label.y + self.gap), row.max);
            return LabelPlacement { label: label_rect, control, stacked: true };
        }
        let (column, control_flow) = flow.take_start(self.width);
        let (_, control_flow) = control_flow.take_start(self.gap);
        let drop = control_baseline.map_or(0.0, |control| control - label_baseline);
        let (label_drop, control_drop) = (drop.max(0.0), (-drop).max(0.0));
        let label_size = Vec2::new(label.x.min(column.width()), label.y);
        let label_rect = Flow::new(column, flow.is_rtl()).align(label_size, Align::Min, Align::Min).translate(Vec2::new(0.0, label_drop));
        let control_rect = control_flow.rect();
        let control = Rect::from_min_max(Pos2::new(control_rect.left(), control_rect.top() + control_drop), control_rect.max);
        LabelPlacement { label: label_rect, control, stacked: false }
    }
}
