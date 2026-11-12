//! The label scope (12.4): a container with a side label records the label's node, and the node-emitting
//! components inside register with it. At the container's report step the runtime patches the nodes in the same
//! frame. The merge rule uses the same path: a `Role::Merged` container's name, description and states go into the
//! one child's node.

use super::emit;
use crate::runtime::egui::{access as adapter, memory as store};
use crate::runtime::env::Message;
use crate::runtime::inspect::Finding;
use crate::runtime::layout_memory;
use crate::runtime::memory::{self, sub};
use crate::runtime::pass_state;
use craft_behavior::PressState;
use craft_theme::{Flags, variant::Tone};
use egui::{Context, Id, Rect, Vec2};

/// Whether a field label names its control or describes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum FieldLabelRole {
    /// The label is the control's name.
    #[default]
    Names,
    /// The control keeps its own name and the label describes it.
    Describes,
}

/// The label a container shows beside its body.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct FieldLabel {
    /// The label's node id.
    pub id: Id,
    /// The label's rect.
    pub rect: Rect,
    /// The label's press this pass, for a control that scrubs by its label.
    pub press: PressState,
    /// The pointer is over the label this pass.
    pub hovered: bool,
    /// The label's text.
    pub text: String,
    /// Whether it names or describes the controls.
    pub role: FieldLabelRole,
    /// A line that joins the controls' description, not their name.
    pub detail: Option<String>,
    /// The label was clicked or activated this pass: focus goes to its control.
    pub activated: bool,
    /// The container shows its control's validity message as its status line (`FooterRegion::status`).
    pub status: bool,
}

impl FieldLabel {
    /// A label of `text` at `rect`, the node `id`.
    pub fn new(id: Id, rect: Rect, text: impl Into<String>) -> Self {
        FieldLabel {
            id,
            rect,
            press: PressState::IDLE,
            hovered: false,
            text: text.into(),
            role: FieldLabelRole::Names,
            detail: None,
            activated: false,
            status: false,
        }
    }

    /// Whether the label names or describes the controls.
    #[must_use]
    pub fn role(mut self, role: FieldLabelRole) -> Self {
        self.role = role;
        self
    }

    /// Adds a detail line to the description of the controls the label covers.
    #[must_use]
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Whether the container shows its control's validity message as its status line.
    #[must_use]
    pub fn status(mut self, status: bool) -> Self {
        self.status = status;
        self
    }

    /// Whether the label was clicked or activated this pass.
    #[must_use]
    pub fn activated(mut self, activated: bool) -> Self {
        self.activated = activated;
        self
    }

    /// The label's press this pass.
    #[must_use]
    pub fn with_press(mut self, press: PressState) -> Self {
        self.press = press;
        self
    }

    /// Whether the pointer is over the label this pass.
    #[must_use]
    pub fn with_hover(mut self, hovered: bool) -> Self {
        self.hovered = hovered;
        self
    }

    /// The label is held down or being dragged.
    pub fn pressed(&self) -> bool {
        self.press.phase.is_press()
    }
}

/// What a scope does at its container's report step.
#[derive(Clone, Debug)]
pub(crate) enum Kind {
    /// The controls inside are named by this label.
    Label(FieldLabel),
    /// The body shows one component whose node takes the container's facts.
    Merge,
}

struct Scope {
    container: Id,
    /// The component whose eviction takes the facts the scope records.
    owner: Id,
    kind: Kind,
    /// The direct controls: node components inside this scope that sit in no other node component of it.
    nodes: Vec<ScopeNode>,
    /// Node components inside this scope whose step 16 has not come yet, outermost first, with the number of focus stops
    /// registered before they began.
    open: Vec<(Id, usize)>,
    /// The screen y of the top of the rect the label step placed the controls in; the recorded baseline and centre
    /// are stored measured from it.
    control_top: f32,
    /// The screen y of the bottom of the first control that reported its line: the label's line is that control's.
    first_bottom: Option<f32>,
    /// The screen y of the first baseline of the first control on the first line.
    baseline: Option<f32>,
    /// The screen y of the centre line of the first control, for a control that paints no text.
    centre: Option<f32>,
    /// The control that registered a reset value and whether its value differs from it.
    reset: Option<ResetFact>,
    /// The first message a control inside reported, with its resolved text.
    message: Option<(Message, Tone, String)>,
    /// A control inside scrubs by the label.
    scrub: bool,
    /// The rect the label offers to its control as part of the control's hit target: the label of a leaf label
    /// step whose child draws only its mark (`Component::LABELLED_AFTER`). The control takes it at its step 8.
    claim: Option<Rect>,
    /// The narrowest width of the first control.
    control_min: Option<f32>,
}

