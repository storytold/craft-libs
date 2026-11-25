//! `LabelColumn` rows: the label aligned to the control's first baseline from last pass.

use crate::runtime::layout_memory;
use crate::runtime::memory::sub;
use craft_behavior::{Flow, LabelColumn, LabelPlacement};
use egui::{Id, Ui, Vec2};

/// Places a label of `label` size with baseline `label_baseline` and the control of the row `id` in `flow`'s
/// rect. The label's baseline follows the control baseline `record_control_baseline` stored last pass; on the
/// first appearance the tops align and the missing entry asks for one discard pass.
pub fn place_label(ui: &Ui, id: Id, column: &LabelColumn, flow: Flow, label: Vec2, label_baseline: f32) -> LabelPlacement {
    let control_baseline = control_baseline(ui.ctx(), id, label, label_baseline);
    column.place(flow, label, label_baseline, control_baseline)
}

/// Stores the distance from the top of the control of the row `id` to its first baseline, for the next
/// pass's `place_label`.
pub fn record_control_baseline(ui: &Ui, id: Id, baseline: f32) {
    layout_memory::record(ui.ctx(), sub(id, "baseline"), Vec2::new(0.0, baseline));
}

/// The control's first baseline from its top, as `record_control_baseline` stored it last pass. A control that paints
/// no text stored its centre line instead; the label's baseline then sits where its text is centred on that line.
pub fn control_baseline(ctx: &egui::Context, id: Id, label: Vec2, label_baseline: f32) -> Option<f32> {
    layout_memory::read(ctx, sub(id, "baseline"))
        .map(|size| size.y)
        .or_else(|| layout_memory::read(ctx, sub(id, "centre")).map(|size| size.y + label_baseline - label.y / 2.0))
}
