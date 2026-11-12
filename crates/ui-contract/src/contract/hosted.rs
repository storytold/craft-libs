//! A leaf a component hosts in a slot of its own (a row's field, a notice's drop hint), erased so the host can keep
//! leaves of different types: measured under the slot's key, then shown once at the rect the host placed it in.

use super::{Component, MeasureCx, PaintUi, Show};
use craft_behavior::Proposal;
use egui::{Rect, Vec2};

/// The key a hosted leaf is measured and shown under: the slot's name and its index among the host's slots.
pub type HostedKey = (&'static str, usize);

/// A hosted leaf of a host `H`, erased.
pub trait Hosted<H: Component> {
    /// The leaf's size at `proposal`, as `show` will show it under `key`.
    fn measure(&mut self, cx: &MeasureCx<'_, H>, key: HostedKey, proposal: Proposal) -> Vec2;

    /// Shows the leaf at `rect` under `key` and hands its output on.
    fn show(self: Box<Self>, ui: &mut PaintUi<'_>, key: HostedKey, rect: Rect);
}

/// A hosted leaf and what takes its output; a hidden one keeps its slot's room and shows nothing.
pub struct Hosting<C, F> {
    component: C,
    output: F,
    shown: bool,
}

impl<C: Show, F: FnOnce(C::Output)> Hosting<C, F> {
    /// `component`, whose output goes to `output` in the pass it is shown.
    pub fn new(component: C, output: F) -> Self {
        Hosting { component, output, shown: true }
    }

    /// Keeps the slot as wide as the component measures and shows nothing in it.
    #[must_use]
    pub fn hidden(mut self) -> Self {
        self.shown = false;
        self
    }
}

impl<H: Component, C: Show, F: FnOnce(C::Output)> Hosted<H> for Hosting<C, F> {
    fn measure(&mut self, cx: &MeasureCx<'_, H>, key: HostedKey, proposal: Proposal) -> Vec2 {
        cx.child(key, &mut self.component, proposal)
    }

    fn show(self: Box<Self>, ui: &mut PaintUi<'_>, key: HostedKey, rect: Rect) {
        if self.shown {
            (self.output)(ui.child(key, rect, self.component));
        }
    }
}