/// A control that reported its node in a scope: the name and description its node got at its own report.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ScopeNode {
    pub id: Id,
    /// The control keeps its own name when a label names it.
    pub keeps_name: bool,
    pub name: Option<String>,
    pub description: Option<String>,
    /// The missing-name finding of a node that may still be named by the scope: reported when it is not.
    pub unnamed: Option<Finding>,
    /// The first focus stop inside a control that has none of its own: where a click on its label moves focus.
    pub inner_stop: Option<crate::runtime::focus::FocusTarget>,
    /// The control is disabled: a label that names only disabled controls is disabled too.
    pub disabled: bool,
}

/// What a control tells its label scope about its reset (10.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResetFact {
    /// The control that takes `Intent::Reset`.
    pub control: Id,
    /// The value differs from the reset value.
    pub differs: bool,
}

#[derive(Default)]
pub(crate) struct Stack(Vec<Scope>);

/// How many label and merge scopes are open around the running code.
pub(crate) fn depth(ctx: &Context) -> usize {
    pass_state::with::<Stack, _>(ctx, |stack| stack.0.len())
}

/// Runs `body` outside every open label and merge scope, as a layer's content runs: a tooltip or popup shown from a
/// control is not part of the field the control sits in, so it neither names, lines up nor sizes that field's label.
pub(crate) fn outside<R>(ctx: &Context, body: impl FnOnce() -> R) -> R {
    let open = pass_state::with::<Stack, _>(ctx, |stack| std::mem::take(&mut stack.0));
    let out = body();
    pass_state::with::<Stack, _>(ctx, |stack| stack.0 = open);
    out
}

/// Opens the scope of `container`; its body shows inside it.
pub(crate) fn begin(ctx: &Context, container: Id, owner: Id, kind: Kind) {
    pass_state::with::<Stack, _>(ctx, |stack| {
        stack.0.push(Scope {
            container,
            owner,
            kind,
            nodes: Vec::new(),
            open: Vec::new(),
            control_top: 0.0,
            first_bottom: None,
            baseline: None,
            centre: None,
            reset: None,
            message: None,
            control_min: None,
            scrub: false,
            claim: None,
        })
    });
}

/// The innermost label scope's label, read by `cx.field_label()`.
pub(crate) fn field_label(ctx: &Context) -> Option<FieldLabel> {
    pass_state::with::<Stack, _>(ctx, |stack| {
        stack.0.iter().rev().find_map(|scope| match &scope.kind {
            Kind::Label(label) => Some(label.clone()),
            Kind::Merge => None,
        })
    })
}

/// Notes that the node component `id` created its node at step 8: the node components that run inside it are
/// not direct controls of the scope.
pub(crate) fn enter(ctx: &Context, id: Id) {
    pass_state::with::<Stack, _>(ctx, |stack| {
        if let Some(scope) = stack.0.last_mut() {
            scope.open.push((id, crate::runtime::focus::stop_count(ctx)));
        }
    });
}

/// Registers the node `id` with the innermost scope at its step 16, when no other node component of the scope
/// holds it. Children reach step 16 first, so they find their parent still open.
pub(crate) fn register(ctx: &Context, mut node: ScopeNode) {
    let unnamed = pass_state::with::<Stack, _>(ctx, |stack| {
        let Some(scope) = stack.0.last_mut() else { return node.unnamed.take() };
        let direct = match scope.open.iter().rposition(|(open, _)| *open == node.id) {
            Some(at) => {
                node.inner_stop = crate::runtime::focus::first_stop_inside(ctx, node.id, scope.open[at].1);
                scope.open.truncate(at);
                at == 0
            }
            None => scope.open.is_empty(),
        };
        if direct {
            scope.nodes.push(node);
            None
        } else {
            node.unnamed.take()
        }
    });
    report_unnamed(ctx, unnamed);
}

