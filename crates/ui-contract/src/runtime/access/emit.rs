//! Step 16's fill of the nodes step 8 created, and of the sub-nodes below them (12.2).

use super::{Access, GridPlace, Name, Numeric, PopupKind, Role, SetPosition, SubNode, scope};
use crate::runtime::egui::access::{self as adapter, NodeAction, NodeRole, NodeSpec, Toggle, write_node};
use crate::runtime::env::Label;
use crate::runtime::inspect::{self, Finding};
use crate::runtime::{keys, memory::sub};
use craft_behavior::{PartRef, RoleRule, is_checkable, is_selectable, role_violations};
use craft_theme::Flags;
use egui::{Context, Id, Rect};

/// What step 16 knows about a component's own node.
pub(crate) struct Own<'a> {
    pub id: Id,
    pub component: &'static str,
    pub role: NodeRole,
    pub rect: Rect,
    pub label: &'a str,
    /// The node's one control carries its label as its name: the node itself has no name.
    pub names_nothing: bool,
    pub tooltip: Option<&'a str>,
    /// The full text of the first elided run, the name when nothing else names the node.
    pub elided: Option<&'a str>,
    pub description: Option<&'a str>,
    /// What the control's check says while it warns or refuses: the description after `Common.description`,
    /// until a field container's label scope shows it as the field's error text.
    pub validity: Option<&'a str>,
    /// The bound command's hint.
    pub hint: Option<&'a str>,
    /// The bound command's shortcut.
    pub shortcut: Option<&'a str>,
    /// A display component describes itself with its tooltip.
    pub display: bool,
    pub flags: Flags,
    pub enabled: bool,
    pub read_only: bool,
    pub focusable: bool,
    pub clickable: bool,
    pub keeps_name: bool,
    /// The axes the component's body scrolls on, vertical then horizontal: the node serves the scroll actions.
    pub scrolls: (bool, bool),
    pub access: &'a Access,
    /// The popups the component shows this pass.
    pub popups: &'a [PopupShown],
    /// Parts of the component open its popups: their sub-nodes report the expanded state and serve Expand and
    /// Collapse, so the component's own node does not.
    pub popups_by_part: bool,
    /// The component's horizontal axis keeps its left-to-right order under a right-to-left interface.
    pub ltr_axis: bool,
}

/// A popup an anchor shows this pass.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PopupShown {
    /// The popup's layer id: the id of its root node.
    pub id: Id,
    pub kind: PopupKind,
    pub open: bool,
}

/// A sub-node and what the pipeline knows about its part.
#[doc(hidden)]
pub struct SubDecl<'a> {
    pub owner: Id,
    pub component: &'static str,
    pub node: &'a SubNode,
    pub rect: Rect,
    pub enabled: bool,
    pub focusable: bool,
    pub clickable: bool,
    /// The part is the component's active part: AccessKit focus goes to it.
    pub active: bool,
    pub face: Option<FaceFacts<'a>>,
    /// The component's label, the name of a sub-node with `Name::FromLabel` and no face.
    pub label: &'a str,
    /// The node the sub-node attaches to.
    pub parent: Id,
}

/// What the face painted for a part adds to the part's sub-node.
#[doc(hidden)]
pub struct FaceFacts<'a> {
    pub name: &'a str,
    pub flags: Flags,
    pub numeric: Option<&'a Numeric>,
    pub expanded: Option<bool>,
}

/// The id of the sub-node of `part` below the component `id`.
#[doc(hidden)]
pub fn sub_node_id(id: Id, part: PartRef) -> Id {
    sub(id, ("node", part))
}

/// The name, or the node that names it.
struct Named {
    text: Option<String>,
    labelled_by: Vec<Id>,
    hidden: bool,
}

fn named(name: &Name, label: &str, fallbacks: &[Option<&str>]) -> Named {
    let (text, labelled_by, hidden) = match name {
        Name::FromLabel => (label.to_owned(), Vec::new(), false),
        Name::Text(text) => (text.clone(), Vec::new(), false),
        Name::Joined(parts) => (parts.join(", "), Vec::new(), false),
        Name::LabelledBy(id) => (String::new(), vec![*id], false),
        Name::Decorative => (String::new(), Vec::new(), true),
    };
    let text = if text.is_empty() && labelled_by.is_empty() && !hidden {
        fallbacks.iter().flatten().find(|text| !text.is_empty()).map_or(text, |text| (*text).to_owned())
    } else {
        text
    };
    Named { text: (!text.is_empty()).then_some(text), labelled_by, hidden }
}

