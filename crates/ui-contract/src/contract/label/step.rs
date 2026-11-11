//! Running the label step: for a container's row or a leaf child, with its label scope.

use super::leaf::{LabelLeaf, colon};
use super::place::{Placed, decide, hidden_control, label_gap};
use super::props::{FieldLayout, LabelKind, SideLabel};
use crate::contract::Props;
use crate::contract::Show;
use crate::runtime::access::{
    self,
    scope::{self, FieldLabel},
};
use crate::runtime::egui::{alloc, child};
use crate::runtime::env::{Env, Label};
use crate::runtime::keys;
use crate::runtime::memory::sub;
use craft_behavior::Flow;
use egui::{Id, Rect, Ui, UiBuilder, Vec2};

/// What the step needs of the row it runs for.
pub(crate) struct LabelIn<'a> {
    /// The scope the controls register with; the container's own id, or a derived id for `labelled`.
    pub scope: Id,
    /// The component whose eviction takes the label scope's facts.
    pub owner: Id,
    /// The node the label's node sits under in AccessKit: the composite's own id, when it has a node.
    pub parent: Option<Id>,
    pub row: Rect,
    /// The width the mount offers the row. A content-sized row is as wide as its content took, so the choice between
    /// beside and above reads the room, not the row.
    pub room: f32,
    /// The row sits in a column that takes its width from its content: a beside label never stacks for want of room,
    /// since the column is as wide as the label, the gap and the control need.
    pub content_row: bool,
    pub text: &'a Label,
    pub side: &'a SideLabel,
    /// The help line the container's footer shows; it joins the control's description with the label's detail.
    pub help: Option<&'a Label>,
    pub enabled: bool,
    pub size: craft_theme::ControlSize,
    /// The narrowest the control can be; a row too narrow for the label column, the gap and this stacks the label above.
    pub control_min: f32,
    /// The control is a leaf child shown in the flow with its label after it, not a field's body.
    pub leaf: bool,
}

/// The label fact the scope records: the name's text, and the detail and the help line (resolved) as the control's
/// description.
fn field_label(env: &Env, input: &LabelIn<'_>, id: Id, rect: Rect) -> FieldLabel {
    let label = FieldLabel::new(id, rect, input.text.as_str()).role(input.side.role);
    let described: Vec<String> =
        [input.side.detail.as_ref(), input.help].into_iter().flatten().map(|text| env.view().resolve(text).as_str().to_owned()).collect();
    if described.is_empty() { label } else { label.detail(described.join(". ")) }
}

/// What the label step needs of one leaf child it labels (`PaintUi::labelled`, `BodyCx::labelled`).
pub(crate) struct LeafLabel<'a> {
    /// The child's id; the label scope derives from it.
    pub id: Id,
    /// The component whose eviction takes the label scope's facts.
    pub owner: Id,
    /// The node the label's node sits under in AccessKit, when the caller has one.
    pub parent: Option<Id>,
    /// The row the label and the child share.
    pub row: Rect,
    pub text: &'a Label,
    pub side: &'a SideLabel,
    /// The enabled state and size the child inherits.
    pub enabled: bool,
    pub size: craft_theme::ControlSize,
}

/// A leaf child's label step that has run: the rect left for the child and the scope the caller closes.
pub(crate) struct LeafPlaced {
    pub control: Rect,
    /// The label's rect; `Rect::NOTHING` for a hidden label.
    pub label: Rect,
    scope: Id,
}

impl LeafPlaced {
    /// Closes the label scope after the child ran.
    pub(crate) fn finish(self, ctx: &egui::Context) {
        scope::finish(ctx, self.scope, None);
    }
}