fn report_unnamed(ctx: &Context, unnamed: Option<Finding>) {
    if let Some(finding) = unnamed {
        emit::report_finding(ctx, finding);
    }
}

/// Sets the screen y of the top of the rect the label step of `container` placed its controls in. The label lines up
/// with a baseline measured from there: a control that sits lower in that rect (a short piece a row centres beside a
/// taller one) reports a baseline further down.
pub(crate) fn place_controls(ctx: &Context, container: Id, top: f32) {
    pass_state::with::<Stack, _>(ctx, |stack| {
        if let Some(scope) = stack.0.iter_mut().rev().find(|scope| scope.container == container && matches!(scope.kind, Kind::Label(_))) {
            scope.control_top = top;
        }
    });
}

/// Records the line of a control at `rect` in the innermost label scope: its first baseline, or for a control that
/// paints no text its centre line. The label lines up with the first line of what it labels: a baseline counts from a
/// control that starts above the bottom of the first control that reported, so text that wraps onto a later line (a
/// note after a row of chips) does not move the label down; otherwise the label centres on the first control.
pub(crate) fn record_line(ctx: &Context, rect: Rect, baseline: Option<f32>) {
    pass_state::with::<Stack, _>(ctx, |stack| {
        if let Some(scope) = stack.0.iter_mut().rev().find(|scope| matches!(scope.kind, Kind::Label(_))) {
            let first_bottom = *scope.first_bottom.get_or_insert(rect.bottom());
            match baseline {
                Some(baseline) if rect.top() < first_bottom => {
                    scope.baseline.get_or_insert(baseline);
                }
                Some(_) => {}
                None => {
                    scope.centre.get_or_insert(rect.center().y);
                }
            }
        }
    });
}

/// Whether a label scope is open: a control then reports its minimum width.
pub(crate) fn in_label(ctx: &Context) -> bool {
    pass_state::with::<Stack, _>(ctx, |stack| stack.0.iter().any(|scope| matches!(scope.kind, Kind::Label(_))))
}

/// Records the narrowest width of a control inside the innermost label scope. The first control to report wins.
pub(crate) fn record_control_min(ctx: &Context, width: f32) {
    let container = pass_state::with::<Stack, _>(ctx, |stack| {
        stack.0.iter().rev().find(|scope| matches!(scope.kind, Kind::Label(_))).map(|scope| scope.container)
    });
    if let Some(container) = container {
        layout_memory::contribute(ctx, std::sync::Arc::new(ControlMin { container, width }));
    }
}

/// A control's narrowest width reported to the label scope of `container`, as a measuring run hands it forward.
struct ControlMin {
    container: Id,
    width: f32,
}

impl layout_memory::Contribution for ControlMin {
    fn apply(&self, ctx: &Context) {
        pass_state::with::<Stack, _>(ctx, |stack| {
            if let Some(scope) = stack.0.iter_mut().rev().find(|scope| scope.container == self.container && matches!(scope.kind, Kind::Label(_))) {
                scope.control_min.get_or_insert(self.width);
            }
        });
    }

    fn report(&self) -> Option<(Id, f32)> {
        Some((sub(self.container, "control_min"), self.width))
    }

    fn undo(&self, ctx: &Context) -> layout_memory::TakeBack {
        let container = self.container;
        let label = move |scope: &&mut Scope| scope.container == container && matches!(scope.kind, Kind::Label(_));
        let was = pass_state::with::<Stack, _>(ctx, |stack| stack.0.iter_mut().rev().find(label).map(|scope| scope.control_min));
        Box::new(move |ctx| {
            let Some(was) = was else { return };
            pass_state::with::<Stack, _>(ctx, |stack| {
                if let Some(scope) = stack.0.iter_mut().rev().find(label) {
                    scope.control_min = was;
                }
            });
        })
    }
}

