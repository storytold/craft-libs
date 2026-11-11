//! Where the label and its control sit: the beside, above and after placements and the gap.

use super::leaf::{LabelLeaf, colon};
use super::props::{FieldLayout, LabelAlign, LabelKind};
use super::step::LabelIn;
use crate::contract::Show;
use crate::runtime::access::scope::{self};
use crate::runtime::egui::{alloc, child};
use crate::runtime::env::Env;
use crate::runtime::memory::sub;
use craft_behavior::{ColumnWidth, Flow, LabelColumn};
use egui::{Align, Rect, Ui, UiBuilder, Vec2};

/// Where the label step put the label and the control.
pub(crate) struct Placed {
    /// The label's rect; `Rect::NOTHING` for a hidden label.
    pub label: Rect,
    /// The rect left for the control.
    pub control: Rect,
}

/// The `After` placement: the control's cell first, then the gap, then the label. A leaf control (a box, circle or
/// track has no text baseline) and its label are centred on each other across the row. For a field's control, as in
/// `LabelColumn::place`, the one with the lower baseline moves down so the two baselines line up, using the baseline
/// `record_control_baseline` stored last pass.
fn place_after(ctx: &egui::Context, input: &LabelIn<'_>, flow: Flow, gap: f32, label: Vec2, label_baseline: f32) -> Placed {
    let rtl = flow.is_rtl();
    let (control_cell, rest) = flow.take_start(input.control_min.min(flow.rect().width()));
    let (_, rest) = rest.take_start(gap);
    let label_width = label.x.min(rest.rect().width());
    if input.leaf {
        let height = control_cell.height().max(label.y);
        let label = Flow::new(rest.rect(), rtl).align(Vec2::new(label_width, height), Align::Min, Align::Min);
        let control = control_cell.translate(Vec2::new(0.0, (height - control_cell.height()) / 2.0));
        return Placed { label, control };
    }
    let control_baseline = crate::layout::control_baseline(ctx, input.scope, label, label_baseline);
    let drop = control_baseline.map_or(0.0, |control| control - label_baseline);
    let (label_drop, control_drop) = (drop.max(0.0), (-drop).max(0.0));
    let label_size = Vec2::new(label_width, label.y);
    let label = Flow::new(rest.rect(), rtl).align(label_size, Align::Min, Align::Min).translate(Vec2::new(0.0, label_drop));
    let control = Rect::from_min_max(egui::pos2(control_cell.left(), control_cell.top() + control_drop), control_cell.max);
    Placed { label, control }
}

/// The rect a hidden label leaves for its control. In a body whose labelled rows share a label column the label
/// still takes the column, so the control starts in the control column; elsewhere, or where the row is too narrow
/// for the column, the control takes the row.
pub(super) fn hidden_control(ctx: &egui::Context, input: &LabelIn<'_>, flow: Flow, gap: f32) -> Rect {
    if input.side.layout != FieldLayout::Beside {
        return input.row;
    }
    match crate::layout::ambient_width(ctx, &crate::layout::LABELS, 0.0).filter(|column| *column > 0.0) {
        Some(column) if input.room >= column + gap + input.control_min => {
            let (_, rest) = flow.take_start(column);
            rest.take_start(gap).1.rect()
        }
        _ => input.row,
    }
}

