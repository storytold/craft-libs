//! Parts that open and close a popup of their component: declaration, step 9's toggles and their sub-nodes.

use super::popups::{OpenOp, PopupHandle, queue};
use crate::contract::PartBuilder;
use crate::contract::cx::CxCore;
use crate::runtime::focus::{self};
use craft_behavior::Intent;
use craft_behavior::PartRef;
use egui::Id;

/// What a part declared with `PartBuilder::toggles_at`: the popup it opens and whether it acts on press.
#[derive(Clone, Copy, Debug)]
#[doc(hidden)]
pub struct ToggleDecl {
    pub popup: &'static str,
    pub index: usize,
    /// The toggle acts on the pass the press began, and a release over the opened popup picks the row under it; off,
    /// it acts on the click's release.
    pub on_press: bool,
}

impl ToggleDecl {
    /// A toggle of the popup `popup` repeat `index`, acting on press.
    pub(crate) const fn new(popup: &'static str, index: usize) -> Self {
        ToggleDecl { popup, index, on_press: true }
    }
}

impl crate::contract::cx::Toggle {
    fn handle(&self, owner: Id) -> PopupHandle {
        PopupHandle::new(owner, self.decl.popup).at(self.decl.index)
    }
}

impl PartBuilder<'_> {
    /// The part opens and closes the repeat `index` of the component's popup `popup`: `toggles` for a component with
    /// one popup per part. Open one popup at a time: opening the second closes the first. Several parts may toggle the
    /// same popup.
    pub fn toggles_at(self, popup: &'static str, index: usize) -> Self {
        self.declared.toggles = Some(ToggleDecl::new(popup, index));
        self
    }

    /// The part's toggle acts on a click's release instead of the press: for a part that also drags (an icon that
    /// moves) or that opens a popover the user works in rather than picks from. Follows `toggles` or `toggles_at`.
    pub fn on_click(self) -> Self {
        let toggle = self.declared.toggles.as_mut();
        debug_assert!(toggle.is_some(), "`on_click` follows `toggles` or `toggles_at`");
        if let Some(toggle) = toggle {
            toggle.on_press = false;
        }
        self
    }
}

/// Step 9, before `update`: what the component's parts declared with `PartBuilder::toggles`. A press on the part
/// (the release of a click, for a part that declared `on_click`) or Enter or Space on it when focused toggles its
/// popup; a release over a row of a popup a press opened picks that row. AccessKit Expand and Collapse
/// on its sub-node open and close it, Alt+Down opens the popup of the focused toggling part (else the first one that
/// is closed), and Alt+Up closes the open ones. Opening one popup of the component closes the others. Each toggle
/// keeps the popup's open state for the part's `expanded`. A click a toggling part took is not left as the root's
/// activation.
pub(crate) fn drive_toggles(core: &mut CxCore<'_>, overlays: bool, name: &str) {
    if core.toggles.is_empty() {
        return;
    }
    debug_assert!(overlays, "`{name}` hosts a popup but sets `OVERLAYS = false`");
    let mut toggles = std::mem::take(&mut core.toggles);
    let owner = core.id;
    let active = focus::active_part(&core.ctx, owner);
    let mut handled = false;
    let mut opened = None;
    for (at, toggle) in toggles.iter().enumerate() {
        let popup = toggle.handle(owner);
        let part = toggle.part;
        // The root takes Expand and Collapse for a sole toggling part that has no node of its own.
        let sole = toggles.len() == 1;
        let named = |request: &craft_behavior::AccessRequest| {
            (request.target == Some(part) || (sole && request.target.is_none()))
                && matches!(request.action, craft_behavior::AccessAction::Expand | craft_behavior::AccessAction::Collapse)
        };
        while let Some(request) = core.intents.take_access_where(named) {
            if request.action == craft_behavior::AccessAction::Expand {
                popup.open_in(core);
                opened = Some(at);
            } else {
                popup.close_in(core);
            }
            handled = true;
        }
        let phase = core.results.get(&(part.key, part.index)).map(|result| result.press.phase);
        let on_press = toggle.decl.on_press;
        let by_pointer = match phase {
            Some(phase) if on_press => matches!(phase, craft_behavior::PressPhase::Down { .. }),
            Some(phase) => phase.is_click(),
            None => false,
        };
        let clicked = |request: &craft_behavior::AccessRequest| request.target == Some(part) && request.action == craft_behavior::AccessAction::Click;
        let by_access = core.intents.take_access_where(clicked).is_some();
        let toggled = by_pointer || by_access || key_activated(core, active, part);
        if toggled {
            popup.registered(core);
            if !popup.is_open_in(core) {
                opened = Some(at);
            }
            queue(&core.ctx, popup.id, OpenOp::Toggle);
            handled = true;
        }
        // The toggling part took the click, or took its press: the root's pointer activation is not a second toggle.
        if (toggled || (on_press && phase.is_some_and(|phase| phase.is_click()))) && core.activation.is_some_and(|activation| !activation.by_key) {
            core.activation = None;
        }
    }
    let mut open = Vec::with_capacity(toggles.len());
    for toggle in &toggles {
        open.push(toggle.handle(owner).is_open_in(core));
    }
    if let Some(keeper) = opened {
        // Opening one popup closes the others. Several parts may toggle one popup (an icon and the name beside it):
        // theirs stays open.
        let kept = toggles[keeper].handle(owner).id;
        for (at, toggle) in toggles.iter().enumerate() {
            if toggle.handle(owner).id != kept && open[at] {
                toggle.handle(owner).close_in(core);
                open[at] = false;
            }
        }
    }
    if open.contains(&true) {
        if core.intents.take(Intent::Close) {
            for (toggle, _) in toggles.iter().zip(&open).filter(|(_, open)| **open) {
                toggle.handle(owner).close_in(core);
            }
        }
        // An open popup keeps Alt+Down: nothing opens beside it.
        core.intents.take(Intent::Open);
    } else if core.intents.take(Intent::Open) && !handled {
        let target = active.and_then(|part| toggles.iter().find(|toggle| toggle.part == part)).or(toggles.first());
        if let Some(toggle) = target {
            toggle.handle(owner).open_in(core);
        }
    }
    for toggle in &mut toggles {
        toggle.open = toggle.handle(owner).is_open_in(core);
    }
    core.toggles = toggles;
}

/// Whether a key (Enter or Space) activated `part`, the focused part of the component.
fn key_activated(core: &CxCore<'_>, active: Option<PartRef>, part: PartRef) -> bool {
    active == Some(part) && core.activation.is_some_and(|activation| activation.by_key)
}

/// Whether `part` is one of the `toggles` parts `toggles` lists with a sub-node in `nodes`: the sub-node reports the
/// popup state and serves Expand and Collapse, and the component's own node does not. A toggling part that is the
/// whole face of the component has no sub-node and leaves them to the root.
pub(crate) fn has_toggle_nodes(toggles: &[crate::contract::cx::Toggle], nodes: &[crate::runtime::access::SubNode]) -> bool {
    nodes.iter().any(|node| toggles.iter().any(|toggle| toggle.part == node.part))
}