/// The narrowest width the first control of the open label scope of `container` has reported so far; 0 before any.
pub(crate) fn open_control_min(ctx: &Context, container: Id) -> f32 {
    pass_state::with::<Stack, _>(ctx, |stack| {
        stack.0.iter().rev().find(|scope| scope.container == container).and_then(|scope| scope.control_min).unwrap_or(0.0)
    })
}

/// The narrowest width the first control of `container`'s label scope reported in this pass's measuring run; 0 when
/// no measuring run ran.
pub(crate) fn measured_control_min(ctx: &Context, container: Id) -> f32 {
    layout_memory::measured(ctx, sub(container, "control_min")).map_or(0.0, |measured| measured.took.x)
}

/// Registers the reset of the control `control` with the innermost label scope.
pub(crate) fn register_reset(ctx: &Context, control: Id, differs: bool) {
    pass_state::with::<Stack, _>(ctx, |stack| {
        if let Some(scope) = stack.0.iter_mut().rev().find(|scope| matches!(scope.kind, Kind::Label(_))) {
            scope.reset.get_or_insert(ResetFact { control, differs });
        }
    });
}

/// Registers the validity message of a control with the innermost label scope; `text` is the message resolved.
/// `tone` is the effective tone of the message. The first message wins. Returns whether that scope's container shows
/// the message as its status line; without one, the control shows it itself.
pub(crate) fn register_message(ctx: &Context, message: &Message, tone: Tone, text: String) -> bool {
    pass_state::with::<Stack, _>(ctx, |stack| {
        let Some(scope) = stack.0.iter_mut().rev().find(|scope| matches!(scope.kind, Kind::Label(_))) else { return false };
        let status = matches!(&scope.kind, Kind::Label(label) if label.status);
        scope.message.get_or_insert_with(|| (message.clone(), tone, text));
        status
    })
}

/// The message the label scope of `container` recorded last frame.
pub(crate) fn last_message(ctx: &Context, container: Id) -> Option<(Message, Tone)> {
    store::peek_clone::<(Message, Tone)>(ctx, message_slot(container))
}

/// The slot of a container's message; it goes with the scope's owner.
fn message_slot(container: Id) -> Id {
    sub(container, "field_message")
}

/// Registers that a control of the innermost label scope scrubs by the label, so the label senses a drag.
pub(crate) fn register_scrub(ctx: &Context) {
    pass_state::with::<Stack, _>(ctx, |stack| {
        if let Some(scope) = stack.0.iter_mut().rev().find(|scope| matches!(scope.kind, Kind::Label(_))) {
            scope.scrub = true;
        }
    });
}

/// Offers `rect`, the label of the scope just begun, to the scope's control as part of its hit target.
pub(crate) fn offer_claim(ctx: &Context, rect: Rect) {
    pass_state::with::<Stack, _>(ctx, |stack| {
        if let Some(scope) = stack.0.last_mut() {
            scope.claim = Some(rect);
        }
    });
}

/// Takes the label rect the innermost label scope offers its control: the first control that asks gets it.
pub(crate) fn claim(ctx: &Context) -> Option<Rect> {
    pass_state::with::<Stack, _>(ctx, |stack| {
        stack.0.iter_mut().rev().find(|scope| matches!(scope.kind, Kind::Label(_))).and_then(|scope| scope.claim.take())
    })
}

/// Whether a control of the label scope of `container` scrubbed by the label last frame.
pub(crate) fn last_scrub(ctx: &Context, container: Id) -> bool {
    store::peek_clone::<bool>(ctx, scrub_slot(container)).unwrap_or(false)
}

/// The slot of a container's scrub fact; it goes with the scope's owner.
fn scrub_slot(container: Id) -> Id {
    sub(container, "scrub")
}

/// The reset fact the label scope of `container` recorded last frame.
pub(crate) fn last_reset(ctx: &Context, container: Id) -> Option<ResetFact> {
    store::peek_clone::<ResetFact>(ctx, reset_slot(container))
}

/// The slot of a container's reset fact; it goes with the scope's owner.
fn reset_slot(container: Id) -> Id {
    sub(container, "reset_fact")
}

/// What a merged container adds to its child's node.
pub(crate) struct Merged {
    pub name: String,
    pub description: Option<String>,
    pub flags: Flags,
    pub enabled: bool,
}