/// The expanded state of a node: an explicit value, else open when `OPEN` or `EXPANDED` is shown this pass. A
/// component that can be open and is closed says so with `Access::expanded`; a node with no state is not
/// expandable, so `Expand` and `Collapse` are declared only where the state exists.
fn expanded_of(flags: Flags, explicit: Option<bool>) -> Option<bool> {
    explicit.or_else(|| flags.intersects(Flags::OPEN | Flags::EXPANDED).then_some(true))
}

/// The state flags of a node.
fn write_states(spec: &mut NodeSpec, flags: Flags, enabled: bool, read_only: bool) {
    spec.disabled = !enabled;
    if flags.contains(Flags::MIXED) {
        spec.toggled = Some(Toggle::Mixed);
    } else if flags.contains(Flags::CHECKED) || is_checkable(spec.role) {
        spec.toggled = Some(if flags.contains(Flags::CHECKED) { Toggle::On } else { Toggle::Off });
    }
    if flags.contains(Flags::SELECTED) || is_selectable(spec.role) {
        spec.selected = Some(flags.contains(Flags::SELECTED));
    }
    spec.invalid = flags.contains(Flags::INVALID);
    spec.busy = flags.contains(Flags::BUSY);
    spec.read_only = read_only || flags.contains(Flags::READ_ONLY);
}

fn write_position(spec: &mut NodeSpec, position: Option<SetPosition>, level: Option<usize>) {
    spec.position = position.map(|position| (position.index, position.size));
    spec.level = level.or_else(|| position.and_then(|position| position.level));
    if let Some(position) = position.filter(|_| spec.role == adapter::NodeRole::Row) {
        spec.grid = GridPlace { row_index: Some(position.index), row_count: position.size, column_index: None, column_count: position.columns };
    }
}

/// The actions the interaction and the numeric facts declare.
fn declare_actions(spec: &mut NodeSpec, focusable: bool, clickable: bool, numeric: Option<&Numeric>) {
    if focusable {
        spec.actions.push(NodeAction::Focus);
    }
    if clickable {
        spec.actions.push(NodeAction::Click);
    }
    if let Some(numeric) = numeric.filter(|numeric| numeric.step.is_some()) {
        // A step that cannot move the value is not offered: no Increment at the range's end, unless the value wraps.
        let stuck = |at_end: bool| at_end && !numeric.wraps;
        if !stuck(numeric.value.is_some_and(|value| value >= *numeric.range.end())) {
            spec.actions.push(NodeAction::Increment);
        }
        if !stuck(numeric.value.is_some_and(|value| value <= *numeric.range.start())) {
            spec.actions.push(NodeAction::Decrement);
        }
    }
    if numeric.is_some() {
        spec.actions.push(NodeAction::SetValue);
    }
}

/// The scroll actions of a node that scrolls: a page along each axis, and an offset.
fn declare_scroll_actions(spec: &mut NodeSpec, (vertical, horizontal): (bool, bool)) {
    if vertical {
        spec.actions.extend([NodeAction::ScrollUp, NodeAction::ScrollDown]);
    }
    if horizontal {
        spec.actions.extend([NodeAction::ScrollLeft, NodeAction::ScrollRight]);
    }
    if vertical || horizontal {
        spec.actions.push(NodeAction::SetScrollOffset);
    }
}

fn custom_actions(spec: &mut NodeSpec, actions: &[(i32, Label)]) {
    spec.custom_actions = actions.iter().map(|(id, label)| (*id, label.as_str().to_owned())).collect();
}

/// Reports what `rule` finds wrong with a node of `component`. A missing name is returned instead when the node
/// may still get one from the label or merge scope it sits in; the scope reports it if the node stays unnamed.
fn report_violations(ctx: &Context, component: &'static str, rule: &RoleRule<'_>, nameable: bool) -> Option<Finding> {
    let mut unnamed = None;
    for missing in role_violations(rule) {
        let finding = Finding { rule: "RoleRule", component, subject: format!("{:?}: {missing}", rule.role) };
        if nameable && missing == "a name" {
            unnamed = Some(finding);
        } else {
            report_finding(ctx, finding);
        }
    }
    unnamed
}

