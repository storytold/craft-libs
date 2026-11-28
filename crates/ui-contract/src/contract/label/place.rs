//! Where the label and its control sit: the beside, above and after placements and the gap.

use super::leaf::{LabelLeaf, colon};
use super::props::{CaptionColumn, FieldLayout, LabelKind, LabelScheme};
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
    /// The rect of the content shown inside the label after its text; `Rect::NOTHING` without one.
    pub trailing: Rect,
    /// The width a caption above its control takes whole (its own width, at most its cell's); the field is never
    /// narrower, so the caption shows whole. 0 for a label beside or after its control.
    pub caption: f32,
}

/// The `After` placement: the control's cell first, then the gap, then the label. A leaf control (a box, circle or
/// track has no text baseline) and its label are centred on each other across the row. For a field's control, as in
/// `LabelColumn::place`, the one with the lower baseline moves down so the two baselines line up, using the baseline
/// `record_control_baseline` stored last pass.
fn place_after(ctx: &egui::Context, input: &LabelIn<'_>, flow: Flow, gap: f32, label: Vec2, label_baseline: f32, first_line: f32) -> Placed {
    let rtl = flow.is_rtl();
    let (control_cell, rest) = flow.take_start(input.control_min.min(flow.rect().width()));
    let (_, rest) = rest.take_start(gap);
    let label_width = label.x.min(rest.rect().width());
    if input.leaf {
        // The control and the label's first line are centred on each other; a wrapped label runs on below.
        let line = control_cell.height().max(first_line);
        let label_drop = (line - first_line) / 2.0;
        let label = Flow::new(rest.rect(), rtl).align(Vec2::new(label_width, label.y), Align::Min, Align::Min).translate(Vec2::new(0.0, label_drop));
        let control = control_cell.translate(Vec2::new(0.0, (line - control_cell.height()) / 2.0));
        return Placed { label, control, trailing: Rect::NOTHING, caption: 0.0 };
    }
    let control_baseline = crate::layout::control_baseline(ctx, input.scope, label, label_baseline);
    let drop = control_baseline.map_or(0.0, |control| control - label_baseline);
    let (label_drop, control_drop) = (drop.max(0.0), (-drop).max(0.0));
    let label_size = Vec2::new(label_width, label.y);
    let label = Flow::new(rest.rect(), rtl).align(label_size, Align::Min, Align::Min).translate(Vec2::new(0.0, label_drop));
    let control = Rect::from_min_max(egui::pos2(control_cell.left(), control_cell.top() + control_drop), control_cell.max);
    Placed { label, control, trailing: Rect::NOTHING, caption: 0.0 }
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
            crate::layout::note_beside(ctx, &crate::layout::LABELS, input.room - gap - input.control_min);
            let (_, rest) = flow.take_start(column);
            rest.take_start(gap).1.rect()
        }
        _ => input.row,
    }
}

/// The label column a control with its label after it (a check box row: a labelled check box, or a field whose label
/// follows its control) starts past. Under a form whose labelled rows share a label column, a row of the column's flow
/// starts past the column and the gap, so its control lines up with theirs, when the row has room for the column, the
/// gap and the control: that column, or 0 while the form has none. `None` where the row stays at its start whatever
/// the column: an inspector's check rows, a row outside the column's flow, and a row too narrow for the column.
fn after_column(ctx: &egui::Context, env: &Env, input: &LabelIn<'_>, gap: f32) -> Option<f32> {
    if input.side.layout != FieldLayout::After || !input.in_column || env.values.get::<LabelScheme>() != LabelScheme::Form {
        return None;
    }
    match crate::layout::ambient_width(ctx, &crate::layout::LABELS, 0.0).filter(|column| *column > 0.0) {
        Some(column) if input.room >= column + gap + input.control_min => {
            crate::layout::note_beside(ctx, &crate::layout::LABELS, input.room - gap - input.control_min);
            Some(column)
        }
        Some(_) => None,
        None => Some(0.0),
    }
}