/// The label step for one leaf child: measures the child's narrowest width, runs the step in `row` and returns the
/// rect left for the child. The caller shows the child there and then calls `LeafPlaced::finish`.
pub(crate) fn leaf<C: Show>(ui: &mut Ui, env: &Env, at: &LeafLabel<'_>, child: &mut C) -> LeafPlaced {
    let scope = sub(at.id, "field");
    // The child's own `enabled(false)` disables its label too: a disabled field's label does not scrub.
    let enabled = at.enabled && child.common(crate::contract::Internal::new()).enabled.unwrap_or(true);
    let control_min = crate::contract::run::measure_under(child, &*ui, Some(at.id), craft_behavior::Proposal::Min, Some((enabled, at.size))).x;
    let input = LabelIn {
        scope,
        owner: at.owner,
        parent: at.parent,
        row: at.row,
        room: at.row.width(),
        content_row: env.content.0,
        text: at.text,
        side: at.side,
        help: None,
        enabled,
        size: at.size,
        control_min,
        leaf: true,
    };
    let placed = run(ui, env, &input);
    LeafPlaced { control: placed.control, label: placed.label, scope }
}

/// Runs the label step in `ui`: shows the label, opens the label scope and returns where the label and the control
/// sit. The caller closes the scope with `scope::finish` after the control ran.
pub(crate) fn run(ui: &mut Ui, env: &Env, input: &LabelIn<'_>) -> Placed {
    let ctx = ui.ctx().clone();
    let rtl = env.direction.is_rtl();
    let tokens = &env.tokens;
    let (gap, slot_side) = (label_gap(env), tokens.size.handle.sm);
    let reset = scope::last_reset(&ctx, input.scope);
    let flow = Flow::new(input.row, rtl);
    let label_id = sub(input.scope, "label");
    if input.side.kind == LabelKind::Hidden {
        scope::begin(&ctx, input.scope, input.owner, scope::Kind::Label(field_label(env, input, label_id, Rect::NOTHING)));
        return Placed { label: Rect::NOTHING, control: hidden_control(&ctx, input, flow, gap) };
    }
    let slot = if reset.is_some() && input.side.reset_dot { slot_side } else { 0.0 };
    let mut leaf = LabelLeaf::new(input.text.clone(), slot);
    leaf.scrub = input.enabled && (input.side.scrub || scope::last_scrub(&ctx, input.scope));
    leaf.resettable = reset.is_some();
    leaf.differs = reset.is_some_and(|fact| fact.differs);
    leaf.colon = colon(env, input);
    match &input.side.kind {
        LabelKind::Glyph(glyph) => leaf.glyph = Some(glyph.resolved(env)),
        _ => {
            leaf.icon = input.side.icon.clone();
            leaf.detail = input.side.detail.as_ref().map(|detail| env.view().resolve(detail));
        }
    }
    let kids_scope = sub(input.scope, "label_ui");
    let builder = UiBuilder::new().max_rect(input.row).layout(child::pinned_layout()).scope_id(kids_scope);
    let builder = match input.parent {
        Some(parent) => access::with_parent(builder, parent),
        None => builder,
    };
    let mut kids = alloc::child(ui, builder);
    let kid_env = crate::contract::cx::child_env(env, input.enabled, input.size);
    let installed = Env::install_for(&kids, kid_env);
    let glyph = leaf.glyph.is_some();
    let placed = decide(&ctx, env, input, &kids, &mut leaf, glyph, flow);
    let label_id_for_scope = sub(input.scope, "label");
    let shown = Props::id(leaf, label_id_for_scope).show_at(&mut kids, placed.label);
    Env::restore(&ctx, installed);
    if let Some(fact) = reset.filter(|_| shown.double_click || shown.dot_click) {
        keys::route_whole_reset(&ctx, fact.control);
    }
    let label = field_label(env, input, label_id_for_scope, placed.label).with_press(shown.press).activated(shown.activated);
    scope::begin(&ctx, input.scope, input.owner, scope::Kind::Label(label));
    placed
}