/// Decides where the label and the control of `input` sit, from `input.control_min`, `input.room` and the group's
/// label column. Shows and records nothing.
pub(super) fn decide(ctx: &egui::Context, env: &Env, input: &LabelIn<'_>, kids: &Ui, leaf: &mut LabelLeaf, glyph: bool, flow: Flow) -> Placed {
    let rtl = env.direction.is_rtl();
    let gap = label_gap(env);
    let mut natural = leaf.measure_in(kids, craft_behavior::Proposal::Ideal);
    if input.side.layout == FieldLayout::After {
        // The label takes the room the control and the gap leave, and its detail wraps there.
        let room = (input.row.width() - input.control_min - gap).max(0.0);
        if natural.x > room {
            natural = leaf.measure_in(kids, craft_behavior::Proposal::Width(room));
        }
    }
    let baseline = leaf.baseline.get();
    if input.side.layout == FieldLayout::Line {
        let (cell, rest) = flow.take_end(env.tokens.size.length.number);
        let width = natural.x.min((rest.rect().width() - gap).max(0.0));
        let drop = (cell.height() - natural.y).max(0.0) / 2.0;
        let label = Flow::new(rest.rect(), rtl).align(Vec2::new(width, natural.y), Align::Min, Align::Min).translate(Vec2::new(0.0, drop));
        return Placed { label, control: cell };
    }
    let (column, stack_below) = match input.side.layout {
        FieldLayout::Beside => {
            let own = natural.x;
            let width = crate::layout::ambient_width(ctx, &if glyph { crate::layout::GLYPHS } else { crate::layout::LABELS }, own)
                .filter(|width| *width > 0.0)
                .unwrap_or(own);
            (width, width + gap + input.control_min)
        }
        FieldLayout::Above => (natural.x, f32::INFINITY),
        // `Line` returned above.
        FieldLayout::After | FieldLayout::Line => (natural.x, 0.0),
        FieldLayout::Caption(cell) => {
            let width = match cell {
                ColumnWidth::Fixed(width) => width.min(natural.x),
                _ => natural.x,
            };
            (width, f32::INFINITY)
        }
    };
    // The room decides; a beside label gets the width it needs even where last frame's content was narrower. A label
    // that never sits beside stacks in an unbounded room as well.
    let stacks = (input.room < stack_below || stack_below == f32::INFINITY) && !(input.content_row && input.side.layout == FieldLayout::Beside);
    // A stacked label has the row's width at most; its name elides there.
    let column = if stacks { column.min(input.row.width()) } else { column };
    let flow = if stacks || input.row.width() >= stack_below {
        flow
    } else {
        let mut wide = input.row;
        if rtl {
            wide.min.x = wide.max.x - stack_below;
        } else {
            wide.max.x = wide.min.x + stack_below;
        }
        Flow::new(wide, rtl)
    };

    if input.side.layout == FieldLayout::After {
        place_after(ctx, input, flow, gap, natural, baseline)
    } else {
        let placed = crate::layout::place_label(
            kids,
            input.scope,
            &LabelColumn::new(column, if stacks { f32::INFINITY } else { 0.0 }, gap),
            flow,
            natural.min(Vec2::new(column, natural.y)),
            baseline,
        );
        let mut label = placed.label;
        if !placed.stacked && input.side.align == LabelAlign::End {
            let spare = (column - label.width()).max(0.0);
            label = label.translate(Vec2::new(if rtl { -spare } else { spare }, 0.0));
        }
        Placed { label, control: placed.control }
    }
}

/// The gap between a label and its control: `space.gap.inline`, `space.gap.item` inside a bar.
pub(crate) fn label_gap(env: &Env) -> f32 {
    let gaps = &env.tokens.space.gap;
    if env.values.get::<crate::contract::container::InBar>().0 { gaps.item } else { gaps.inline }
}

/// Where the control sits once `input.control_min` is known: the decision `run` takes, without showing the label. A
/// measuring run calls it after the body ran, with the control minimum that run recorded.
pub(crate) fn control_after(ui: &mut Ui, env: &Env, input: &LabelIn<'_>) -> Rect {
    let ctx = ui.ctx().clone();
    let flow = Flow::new(input.row, env.direction.is_rtl());
    if input.side.kind == LabelKind::Hidden {
        return hidden_control(&ctx, input, flow, label_gap(env));
    }
    let reset = scope::last_reset(&ctx, input.scope);
    let slot = if reset.is_some() && input.side.reset_dot { env.tokens.size.handle.sm } else { 0.0 };
    let mut leaf = LabelLeaf::new(input.text.clone(), slot);
    leaf.resettable = reset.is_some();
    leaf.colon = colon(env, input);
    match &input.side.kind {
        LabelKind::Glyph(glyph) => leaf.glyph = Some(glyph.resolved(env)),
        _ => {
            leaf.icon = input.side.icon.clone();
            leaf.detail = input.side.detail.as_ref().map(|detail| env.view().resolve(detail));
        }
    }
    let glyph = leaf.glyph.is_some();
    let builder = UiBuilder::new().max_rect(input.row).layout(child::pinned_layout()).scope_id(sub(input.scope, "label_ui"));
    let kids = alloc::child(ui, builder);
    let installed = Env::install_for(&kids, crate::contract::cx::child_env(env, input.enabled, input.size));
    let placed = decide(&ctx, env, input, &kids, &mut leaf, glyph, flow);
    Env::restore(&ctx, installed);
    placed.control
}