/// Records a finding, and logs it in debug builds.
pub(crate) fn report_finding(ctx: &Context, finding: Finding) {
    if cfg!(debug_assertions) {
        log::warn!("{}: a node needs {}", finding.component, finding.subject);
    }
    inspect::finding(ctx, finding);
}

/// Fills the node step 8 created for `own`, registers it for action routing and in the label scope.
pub(crate) fn report(ctx: &Context, own: &Own<'_>) {
    let access = own.access;
    // A control the label scope names keeps its tooltip as a description, not as a name the label replaces.
    let named_by_scope = own.label.is_empty() && scope::field_label(ctx).is_some_and(|label| label.role == scope::FieldLabelRole::Names);
    let name_tooltip = own.tooltip.filter(|_| !named_by_scope);
    let mut name = named(&access.name, own.label, &[name_tooltip, own.elided]);
    if own.names_nothing {
        name.text = None;
    }
    let description = access
        .description
        .as_deref()
        .or(own.description)
        .or(own.validity)
        .or(own.hint)
        .or_else(|| own.tooltip.filter(|tip| own.display || name.text.as_deref() != Some(*tip)));
    let popup = own.popups.first();
    let expanded = expanded_of(own.flags, access.expanded.or_else(|| popup.filter(|_| !own.popups_by_part).map(|popup| popup.open)));
    let mut relations = access.relations.clone();
    relations.labelled_by.extend(name.labelled_by.iter().copied());
    relations.controls.extend(own.popups.iter().filter(|popup| popup.open).map(|popup| popup.id));
    let numeric = access.numeric.as_ref();
    let mut spec = NodeSpec::new(own.role);
    spec.rect = Some(own.rect);
    spec.hidden = name.hidden;
    spec.label = name.text.clone();
    spec.description = description.map(str::to_owned);
    spec.placeholder = access.placeholder.clone();
    spec.value = access.value.clone();
    spec.color = access.color;
    spec.numeric = numeric.cloned();
    spec.expanded = expanded;
    spec.has_popup = access.has_popup.or_else(|| popup.map(|popup| popup.kind));
    spec.required = access.required;
    spec.url = access.url.clone();
    spec.shortcut = access.shortcut.clone().or_else(|| own.shortcut.map(str::to_owned));
    spec.access_key = access.access_key;
    spec.labelled_by = relations.labelled_by.clone();
    spec.described_by = relations.described_by.clone();
    spec.controls = relations.controls.clone();
    spec.flow_to = relations.flow_to.clone();
    spec.live = access.live;
    spec.current = own.flags.contains(Flags::CURRENT);
    write_position(&mut spec, access.position, access.level);
    spec.grid = GridPlace {
        row_index: spec.grid.row_index.or(access.grid.row_index),
        row_count: spec.grid.row_count.or(access.grid.row_count),
        column_index: spec.grid.column_index.or(access.grid.column_index),
        column_count: spec.grid.column_count.or(access.grid.column_count),
    };
    write_states(&mut spec, own.flags, own.enabled, own.read_only);
    spec.ltr_axis = own.ltr_axis;
    spec.busy |= access.busy;
    if let Some(expanded) = expanded {
        spec.actions.push(if expanded { NodeAction::Collapse } else { NodeAction::Expand });
    }
    declare_actions(&mut spec, own.focusable, own.clickable, numeric);
    declare_scroll_actions(&mut spec, (own.scrolls.0 || access.pans.0, own.scrolls.1 || access.pans.1));
    custom_actions(&mut spec, &access.custom_actions);
    write_node(ctx, own.id, &spec);
    if let Some((anchor, focus)) = &access.text_selection {
        let end = |end: &crate::runtime::access::TextEnd| (sub_node_id(own.id, end.part), end.character);
        adapter::write_selection(ctx, own.id, end(anchor), end(focus));
    }
    let unnamed = report_violations(
        ctx,
        own.component,
        &RoleRule::new(own.role)
            .named(name.text.is_some() || !relations.labelled_by.is_empty() || name.hidden)
            .range(numeric.map(|numeric| &numeric.range))
            .positioned(access.position.is_some())
            .grid(spec.grid.facts())
            .leveled(access.level.is_some())
            .expanded(expanded)
            .controls(!relations.controls.is_empty()),
        true,
    );
    keys::register_node(ctx, own.id, own.id, None, own.focusable);
    keys::register_expanded(ctx, own.id, expanded);
    scope::register(
        ctx,
        scope::ScopeNode {
            id: own.id,
            keeps_name: own.keeps_name,
            name: name.text.clone(),
            description: spec.description.clone(),
            unnamed,
            inner_stop: None,
            disabled: !own.enabled,
        },
    );
}