/// Decides where the label and the control of `input` sit, from `input.control_min`, `input.room` and the group's
/// label column. Shows and records nothing.
pub(super) fn decide(ctx: &egui::Context, env: &Env, input: &LabelIn<'_>, kids: &Ui, leaf: &mut LabelLeaf, glyph: bool, flow: Flow) -> Placed {
    let rtl = env.direction.is_rtl();
    let gap = label_gap(env);
    let after_column = after_column(ctx, env, input, gap);
    let indent = after_column.filter(|column| *column > 0.0).map_or(0.0, |column| column + gap);
    let mut natural = leaf.measure_in(kids, craft_behavior::Proposal::Ideal);
    if input.side.layout == FieldLayout::After && !input.egui_cell {
        // The label takes the room the control and the gap leave, and its detail wraps there.
        let room = (input.row.width() - indent - input.control_min - gap).max(0.0);
        let wraps = natural.x > room;
        // A wider label column leaves the label less room: the row's measure holds while the group's column stays
        // within the column it read, or for a label on one line, within the widest column that keeps it there.
        if let Some(column) = after_column {
            let one_line = input.row.width() - input.control_min - gap - natural.x - gap;
            crate::layout::note_beside(ctx, &crate::layout::LABELS, if wraps { column } else { one_line });
        }
        if wraps {
            natural = leaf.measure_in(kids, craft_behavior::Proposal::Width(room));
        }
    }
    let baseline = leaf.baseline.get();
    // An egui grid's cell is decided at what it needs from its left edge in any direction (`LabelIn::egui_cell`); a
    // row grows at its end edge.
    let grows = if input.egui_cell { Grows::Cell } else { Grows::End };
    if input.side.layout == FieldLayout::Line {
        let number = env.control(input.size).number;
        let flow = if input.egui_cell { widened(flow, natural.x + gap + number, grows) } else { flow };
        let (cell, rest) = flow.take_end(number);
        let width = natural.x.min((rest.rect().width() - gap).max(0.0));
        let drop = (cell.height() - natural.y).max(0.0) / 2.0;
        let label = Flow::new(rest.rect(), rtl).align(Vec2::new(width, natural.y), Align::Min, Align::Min).translate(Vec2::new(0.0, drop));
        return Placed { label, control: cell, trailing: Rect::NOTHING, caption: 0.0 };
    }
    let key = if glyph { crate::layout::GLYPHS } else { crate::layout::LABELS };
    let mut read = false;
    let (column, stack_below) = match input.side.layout {
        FieldLayout::Beside => {
            let own = natural.x;
            // An inline form shares no label column unless it asks for one ([`CaptionColumn`]): each caption is as wide
            // as its own text.
            let shared = env.values.get::<LabelScheme>() != LabelScheme::Inline || env.values.get::<CaptionColumn>().shared();
            let width = crate::layout::ambient_width(ctx, &key, own).filter(|width| shared && *width > 0.0);
            read = width.is_some();
            let width = width.unwrap_or(own);
            (width, width + gap + input.control_min)
        }
        FieldLayout::Above => (natural.x, f32::INFINITY),
        // `Line` returned above; an `Inside` label shows no leaf, so it is never decided.
        FieldLayout::After | FieldLayout::Line | FieldLayout::Inside => (natural.x, 0.0),
        FieldLayout::Caption(cell) => {
            let width = match cell {
                ColumnWidth::Fixed(width) => width.min(natural.x),
                _ => natural.x,
            };
            (width, f32::INFINITY)
        }
    };
    let caption = if matches!(input.side.layout, FieldLayout::Above | FieldLayout::Caption(_)) { column } else { 0.0 };
    // The room decides; a beside label gets the width it needs even where last frame's content was narrower. A label
    // that never sits beside stacks in an unbounded room as well.
    let pinned = (input.content_row || input.egui_cell) && input.side.layout == FieldLayout::Beside;
    let stacks = (input.room < stack_below || stack_below == f32::INFINITY) && !pinned;
    // A label that sits beside the group's column measured so far stacks once the column passes this.
    if read && !stacks && !pinned {
        crate::layout::note_beside(ctx, &key, input.room - gap - input.control_min);
    }
    // A stacked label has the row's width at most, where its name elides; in an egui grid's cell it keeps its own.
    let column = if stacks && !input.egui_cell { column.min(input.row.width()) } else { column };
    // A beside label that does not stack takes its column, the gap and the control's least, past the row's end where
    // the row is narrower. An egui grid's cell is as wide as its label and control need, whatever the layout.
    let need = match input.side.layout {
        FieldLayout::After if input.egui_cell => indent + input.control_min + gap + natural.x,
        _ if input.egui_cell && stacks => column.max(input.control_min),
        _ if stacks => 0.0,
        _ => stack_below,
    };
    let flow = widened(flow, need, grows);

    if input.side.layout == FieldLayout::After {
        let flow = if indent > 0.0 { flow.take_start(indent).1 } else { flow };
        place_after(ctx, input, flow, gap, natural, baseline, leaf.first_line.get())
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
        // The scheme decides where a beside label sits in its column: against the control in a form, at the row's
        // start in an inspector.
        if !placed.stacked && env.values.get::<LabelScheme>() == LabelScheme::Form {
            let spare = (column - label.width()).max(0.0);
            label = label.translate(Vec2::new(if rtl { -spare } else { spare }, 0.0));
        }
        Placed { label, control: placed.control, trailing: Rect::NOTHING, caption }
    }
}