/// Runs the label step for one leaf child in the flow of `ui`: `label` sits beside the child or above it, with the
/// icon and detail of `side`, and the child takes the width the label leaves. The row is as high as the placement
/// measured: the child (plus the label line when the label stacks), and the label when it is taller. `id` is the
/// child's id, `owner` the component whose eviction takes the label scope's facts, and `under` the enabled state and
/// size the child inherits.
pub(crate) fn flow_leaf<C: Show>(
    ui: &mut Ui,
    id: Id,
    owner: Id,
    under: (bool, craft_theme::ControlSize),
    label: Label,
    side: SideLabel,
    child: C,
) -> C::Output {
    let (enabled, size) = under;
    let env = Env::of(ui);
    let mut child = child;
    let under = Some(under);
    let ideal = crate::contract::run::measure_under(&mut child, &*ui, Some(id), craft_behavior::Proposal::Ideal, under);
    let avail = ui.available_rect_before_wrap();
    // The child's own height: a stacked label pushes the control down, and the allocation below follows the placement.
    let row = Rect::from_min_size(avail.min, Vec2::new(avail.width(), ideal.y));
    let at = LeafLabel { id, owner, parent: None, row, text: &label, side: &side, enabled, size };
    let placed = leaf(ui, &env, &at, &mut child);
    // In a column that sizes from its content the row is the label plus the control, never the room around them.
    // A row of a horizontal flow is as wide as its label and control: the room it has there is unbounded.
    let content = env.content.0 || ui.layout().is_horizontal();
    let control_width = if content { placed.control.width().min(ideal.x) } else { placed.control.width() };
    let mut control = Rect::from_min_size(placed.control.min, Vec2::new(control_width, ideal.y));
    let rtl = env.direction.is_rtl();
    if content && rtl {
        control = control.translate(Vec2::new(placed.control.width() - control_width, 0.0));
    }
    let extent = if !content {
        avail.width()
    } else if rtl {
        avail.max.x - control.min.x.min(placed.label.min.x)
    } else {
        control.max.x.max(placed.label.max.x) - avail.min.x
    };
    // A control beside its label in a content-sized column reports how far it reaches past the label column, so
    // the column's container measures its rows at the widest label.
    if content && ui.is_sizing_pass() && side.layout == FieldLayout::Beside && placed.label.bottom() > placed.control.top() {
        let column = if matches!(side.kind, LabelKind::Glyph(_)) { crate::layout::GLYPHS } else { crate::layout::LABELS };
        crate::layout::report_beyond(ui.ctx(), std::slice::from_ref(&column), label_gap(&env) + control_width);
    }
    let output = crate::contract::run::show_leaf(crate::contract::Props::id(child, id), ui, crate::contract::run::ShowAt::Rect(control), None);
    let bottom = control.bottom().max(placed.label.bottom());
    placed.finish(ui.ctx());
    alloc::space(ui, Vec2::new(extent, (bottom - row.top()).max(ideal.y)));
    output
}

/// Shows `child` in the flow of `ui`. A component whose `LABELLED_AFTER` is set and that carries a label shows it
/// through the label step, after the control, unless a label scope already names the control (a `Field`, `labelled`;
/// a member of a group, a radio, is named by the group and keeps its own text):
/// that scope's label is the visible text, so the control's own is not shown twice. Every other leaf shows as it is.
pub(crate) fn show_leaf_in_flow<C: Show>(mut child: C, ui: &mut Ui) -> C::Output {
    let env = Env::of(ui);
    let named_by_scope = env.arena.is_none() && scope::in_label(ui.ctx());
    let shows = C::LABELLED_AFTER && child.shows_label() && child.common(crate::contract::Internal::new()).label != Label::Empty && !named_by_scope;
    if !shows {
        return crate::contract::run::show_leaf(child, ui, crate::contract::run::ShowAt::Flow, None);
    }
    let label = std::mem::take(&mut child.common_mut(crate::contract::Internal::new()).label);
    let id = crate::contract::steps::id::<C>(ui, &child.common(crate::contract::Internal::new()).id, false, true);
    let side = SideLabel::new().layout(FieldLayout::After);
    flow_leaf(ui, id, id, (env.enabled, env.size), label, side, child)
}