/// One change a label scope makes to a node, decided here and written by `runtime::egui::access`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LabelPatch {
    /// The label's own node: the role `Label` and the label's text, each only where the node has none yet, and
    /// disabled when every control it labels is.
    Label { id: Id, text: String, disabled: bool },
    /// `node` keeps its name and is described by `label`.
    DescribedBy { node: Id, label: Id },
    /// `node` is named `text`, the label's text, and keeps `label` as the node that labels it.
    LabelledBy { node: Id, label: Id, text: String },
    /// `container` is labelled by `label`: it holds several controls.
    GroupLabelledBy { container: Id, label: Id },
    /// `text` joins the description of `node`.
    Detail { node: Id, text: String },
}

/// What `label` does to the `nodes` registered in the scope of `container`: one control is labelled by it
/// (described by it when the control keeps its own name or the label describes), several make the container's node
/// labelled by it, none leaves everything alone.
pub(crate) fn label_patches(container: Id, label: &FieldLabel, nodes: &[ScopeNode]) -> Vec<LabelPatch> {
    if nodes.is_empty() {
        return Vec::new();
    }
    let mut patches = vec![LabelPatch::Label { id: label.id, text: label.text.clone(), disabled: nodes.iter().all(|node| node.disabled) }];
    match nodes {
        [only] if label.role == FieldLabelRole::Describes || only.keeps_name => {
            patches.push(LabelPatch::DescribedBy { node: only.id, label: label.id });
        }
        [only] => patches.push(LabelPatch::LabelledBy { node: only.id, label: label.id, text: label.text.clone() }),
        _ => patches.push(LabelPatch::GroupLabelledBy { container, label: label.id }),
    }
    if let Some(text) = &label.detail {
        let node = match nodes {
            [only] => only.id,
            _ => container,
        };
        patches.push(LabelPatch::Detail { node, text: text.clone() });
    }
    patches
}

/// What a merged container adds to its child's node: the joined name and description and the states.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MergePatch {
    pub child: Id,
    /// The child's name with the container's after it; `None` when the container has none.
    pub name: Option<String>,
    pub description: Option<String>,
    pub disabled: bool,
    pub selected: bool,
    pub toggled: Option<adapter::Toggle>,
    pub invalid: bool,
    pub busy: bool,
}

/// `own` and `added` joined by `separator`, or whichever is not empty.
fn joined(own: Option<&str>, added: &str, separator: &str) -> String {
    match own {
        Some(own) if !own.is_empty() => format!("{own}{separator}{added}"),
        _ => added.to_owned(),
    }
}

/// What the container's `merged` facts do to its one `child`: names join with ", " and descriptions with ". ".
pub(crate) fn merge_patch(child: &ScopeNode, merged: &Merged) -> MergePatch {
    let toggled = if merged.flags.contains(Flags::MIXED) {
        Some(adapter::Toggle::Mixed)
    } else if merged.flags.contains(Flags::CHECKED) {
        Some(adapter::Toggle::On)
    } else {
        None
    };
    MergePatch {
        child: child.id,
        name: (!merged.name.is_empty()).then(|| joined(child.name.as_deref(), &merged.name, ", ")),
        description: merged.description.as_deref().map(|text| joined(child.description.as_deref(), text, ". ")),
        disabled: !merged.enabled,
        selected: merged.flags.contains(Flags::SELECTED),
        toggled,
        invalid: merged.flags.contains(Flags::INVALID),
        busy: merged.flags.contains(Flags::BUSY),
    }
}

/// Closes the scope of `container` for a measuring run: it records only the control's narrowest width, as this pass's
/// measure for the real run's label step.
pub(crate) fn discard(ctx: &Context, container: Id) {
    let scope =
        pass_state::with::<Stack, _>(ctx, |stack| stack.0.last().is_some_and(|scope| scope.container == container).then(|| stack.0.pop()).flatten());
    if let Some(scope) = scope {
        let took = Vec2::new(scope.control_min.unwrap_or(0.0), 0.0);
        layout_memory::emit_record(ctx, sub(container, "control_min"), layout_memory::Measured::new(Vec2::ZERO, took));
    }
}

