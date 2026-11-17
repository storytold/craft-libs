//! `FooterSlot`: the cell where a container's body places the container's footer (`FooterPlace::Body`).

use crate::contract::{Common, Component, Cx, MeasureCx, Role};
use crate::runtime::pass_state;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{Axis, Metrics, Sense};
use egui::{Context, Id, IdMap, Rect, Response, Vec2};

/// The style of a footer slot: no look of its own.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FooterSlotStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The cell where a container whose footer stands in its body (`FooterPlace::Body`) draws that footer: a dialog's
/// buttons in the cell its body's arrangement keeps for them, as the Color Picker keeps one above its second field
/// group. The slot is as large as the footer measured and takes the width of its cell, so the footer spans the cell.
/// The container runs the footer after the body and draws it at the slot, so the footer's controls, their keys and
/// their order stay the container's. Under no such container the slot takes no room, so a component that offers the
/// cell shows it whether or not a container fills it.
#[derive(crate::contract::Component)]
#[component(sample = FooterSlot::new)]
pub struct FooterSlot {
    #[common]
    common: Common,
}

impl FooterSlot {
    /// A slot for the footer of the container around it.
    pub fn new() -> Self {
        FooterSlot { common: Common::new() }
    }
}

impl Default for FooterSlot {
    fn default() -> Self {
        FooterSlot::new()
    }
}

impl Component for FooterSlot {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = FooterSlotStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: FooterSlotStyleKey) -> FooterSlotStyle {
        FooterSlotStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    /// As wide as its cell: the footer spans it.
    fn fills(&self) -> Option<Axis> {
        Some(Axis::X)
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        cx.env.footer_slot().map_or(Vec2::ZERO, |slot| slot.size)
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        if let Some(slot) = cx.core.env.footer_slot()
            && !crate::runtime::measure::active(&cx.core.ctx)
        {
            pass_state::with::<Slots, _>(&cx.core.ctx, |slots| slots.0.insert(slot.owner, cx.core.rect));
        }
        cx.core.response.clone()
    }
}

/// The rect the footer slot of each container whose footer its body places took in this run, by container.
#[derive(Default)]
pub(crate) struct Slots(IdMap<Rect>);

/// Forgets the slot of `owner` before its body runs, so the footer step reads only a slot this run placed.
pub(crate) fn clear(ctx: &Context, owner: Id) {
    pass_state::with::<Slots, _>(ctx, |slots| slots.0.remove(&owner));
}

/// The rect the body of `owner` placed its footer slot at in this run.
pub(crate) fn take(ctx: &Context, owner: Id) -> Option<Rect> {
    pass_state::with::<Slots, _>(ctx, |slots| slots.0.remove(&owner))
}