/// Creates and fills the sub-node of `decl`.
#[doc(hidden)]
pub fn report_sub(ctx: &Context, decl: &SubDecl<'_>) {
    let node = decl.node;
    let id = sub_node_id(decl.owner, node.part);
    let Role::Node(role) = node.role else { return };
    if let Some(run) = &node.run {
        adapter::write_run(
            ctx,
            &adapter::RunNode {
                id,
                parent: decl.parent,
                rect: node.bounds.unwrap_or(decl.rect),
                text: &run.text,
                rtl: run.rtl,
                positions: &run.positions,
                widths: &run.widths,
                links: &node.run_links,
            },
        );
        keys::register_node(ctx, id, decl.owner, Some(node.part), false);
        return;
    }
    let face_name = decl.face.as_ref().map(|face| face.name);
    let name = named(&node.name, face_name.unwrap_or(decl.label), &[]);
    if name.hidden {
        return;
    }
    let flags = node.flags | decl.face.as_ref().map_or(Flags::empty(), |face| face.flags);
    let numeric = node.numeric.as_ref().or_else(|| decl.face.as_ref().and_then(|face| face.numeric));
    let expanded = expanded_of(flags, node.expanded.or_else(|| decl.face.as_ref().and_then(|face| face.expanded)));
    let mut spec = NodeSpec::new(role);
    spec.rect = Some(node.bounds.unwrap_or(decl.rect));
    spec.label = name.text.clone();
    spec.labelled_by = name.labelled_by.clone();
    spec.description = node.description.clone();
    spec.numeric = numeric.cloned();
    spec.value = node.value.clone();
    spec.expanded = expanded;
    spec.access_key = node.access_key;
    spec.shortcut = node.shortcut.clone();
    spec.has_popup = node.has_popup;
    spec.current = node.current || flags.contains(Flags::CURRENT);
    if let Some(expanded) = expanded {
        spec.actions.push(if expanded { NodeAction::Collapse } else { NodeAction::Expand });
    }
    write_position(&mut spec, node.position, node.level);
    spec.grid = node.grid;
    spec.sort = node.sort;
    spec.sortable = node.sortable;
    write_states(&mut spec, flags, decl.enabled, false);
    declare_actions(&mut spec, decl.focusable, decl.clickable, numeric);
    custom_actions(&mut spec, &node.custom_actions);
    adapter::register_parent(ctx, id, decl.parent);
    write_node(ctx, id, &spec);
    report_violations(
        ctx,
        decl.component,
        &RoleRule::new(role)
            .named(name.text.is_some() || !name.labelled_by.is_empty())
            .range(numeric.map(|numeric| &numeric.range))
            .positioned(node.position.is_some())
            .leveled(spec.level.is_some())
            .grid(spec.grid.facts())
            .expanded(expanded),
        false,
    );
    keys::register_node(ctx, id, decl.owner, Some(node.part), decl.focusable);
    keys::register_expanded(ctx, id, expanded);
    if decl.active {
        adapter::set_focus(ctx, id);
    }
}

/// The root node of an overlay layer: the layer's content attaches below it, and a modal layer's root is modal. An
/// exiting layer's root is hidden from the first pass it fades out, so nothing under it is reported while it still
/// paints; the ids stay the same if the layer reopens.
pub(crate) fn report_layer(ctx: &Context, id: Id, content: Id, rect: Rect, modal: bool, exiting: bool) {
    adapter::register_parent(ctx, content, id);
    let mut spec = NodeSpec::layer_root();
    spec.rect = Some(rect);
    spec.modal = modal;
    spec.hidden = exiting;
    write_node(ctx, id, &spec);
    if exiting {
        // The content `Ui` can attach to the viewport rather than to the root (a popup body that names its own
        // parent), so it is hidden itself.
        adapter::hide(ctx, content);
    }
}
