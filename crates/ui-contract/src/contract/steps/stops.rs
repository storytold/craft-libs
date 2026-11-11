//! Step 7's tail: the focus stops and the roving groups they join.

use super::parts::PartsStep;
use crate::contract::Resolved;
use crate::runtime::egui::memory as store;
use crate::runtime::focus::{self, Group, Stop};
use crate::runtime::memory::sub;
use craft_behavior::{KeyRole, PartRef};
use egui::{Context, Id};

/// The parts of last pass's complete set in which this pass's results are read; first-frame parts read idle.
/// Step 7's tail: the focus stops. The root's interest when the component or any part is focusable, its
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
    pub keys: focus::KeyUse,
    pub role: KeyRole,
    pub rtl: bool,
    pub parts: &'a PartsStep,
    /// The roving group of the body this component is shown in: its own stop joins it.
    pub group: Option<Id>,
    /// The roving group of the innermost body this component is shown in, whether or not it joined it.
    pub body: Option<Id>,
    /// The roving group of the collection row the component is shown in: stops with no group of their own join it.
    pub row: Option<Id>,
    /// The component is the collection item of its row: its parts join the row's group instead of a group of their own.
    pub item: bool,
    /// The component's stop is its scope's default.
    pub default_stop: bool,
}

/// The id of the roving group a container's body forms.
/// Records that `id` is a member of the roving `group` in this pass.
pub(crate) fn join_group(ctx: &Context, group: Id, id: Id) {
    let slot = sub(group, ("member", id));
    store::put(ctx, slot, crate::runtime::pointer::pass_of(ctx));
    crate::runtime::memory::adopt::<u64>(ctx, id, slot);
}

/// `group` when `id` joined it in this pass or the one before, else `None`: a component that is no member keeps its
/// own Tab stop.
pub(crate) fn member_group(ctx: &Context, group: Option<Id>, id: Id) -> Option<Id> {
    let group = group?;
    let pass = crate::runtime::pointer::pass_of(ctx);
    let joined = store::peek_clone::<u64>(ctx, sub(group, ("member", id)))?;
    (joined.saturating_add(1) >= pass).then_some(group)
}

pub(crate) fn body_group(id: Id) -> Id {
    sub(id, "body_roving")
}

pub(crate) fn register_stops(input: &StopsIn<'_>) {
    let ctx = input.ctx;
    debug_assert!(input.parts.rovings.len() <= 1, "a component declares one roving group; a second would be dropped");
    let row_parts = if input.item { input.row } else { None };
    let group = (!input.parts.rovings.is_empty() && row_parts.is_none()).then(|| sub(input.id, "roving"));
    if let (Some(group), Some(roving)) = (group, input.parts.rovings.first()) {
        let arrival = input.parts.decls.iter().rev().find(|(_, extras)| extras.selected && extras.focusable);
        let group = Group::new(group, *roving).rtl(input.rtl);
        let arrival = arrival.map(|(decl, _)| (input.id, Some(PartRef { key: decl.key, index: decl.index })));
        focus::register_group(ctx, if let Some(arrival) = arrival { group.selected(arrival) } else { group });
    }
    let own = || {
        let stop = Stop::new(input.id, input.keys).root(input.holder).role(input.role).in_body(input.body);
        let stop = if input.default_stop { stop.default_stop() } else { stop };
        if input.text { stop.text() } else { stop }
    };
    if input.focusable {
        focus::register_stop(ctx, if let Some(group) = input.group.or(input.row) { own().in_group(group) } else { own() });
    }
    for (decl, extras) in &input.parts.decls {
        if extras.focusable && input.parts_focus {
            let stop = own().part(PartRef { key: decl.key, index: decl.index });
            let stop = if extras.nudges { stop.role(input.role.nudge(true)) } else { stop };
            let stop = if extras.default_stop { stop.default_stop() } else { stop };
            // A part group that joined the body's roving group has its parts registered there.
            let joined = group.and_then(|parts| focus::registration::part_group(ctx, parts));
            focus::register_stop(ctx, if let Some(group) = row_parts.or(joined).or(group).or(input.row) { stop.in_group(group) } else { stop });
        }
    }
}

/// Whether the component is a focus stop: an enabled one, or a disabled one whose tooltip has a `disabled_text`, which
/// shows its reason on keyboard focus (testing 22.4). Step 9 still drops its activations and edits.
pub(crate) fn takes_focus(resolved: &Resolved<'_>) -> bool {
    resolved.enabled || resolved.tooltip.is_some_and(|tooltip| tooltip.spec().disabled_text.is_some())
}
