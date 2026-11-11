//! Free-standing popups: their open slots and the pass's registry, handles, the `Overlays` trait and hover dwell.

use super::opener::{Opening, Trigger};
use crate::contract::{BindMut, Component, Cx};
use crate::runtime::access::PopupShown;
use crate::runtime::egui::areas::{self};
use crate::runtime::egui::memory::Service;
use crate::runtime::frame;
use crate::runtime::memory::{MemoryStage, sub};
use crate::runtime::overlay::{self, CloseReason};
use craft_behavior::Intent;
use craft_behavior::{EditEvent, Mixed};
use egui::{Context, Id};
use std::collections::HashMap;

/// An id and a name; `Copy`, never a snapshot of the open state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopupHandle {
    pub(super) id: Id,
    pub(super) name: &'static str,
}

impl PopupHandle {
    pub(crate) fn new(opener: Id, name: &'static str) -> Self {
        PopupHandle { id: sub(opener, ("popup", name)), name }
    }

    /// The popup of the repeat `index` of this name: a component with one popup per part or per entry gives each its
    /// own. Index 0 is the handle `cx.popup(name)` returns. `open`, `close`, `is_open` and `trigger` register the
    /// popup for the pass, so a handle derived in `update` needs no separate registration.
    #[must_use]
    pub fn at(self, index: usize) -> PopupHandle {
        match index {
            0 => self,
            _ => PopupHandle { id: sub(self.id, ("at", index)), name: self.name },
        }
    }

    pub(super) fn registered<C: Component>(self, cx: &Cx<'_, C>) {
        register(&cx.core.ctx, cx.core.id, self.id);
    }

    /// The popup's id: its open state's and its entry in the layer stack.
    pub fn id(self) -> Id {
        self.id
    }

    /// The name given to `cx.popup`.
    pub fn name(self) -> &'static str {
        self.name
    }
}

/// The open slot of a popup, in its opener's memory stage: the one owner of the popup's open state.
#[derive(Clone, Copy, Default)]
struct OpenSlot(bool);

