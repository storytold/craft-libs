//! Step 16, report: the AccessKit node and its sub-nodes.

use super::stops::takes_focus;
use crate::contract::cx::{CxCore, PartExtras};
use crate::contract::{Component, Cx, Validity};
use crate::primitives::FaceRecord;
use crate::primitives::sensing::PartDecl;
use crate::runtime::access::{self, Access, FaceFacts, Own, Politeness, Role, SetPosition, SubDecl, SubNode, scope};
use crate::runtime::commands::{self, ResolvedCommand};
use crate::runtime::env::{Label, Message};
use crate::runtime::focus::{self};
use crate::runtime::keys;
use crate::style::Flags;
use craft_behavior::Sense;
use egui::Id;

/// What step 16 reads that the pass computed before: whether step 8 made the node and the part declarations.
pub(crate) struct ReportIn<'a> {
    /// The node step 8 created, `None` for a role with no node or while AccessKit is off.
    pub node: bool,
    pub decls: &'a [(PartDecl, PartExtras)],
    pub sense: Sense,
    /// The container's `ContainerProps.live`.
    pub live: Option<Politeness>,
    /// The axes the container's body scrolls on.
    pub scrolls: (bool, bool),
    /// The roving group the component's own stop is in.
    pub group: Option<Id>,
    /// The expanded state of a collapsible container, unless `Component::access` says otherwise.
    pub expanded: Option<bool>,
    /// The position the component reports itself, ahead of its group's and its item facts'.
    pub position: Option<SetPosition>,
}

/// The bound command's hint and shortcut text, which describe and key its node.
fn command_facts(command: &ResolvedCommand, platform: craft_keymap::Platform) -> (Option<&str>, Option<String>) {
    (command.hint.as_deref(), command.chord.as_ref().map(|chord| chord.display(platform)))
}

/// The message of a validity that warns or refuses.
fn validity_of<'c, C: Component>(cx: &'c Cx<'_, C>) -> Option<&'c Message> {
    match &cx.validity {
        Validity::Warn(message) | Validity::Refuse(message) => Some(message),
        _ => None,
    }
}

/// The message of a validity that warns or refuses as text; the node's description when the component gives none.
fn validity_message<C: Component>(cx: &Cx<'_, C>) -> Option<String> {
    validity_of(cx).map(|message| cx.core.env.resolve(&message.text).as_str().to_owned())
}

/// Step 16, report: the AccessKit node filled with its sub-nodes, the label-scope patch and the merge, then the
/// cursor request.
pub(crate) fn report<C: Component>(comp: &C, cx: &Cx<'_, C>, input: &ReportIn<'_>) {
    report_access(comp, cx, input);
    crate::contract::cx::write_cursor(&cx.core);
}