/// Closes the scope of `container`, if it opened one, and patches the nodes registered in it. `merged` is the
/// container's own facts, read by a merge scope.
pub(crate) fn finish(ctx: &Context, container: Id, merged: Option<Merged>) {
    let scope = pass_state::with::<Stack, _>(ctx, |stack| {
        (stack.0.last().is_some_and(|scope| scope.container == container)).then(|| stack.0.pop()).flatten()
    });
    let Some(scope) = scope else { return };
    match scope.kind {
        Kind::Label(label) => {
            let patches = label_patches(scope.container, &label, &scope.nodes);
            let slot = carried_slot(scope.container);
            if patches.iter().any(|patch| matches!(patch, LabelPatch::LabelledBy { .. })) {
                store::put(ctx, slot, label.text.clone());
                memory::adopt::<String>(ctx, scope.owner, slot);
            } else {
                let _ = store::take::<String>(ctx, slot);
            }
            for patch in &patches {
                adapter::write_label_patch(ctx, patch);
                if let LabelPatch::LabelledBy { node, label, .. } = patch
                    && crate::runtime::inspect::recording(ctx)
                {
                    crate::runtime::inspect::labelled(ctx, *node, *label);
                }
            }
            for node in &scope.nodes {
                let named = patches.iter().any(|patch| matches!(patch, LabelPatch::LabelledBy { node: id, .. } if *id == node.id));
                report_unnamed(ctx, node.unnamed.clone().filter(|_| !named));
            }
            // A click on a label, as on any form label, moves focus to its one control.
            if let (true, [only]) = (label.activated, &scope.nodes[..])
                && !only.disabled
            {
                crate::runtime::focus::request(ctx, only.inner_stop.unwrap_or(crate::runtime::focus::FocusTarget::Id(only.id)));
            }
            if let Some(baseline) = scope.baseline {
                layout_memory::record(ctx, sub(scope.container, "baseline"), Vec2::new(0.0, baseline - scope.control_top));
            }
            if let (None, Some(centre)) = (scope.baseline, scope.centre) {
                layout_memory::record(ctx, sub(scope.container, "centre"), Vec2::new(0.0, centre - scope.control_top));
            }
            let slot = reset_slot(scope.container);
            match scope.reset {
                Some(fact) => {
                    store::put(ctx, slot, fact);
                    memory::adopt::<ResetFact>(ctx, scope.owner, slot);
                }
                None => drop(store::take::<ResetFact>(ctx, slot)),
            }
            let message_slot = message_slot(scope.container);
            match scope.message {
                Some((message, tone, text)) => {
                    store::put(ctx, message_slot, (message, tone));
                    memory::adopt::<(Message, Tone)>(ctx, scope.owner, message_slot);
                    adapter::write_label_patch(ctx, &LabelPatch::Detail { node: scope.container, text });
                }
                None => drop(store::take::<(Message, Tone)>(ctx, message_slot)),
            }
            let scrub_slot = scrub_slot(scope.container);
            if scope.scrub {
                store::put(ctx, scrub_slot, true);
                memory::adopt::<bool>(ctx, scope.owner, scrub_slot);
            } else {
                let _ = store::take::<bool>(ctx, scrub_slot);
            }
        }
        Kind::Merge => {
            debug_assert_eq!(scope.nodes.len(), 1, "a merged container shows exactly one component");
            if let (Some(merged), Some(child)) = (merged, scope.nodes.first()) {
                let patch = merge_patch(child, &merged);
                adapter::write_merge_patch(ctx, &patch);
                report_unnamed(ctx, child.unnamed.clone().filter(|_| patch.name.is_none()));
            } else {
                scope.nodes.iter().for_each(|node| report_unnamed(ctx, node.unnamed.clone()));
            }
        }
    }
}

/// The label text the one control of `container`'s label scope carries as its name this pass, if it does: the
/// container's own node is not named by the label as well.
pub(crate) fn carried_name(ctx: &Context, container: Id) -> Option<String> {
    store::peek_clone::<String>(ctx, carried_slot(container))
}

/// The slot of a container's carried label name; it goes with the scope's owner.
fn carried_slot(container: Id) -> Id {
    sub(container, "carried_name")
}