/// How a flow takes the width its label and control need (`widened`).
#[derive(Clone, Copy)]
enum Grows {
    /// A flow that is too narrow grows at its end edge: the left one under RTL.
    End,
    /// An egui grid's cell: exactly as wide as they need from its left edge, in either direction, narrowed as well as
    /// widened. egui offers a grid's last column the rest of the room, where the control would sit at its far edge
    /// under RTL.
    Cell,
}

/// `flow` widened to `need` as `grows` says.
fn widened(flow: Flow, need: f32, grows: Grows) -> Flow {
    let mut wide = flow.rect();
    match grows {
        Grows::Cell => wide.max.x = wide.min.x + need,
        Grows::End if wide.width() >= need => return flow,
        Grows::End if flow.is_rtl() => wide.min.x = wide.max.x - need,
        Grows::End => wide.max.x = wide.min.x + need,
    }
    Flow::new(wide, flow.is_rtl())
}

/// The gap between a label and its control: `space.gap.inline`, `space.gap.item` inside a bar.
pub(crate) fn label_gap(env: &Env) -> f32 {
    let gaps = &env.tokens.space.gap;
    if env.values.get::<crate::contract::container::InBar>().0 { gaps.item } else { gaps.inline }
}

/// Where the control sits once `input.control_min` is known: the decision `run` takes, without showing the label. A
/// measuring run calls it after the body ran, with the control minimum that run recorded.
pub(crate) fn control_after(ui: &mut Ui, env: &Env, input: &LabelIn<'_>) -> Placed {
    let ctx = ui.ctx().clone();
    let flow = Flow::new(input.row, env.direction.is_rtl());
    let content = input.trailing.x > 0.0;
    if input.side.shows_no_leaf(content) {
        return Placed { label: Rect::NOTHING, control: hidden_control(&ctx, input, flow, label_gap(env)), trailing: Rect::NOTHING, caption: 0.0 };
    }
    let reset = scope::last_reset(&ctx, input.scope);
    let slot = if reset.is_some() && input.side.reset_dot { env.tokens.size.handle.sm } else { 0.0 };
    let mut leaf = LabelLeaf::new(input.side.kind.shown(input.text).clone(), slot);
    leaf.resettable = reset.is_some();
    leaf.after = input.side.layout == FieldLayout::After;
    leaf.trailing = input.trailing;
    leaf.named = !input.side.content_only(content);
    leaf.colon = colon(env, input).filter(|_| leaf.named);
    match &input.side.kind {
        LabelKind::Glyph(glyph) => leaf.glyph = Some(glyph.resolved(env.view())),
        _ => {
            leaf.icon = input.side.icon.clone();
            leaf.detail = input.side.detail.as_ref().map(|detail| env.view().resolve(detail));
        }
    }
    let glyph = leaf.glyph.is_some();
    let builder = UiBuilder::new().max_rect(input.row).layout(child::pinned_layout()).scope_id(sub(input.scope, "label_ui"));
    let kids = alloc::child(ui, builder);
    let installed = Env::install_for(&kids, crate::contract::cx::child_env(env, &crate::contract::cx::Under::new(input.enabled, input.size)));
    let placed = decide(&ctx, env, input, &kids, &mut leaf, glyph, flow);
    Env::restore(&ctx, installed);
    placed
}