/// The value a popup's open binding held after the last pass, to tell a change made outside from the popup's own.
#[derive(Clone, Copy, Default)]
struct BoundOpen(Option<bool>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OpenOp {
    Toggle,
    Open,
    Close,
}

/// Popups registered this pass, the open changes waiting for step 15 and the popups shown this pass.
#[derive(Default)]
pub(super) struct Popups {
    registered: Vec<(Id, Id)>,
    pending: Vec<(Id, OpenOp)>,
    pub(super) shown: Vec<(Id, Id)>,
    /// Every popup an owner showed or kept closed this pass, for the anchor's access node.
    pub(super) anchors: Vec<(Id, PopupShown)>,
    /// The opener that opened each layer with openers, by the layer's first opener: its id and trigger (two
    /// openers on one response differ by trigger) and how it fired.
    pub(super) opened_by: HashMap<Id, (Id, Trigger, Opening)>,
}

pub(super) fn with_popups<R>(ctx: &Context, f: impl FnOnce(&mut Popups) -> R) -> R {
    Service::<Popups>::run(ctx, f)
}

/// Step 8b, before the text part runs: the popups `owner` registers are registered again.
pub(crate) fn begin_owner(ctx: &Context, owner: Id) {
    with_popups(ctx, |popups| {
        popups.registered.retain(|(registered, _)| *registered != owner);
        popups.shown.retain(|(shown, _)| *shown != owner);
        popups.anchors.retain(|(anchor, _)| *anchor != owner);
    });
}

fn register(ctx: &Context, owner: Id, popup: Id) {
    with_popups(ctx, |popups| {
        if !popups.registered.contains(&(owner, popup)) {
            popups.registered.push((owner, popup));
        }
    });
}

pub(super) fn queue(ctx: &Context, popup: Id, op: OpenOp) {
    with_popups(ctx, |popups| popups.pending.push((popup, op)));
}

fn folded(ctx: &Context, popup: Id, slot: bool) -> bool {
    with_popups(ctx, |popups| {
        popups.pending.iter().filter(|(id, _)| *id == popup).fold(slot, |open, (_, op)| match op {
            OpenOp::Toggle => !open,
            OpenOp::Open => true,
            OpenOp::Close => false,
        })
    })
}

/// Whether `popup` is open in this pass: its slot with the changes queued so far applied.
fn open_now(stage: &mut MemoryStage, popup: Id) -> bool {
    let slot = stage.shown::<OpenSlot>(popup).0;
    folded(&stage.ctx().clone(), popup, slot)
}

/// The popups of `owner` in this pass, open or closed, for the anchor's access node.
pub(crate) fn popups_shown(ctx: &Context, owner: Id) -> Vec<PopupShown> {
    with_popups(ctx, |popups| popups.anchors.iter().filter(|(anchor, _)| *anchor == owner).map(|(_, popup)| *popup).collect())
}

/// Whether a popup of `owner` was shown in this pass; step 12 reads it as the `OPEN` flag.
pub(crate) fn popup_shown(ctx: &Context, owner: Id) -> bool {
    with_popups(ctx, |popups| popups.shown.iter().any(|(shown, _)| *shown == owner))
}

/// Step 10: the open state of every popup `owner` registered, with the closes `request_close` queued applied.
pub(crate) fn open_map(stage: &mut MemoryStage, owner: Id) -> HashMap<Id, bool> {
    let ctx = stage.ctx().clone();
    let popups: Vec<Id> =
        with_popups(&ctx, |popups| popups.registered.iter().filter(|(registered, _)| *registered == owner).map(|(_, popup)| *popup).collect());
    popups
        .into_iter()
        .map(|popup| {
            if overlay::take_close(&ctx, popup).is_some() {
                queue(&ctx, popup, OpenOp::Close);
            }
            (popup, open_now(stage, popup))
        })
        .collect()
}

/// Step 10, after the hook: the closes `show` decided become queued changes that step 15 writes.
pub(crate) fn apply_closes(ctx: &Context, closed: Vec<(Id, CloseReason)>) {
    for (popup, _) in closed {
        queue(ctx, popup, OpenOp::Close);
    }
}

/// Step 15: writes the open slots of the popups `owner` registered, then forgets the queued changes.
pub(crate) fn write_open(stage: &mut MemoryStage, owner: Id) {
    let ctx = stage.ctx().clone();
    let popups: Vec<Id> =
        with_popups(&ctx, |popups| popups.registered.iter().filter(|(registered, _)| *registered == owner).map(|(_, popup)| *popup).collect());
    for popup in popups {
        let open = open_now(stage, popup);
        stage.shown::<OpenSlot>(popup).0 = open;
        with_popups(&ctx, |popups| popups.pending.retain(|(id, _)| *id != popup));
    }
}

/// The seconds a press must be held for `Trigger::Hold`.
pub const HOLD: f64 = 0.5;

/// The `Cx` trait that registers popups.
pub trait Overlays {
    /// Registers the popup `name` of this component for this pass and returns its handle. A component registers every
    /// popup its `overlay` hook shows in `update` or earlier; inside `overlay` the call only fetches the handle.
    fn popup(&mut self, name: &'static str) -> PopupHandle;
}

impl<C: Component> Overlays for Cx<'_, C> {
    fn popup(&mut self, name: &'static str) -> PopupHandle {
        let handle = PopupHandle::new(self.core.id, name);
        register(&self.core.ctx, self.core.id, handle.id);
        handle
    }
}

impl PopupHandle {
    /// Toggles or opens the popup from `trigger` at step 9, as the pass's pending open change.
    pub fn trigger<C: Component>(self, trigger: Trigger, cx: &mut Cx<'_, C>) {
        debug_assert!(C::OVERLAYS, "`{}` hosts a popup but sets `OVERLAYS = false`", std::any::type_name::<C>());
        self.registered(cx);
        let core = &mut cx.core;
        let ctx = core.ctx.clone();
        let resting = core.response.hovered() || core.focus.visible;
        let toggled = match trigger {
            Trigger::Click => core.activation.is_some(),
            Trigger::Press => matches!(core.press.phase, craft_behavior::PressPhase::Down { .. }),
            Trigger::Context => core.intents.take(Intent::ContextMenu),
            Trigger::Hover => self.dwell(&ctx, core.memory, "dwell", resting, crate::contract::tooltip::DELAY),
            Trigger::Hold => self.dwell(&ctx, core.memory, "dwell", core.press.phase.is_press(), HOLD),
            Trigger::Manual => false,
        };
        if toggled {
            if trigger == Trigger::Click {
                overlay::claim_activation(&ctx, core.id);
            }
            queue(&ctx, self.id, if matches!(trigger, Trigger::Click | Trigger::Press) { OpenOp::Toggle } else { OpenOp::Open });
        } else if trigger != Trigger::Manual && core.intents.take(Intent::Open) {
            queue(&ctx, self.id, OpenOp::Open);
        } else if trigger == Trigger::Hover && open_now(core.memory, self.id) {
            // The pointer may cross the gap between the opener and the popup: the popup closes after a grace period
            // with the pointer on neither.
            let over = resting || pointer_in_popup(&ctx, self.id);
            if self.dwell(&ctx, core.memory, "leave", !over, crate::contract::tooltip::GRACE) {
                queue(&ctx, self.id, OpenOp::Close);
            }
        }
    }

    fn dwell(self, ctx: &Context, stage: &mut MemoryStage, name: &'static str, held: bool, delay: f64) -> bool {
        dwell(ctx, stage, self.id, name, held, delay)
    }

    /// Queues an open that step 15 writes. For `Trigger::Manual` this and the binding are the only ways it opens.
    pub fn open<C: Component>(self, cx: &mut Cx<'_, C>) {
        debug_assert!(C::OVERLAYS, "`{}` hosts a popup but sets `OVERLAYS = false`", std::any::type_name::<C>());
        self.registered(cx);
        queue(&cx.core.ctx, self.id, OpenOp::Open);
    }

    /// Queues a close; step 15 writes the slot, so this pass still shows the popup open.
    pub fn close<C: Component>(self, cx: &mut Cx<'_, C>) {
        debug_assert!(C::OVERLAYS, "`{}` hosts a popup but sets `OVERLAYS = false`", std::any::type_name::<C>());
        self.registered(cx);
        queue(&cx.core.ctx, self.id, OpenOp::Close);
    }

    /// Whether a toggle, `open` or `close` queued in this pass leaves the popup's open state different from the
    /// state the pass began with. Valid from step 9.
    pub fn changed<C: Component>(self, cx: &mut Cx<'_, C>) -> bool {
        self.registered(cx);
        let began = cx.core.memory.shown::<OpenSlot>(self.id).0;
        open_now(cx.core.memory, self.id) != began
    }

    /// Binds the popup's open state to `open`: call it in `update`, after the toggles and closes the component
    /// queues. A change of the bound value since the last pass opens or closes the popup through the open slot; an
    /// open state that differs from the bound value is written to it. A value the bind refuses to take (a given one)
    /// never forces the popup. Returns whether the popup is open in this pass.
    pub fn bind_open<C: Component>(self, cx: &mut Cx<'_, C>, mut open: BindMut<'_, bool>) -> bool {
        self.registered(cx);
        let began = cx.core.memory.shown::<OpenSlot>(self.id).0;
        let seen = cx.core.memory.session::<BoundOpen>(sub(self.id, "bound")).0.unwrap_or(began);
        let held = match open.read(cx) {
            Some(Mixed::Same(held)) => Some(held),
            _ => None,
        };
        if let Some(held) = held
            && held != seen
        {
            queue(&cx.core.ctx, self.id, if held { OpenOp::Open } else { OpenOp::Close });
        }
        let now = open_now(cx.core.memory, self.id);
        let mut seen = held;
        if let Some(held) = held
            && held != now
        {
            let label = cx.core.label_arc();
            if open.write(cx, &EditEvent::set(Mixed::Same(held), now, label, None)) {
                seen = Some(now);
            }
        }
        cx.core.memory.session::<BoundOpen>(sub(self.id, "bound")).0 = seen;
        now
    }

    /// Whether the popup is open in this pass: valid from step 8, with the changes queued so far applied.
    pub fn is_open<C: Component>(self, cx: &mut Cx<'_, C>) -> bool {
        debug_assert!(C::OVERLAYS, "`{}` hosts a popup but sets `OVERLAYS = false`", std::any::type_name::<C>());
        self.registered(cx);
        open_now(cx.core.memory, self.id)
    }
}

/// Whether the pointer is over the shown layer `popup`: a hover popup stays while the pointer is on it.
fn pointer_in_popup(ctx: &Context, popup: Id) -> bool {
    let shown = overlay::entry(ctx, popup);
    shown.zip(areas::pointer_pos(ctx)).is_some_and(|(entry, at)| entry.rect.contains(at))
}

#[derive(Clone, Copy, Default)]
struct Dwell(Option<f64>);

/// Whether `held` has lasted `delay` seconds, timed in the slot `name` of `id`; asks for a pass at the deadline.
pub(super) fn dwell(ctx: &Context, stage: &mut MemoryStage, id: Id, name: &'static str, held: bool, delay: f64) -> bool {
    let now = frame::clock(ctx);
    let since = stage.shown::<Dwell>(sub(id, name));
    if !held {
        since.0 = None;
        return false;
    }
    let start = *since.0.get_or_insert(now);
    let due = start + delay;
    if now >= due {
        since.0 = None;
        return true;
    }
    crate::runtime::egui::repaint::request_after(ctx, crate::runtime::egui::pass::current_viewport(ctx), due - now);
    false
}

impl From<PopupHandle> for Id {
    fn from(popup: PopupHandle) -> Id {
        popup.id
    }
}