fn report_access<C: Component>(comp: &C, cx: &Cx<'_, C>, input: &ReportIn<'_>) {
    let core = &cx.core;
    let (ctx, id) = (&core.ctx, core.id);
    let mut access = Access::default();
    comp.access(&mut access, cx);
    let value_text = comp.value_text(cx);
    if let Some(text) = &value_text {
        match &mut access.numeric {
            Some(numeric) => numeric.text = Some(text.clone()),
            None => access.value = access.value.take().or_else(|| Some(text.clone())),
        }
    }
    *core.value_text.borrow_mut() = value_text;
    for node in &mut access.nodes {
        if let Some(toggle) = core.toggles.iter().find(|toggle| toggle.part == node.part) {
            node.expanded = Some(toggle.open);
        }
    }
    let role = access.role.unwrap_or(C::ROLE);
    access.access_key = access.access_key.or(cx.core.resolved.access_key);
    if access.grid == access::GridPlace::default() {
        access.grid = cx.core.grid;
    }
    debug_assert!(access.role.is_none_or(|alt| C::ALT_ROLES.contains(&alt)), "`Access::role` picks one of `ALT_ROLES`");
    let flags = core.state.flags;
    let resolved = core.resolved;
    let platform = commands::platform(ctx);
    let (hint, shortcut) = core.command.as_deref().map_or((None, None), |command| command_facts(command, platform));
    let shortcut = shortcut.as_deref();
    for (_, label) in &mut access.custom_actions {
        *label = core.env.resolve(label);
    }
    let validity = validity_message(cx);
    if let (Some(message), Some(text)) = (validity_of(cx), &validity) {
        let tone = message.effective_tone(matches!(cx.validity, Validity::Warn(_)));
        scope::register_message(ctx, message, tone, text.clone());
    }
    let tooltip_text = resolved.tooltip.map(|tooltip| core.env.resolve(&tooltip.spec().text));
    let tooltip = tooltip_text.as_ref().map(Label::as_str);
    if access.live.is_none() {
        access.live = input.live;
    }
    if access.expanded.is_none() {
        access.expanded = input.expanded;
    }
    if access.position.is_none() {
        access.position = input.position;
    }
    if access.position.is_none() {
        access.position =
            input.group.and_then(|group| focus::group_position(ctx, id, group)).or_else(|| core.env.facts_of(id).map(|facts| facts.position));
    }
    let paint = core.paint.borrow();
    if role == Role::Merged {
        let facts = scope::Merged {
            name: resolved.label.to_owned(),
            description: access.description.clone().or_else(|| resolved.description.map(str::to_owned)).or_else(|| validity.clone()),
            flags,
            enabled: resolved.enabled,
        };
        scope::finish(ctx, id, Some(facts));
        return;
    }
    scope::finish(ctx, id, None);
    let carried = scope::carried_name(ctx, id);
    let Role::Node(role) = role else { return };
    if !input.node {
        return;
    }
    let inner_of = access::fold::owner_of(ctx, id);
    let folded = access::fold::take(ctx, id);
    if access.value.is_none() {
        access.value = folded.as_ref().and_then(|folded| folded.value.clone());
    }
    let focusable = takes_focus(resolved) && (input.sense.contains(Sense::FOCUS) || folded.is_some());
    let own = Own {
        id,
        component: std::any::type_name::<C>(),
        role,
        rect: core.rect,
        label: resolved.label,
        names_nothing: access.value.is_none() && carried.as_deref() == Some(resolved.label),
        tooltip,
        elided: paint.elided.as_deref(),
        description: resolved.description,
        validity: validity.as_deref(),
        hint,
        shortcut,
        display: C::INPUT == craft_behavior::Input::Display,
        flags,
        enabled: resolved.enabled,
        read_only: resolved.read_only,
        focusable,
        clickable: resolved.enabled && input.sense.contains(Sense::CLICK),
        keeps_name: resolved.keeps_name,
        scrolls: input.scrolls,
        access: &access,
        popups: &if C::OVERLAYS { crate::contract::overlay::popups_shown(ctx, id) } else { Vec::new() },
        popups_by_part: crate::contract::overlay::has_toggle_nodes(&core.toggles, &access.nodes),
        ltr_axis: resolved.ltr_axis,
    };
    if let Some(owner) = inner_of {
        // Shown as the owner's control: the owner's node is the one node, and stands for this root's key focus.
        access::hide(ctx, id);
        access::fold::offer(ctx, owner, access::fold::Folded { inner: id, value: access.value.clone() });
        if flags.contains(Flags::FOCUSED) {
            access::set_focus(ctx, owner);
        }
    } else {
        access::report(ctx, &own);
    }
    if let Some(folded) = &folded {
        keys::register_node(ctx, id, folded.inner, None, focusable);
    }
    if let Some(facts) = core.text_access.take() {
        crate::runtime::access::text::report_text(ctx, id, &facts);
    }
    if core.member_of.is_some() && flags.contains(Flags::FOCUSED) {
        access::set_focus(ctx, id);
    }
    let mut seen = Vec::new();
    let facts = core.env.facts_of(id);
    for node in &mut access.nodes {
        // A cell of an item row knows its row; its column is the position of its column key among the columns the
        // collection shows, or, for a cell keyed by position, its repeat index.
        let (Some(facts), Role::Node(crate::runtime::egui::access::NodeRole::Cell | crate::runtime::egui::access::NodeRole::GridCell)) =
            (facts, node.role)
        else {
            continue;
        };
        let column = match &node.column {
            Some(key) => facts.columns.iter().position(|column| column == key),
            None => node.part.index.map(|index| index as usize),
        };
        node.grid = access::GridPlace {
            row_index: node.grid.row_index.or(Some(facts.position.index)),
            column_index: node.grid.column_index.or(column),
            ..node.grid
        };
    }
    for node in &access.nodes {
        seen.push(node.part);
        report_sub(core, input, node, paint.faces.iter().find(|face| face.part == node.part));
    }
    for face in paint.faces.iter().filter(|face| !seen.contains(&face.part)) {
        report_sub(core, input, &SubNode::new(face.part, face.role), Some(face));
    }
}

fn report_sub(core: &CxCore<'_>, input: &ReportIn<'_>, node: &SubNode, face: Option<&FaceRecord>) {
    let key = (node.part.key, node.part.index);
    let result = core.results.get(&key);
    let declared = input.decls.iter().find(|(decl, _)| (decl.key, decl.index) == key);
    let mut node = node.clone();
    node.flags |= result.map_or(Flags::empty(), |result| result.flags);
    let decl = SubDecl {
        owner: core.id,
        component: "",
        node: &node,
        rect: result.map_or(core.rect, |result| result.rect),
        enabled: core.resolved.enabled && face.is_none_or(|face| face.enabled),
        focusable: takes_focus(core.resolved) && declared.is_some_and(|(_, extras)| extras.focusable),
        clickable: core.resolved.enabled && declared.is_some_and(|(decl, extras)| decl.sense.contains(Sense::CLICK) && !extras.no_access_click),
        active: core.active == Some(node.part),
        face: face.map(|face| FaceFacts { name: &face.name, flags: face.flags, numeric: face.numeric.as_ref(), expanded: face.expanded }),
        label: core.resolved.label,
        parent: node.under.map_or_else(|| access::fold::owner_of(&core.ctx, core.id).unwrap_or(core.id), |under| access::sub_node_id(core.id, under)),
    };
    access::report_sub(&core.ctx, &decl);
}

/// A node of a component whose size failed the guard: it shows nothing, so it is an unnamed group. The component's own
/// role would promise what the role rules require (a slider's range, a tab's position) of a node that has none.
pub(crate) fn report_inert<C: Component>(cx: &Cx<'_, C>) {
    let (Role::Node(_), Role::Node(role)) = (C::ROLE, Role::GROUP) else { return };
    let core = &cx.core;
    let access = Access { position: core.env.facts_of(core.id).map(|facts| facts.position), ..Access::default() };
    access::report(
        &core.ctx,
        &Own {
            id: core.id,
            component: std::any::type_name::<C>(),
            role,
            rect: core.rect,
            label: "",
            names_nothing: false,
            tooltip: None,
            elided: None,
            description: None,
            validity: None,
            hint: None,
            shortcut: None,
            display: false,
            flags: Flags::empty(),
            enabled: core.resolved.enabled,
            read_only: core.resolved.read_only,
            focusable: false,
            clickable: false,
            keeps_name: false,
            scrolls: (false, false),
            access: &access,
            popups: &[],
            popups_by_part: false,
            ltr_axis: core.resolved.ltr_axis,
        },
    );
}
