//! After step 9: the focus stops and the roving groups they join.

use super::parts::PartsStep;
use crate::contract::Resolved;
use crate::runtime::focus::{self, Group, Stop};
use crate::runtime::memory::sub;
use craft_behavior::{Arrows, DocKey, KeyRole, PartRef};
use egui::{Context, Id};

/// The focus stops, registered after `update`: the root's interest when the component or any part is focusable, its
/// `(id, part)` stops, and its roving groups.
pub(crate) struct StopsIn<'a> {
    pub ctx: &'a Context,
    pub id: Id,
    /// The egui widget that holds key focus: the component, or an arena member's root.
    pub holder: Id,
    pub focusable: bool,
    /// The component takes focus (`takes_focus`): its focusable parts are stops.
    pub parts_focus: bool,
    /// The component's one stop is a text part, whose egui widget holds focus.
    pub text: bool,
    /// The text part is read-only: focus reaches it from a press only.
    pub press_only: bool,
    pub keys: focus::KeyUse,
    pub role: KeyRole,
    pub rtl: bool,
    pub parts: &'a PartsStep,
    /// The roving group of the body this component is shown in, when it joined it in this pass (`Cx::join_roving`): its
    /// own stop and its focusable parts join it.
    pub group: Option<Id>,
    /// The roving group of the innermost body this component is shown in, whether or not it joined it.
    pub body: Option<Id>,
    /// The arena root this component is shown in, whether or not it joined the arena.
    pub arena: Option<Id>,
    /// The roving group of the collection row the component is shown in: stops with no group of their own join it.
    pub row: Option<Id>,
    /// The component is the collection item of its row: its parts join the row's group instead of a group of their own.
    pub item: bool,
    /// The component's stop is its scope's default.
    pub default_stop: bool,
    /// The component's one-shot focus request of this pass, taken once its stops are registered.
    pub focus_request: Option<focus::FocusRequest>,
    /// The component's resolved document key: `FocusTarget::Doc` with it focuses the component.
    pub doc_key: Option<DocKey>,
}

/// The id of the roving group a container's body forms.
/// The roving group of a component's own parts.
fn roving_group(id: Id) -> Id {
    sub(id, "roving")
}

/// The roving group a stop shown among a component's parts joins: the body group `joined` when the component joined it
/// in this pass (`Cx::join_roving`), else the group of the component's own parts. The one source for the parts'
/// registration and for a component the header hosts in its cells.
///
/// A part group that nests in the body group (it moves on the crossing axis, so its group was registered with the body
/// group as its parent) keeps its own group: its parts rove among themselves and the body steps over them as one member.
pub(crate) fn parts_group(ctx: &Context, id: Id, joined: Option<Id>) -> Id {
    let own = roving_group(id);
    let nested = joined.is_some_and(|body| focus::with(ctx, |state| state.current_group(own)).is_some_and(|group| group.parent == Some(body)));
    joined.filter(|_| !nested).unwrap_or(own)
}
pub(crate) fn body_group(id: Id) -> Id {
    sub(id, "body_roving")
}

/// Registers the stops, and returns those of the header parts at the end edge, which the body step registers after
/// the header's children.
pub(crate) fn register_stops(input: &StopsIn<'_>) -> Vec<Stop> {
    let ctx = input.ctx;
    debug_assert!(input.parts.rovings.len() <= 1, "a component declares one roving group; a second would be dropped");
    let row_parts = if input.item { input.row } else { None };
    let group = (!input.parts.rovings.is_empty() && row_parts.is_none()).then(|| roving_group(input.id));
    // A part group whose component joined the body's roving group in this pass: its parts are members of the body
    // group, or it is one member of it when it moves on the crossing axis.
    let joined = group.and(input.group);
    let nested = match (joined, input.parts.rovings.first()) {
        (Some(body), Some(roving)) => {
            focus::with(ctx, |state| state.current_group(body)).is_some_and(|body| crosses(roving.arrows, body.roving.arrows))
        }
        _ => false,
    };
    if let (Some(group), Some(roving)) = (group, input.parts.rovings.first()) {
        let arrival = input.parts.decls.iter().rev().find(|(_, extras)| extras.selected && extras.focusable);
        let group = Group::new(group, *roving).rtl(input.rtl);
        let group = match joined.filter(|_| nested) {
            Some(body) => group.parent(body),
            None => group,
        };
        let arrival = arrival.map(|(decl, _)| (input.id, Some(PartRef { key: decl.key, index: decl.index })));
        focus::register_group(ctx, if let Some(arrival) = arrival { group.selected(arrival) } else { group });
    }
    let own = || {
        let stop = Stop::new(input.id, input.keys).root(input.holder).role(input.role).in_body(input.body).in_arena(input.arena);
        let stop = if input.default_stop { stop.default_stop() } else { stop };
        let stop = if input.press_only { stop.press_only() } else { stop };
        if input.text { stop.text() } else { stop }
    };
    if input.focusable {
        focus::register_stop(ctx, if let Some(group) = input.group.or(input.row) { own().in_group(group) } else { own() });
    }
    let part_stops = input.parts_focus && input.parts.decls.iter().any(|(_, extras)| extras.focusable);
    if let Some(key) = input.doc_key.filter(|_| input.focusable || part_stops) {
        focus::register_doc(ctx, key, input.id);
    }
    // A selection part is a press-only stop in no group: neither Tab nor the arrows reach it.
    if input.parts_focus {
        for decl in &input.parts.text.selections {
            let widget = crate::contract::text::selection_id(input.id, decl.part);
            let stop = Stop::new(input.id, decl.keys()).part(decl.part).root(widget).in_body(input.body).press_only();
            focus::register_stop(ctx, stop);
        }
    }
    let mut after_header = Vec::new();
    for (decl, extras) in &input.parts.decls {
        if extras.focusable && input.parts_focus {
            let stop = own().part(PartRef { key: decl.key, index: decl.index });
            let stop = if extras.nudges { stop.role(input.role.nudge(true)) } else { stop };
            let stop = if extras.default_stop { stop.default_stop() } else { stop };
            // A part group that joined the body's roving group has its parts registered there, unless it nests in it.
            let joined = group.map(|_| parts_group(ctx, input.id, input.group.filter(|_| !nested)));
            let stop = if let Some(group) = row_parts.or(joined).or(input.row) { stop.in_group(group) } else { stop };
            if extras.after_header {
                after_header.push(stop);
            } else {
                focus::register_stop(ctx, stop);
            }
        }
    }
    if let Some(request) = input.focus_request {
        focus::take_request(ctx, input.id, request);
    }
    after_header
}

/// Whether a part group moving with `inner` arrows nests in a body group moving with `outer` ones: they take
/// crossing axes (a row of icons in a vertical menu), so the body steps over the parts as one member and the parts
/// rove among themselves. Parts on the body's own axis (a toolbar's command strip) are a run of the body's members.
fn crosses(inner: Arrows, outer: Arrows) -> bool {
    matches!((inner, outer), (Arrows::Horizontal, Arrows::Vertical) | (Arrows::Vertical, Arrows::Horizontal))
}

/// Whether the component is a focus stop: an enabled one, or a disabled one with a reason to show on keyboard focus
/// (testing 22.4): its tooltip's `disabled_text`, or its bound command's hint, which its tooltip shows. Step 9 still
/// drops its activations and edits.
pub(crate) fn takes_focus(resolved: &Resolved<'_>) -> bool {
    resolved.enabled
        || resolved.tooltip.is_some_and(|tooltip| tooltip.spec().disabled_text.is_some())
        || resolved.command.and_then(|command| command.hint.as_deref()).is_some_and(|hint| !hint.is_empty())
}
