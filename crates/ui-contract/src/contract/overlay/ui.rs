//! `OverlayUi`: what the `overlay` hook gets for each anchored popup.

use super::host::{Layer, forget_layer, host};
use super::place::Anchor;
use super::placement::resolve_anchor;
use super::popups::{PopupHandle, with_popups};
use super::props::Overlay;
use crate::contract::{BodyCx, Resolved};
use crate::runtime::access::PopupShown;
use crate::runtime::egui::areas::{self};
use crate::runtime::env::Env;
use crate::runtime::memory::MemoryStage;
use crate::runtime::motion::{self};
use crate::runtime::overlay::{self, CloseReason};
use craft_behavior::PartRef;
use egui::{Context, Id, Rect, Ui};
use std::collections::HashMap;

/// What the `overlay` hook gets: one call per anchored popup of this component, built at step 10 with the open state
/// of every popup `cx.popup` registered this pass.
pub struct OverlayUi<'a> {
    pub(crate) ctx: &'a Context,
    pub(crate) owner: Id,
    /// The owner's rect in its own layer space.
    pub(crate) anchor: Rect,
    /// The owner's layer to the screen: anchors are layer-local, popups sit in screen space.
    pub(crate) to_screen: egui::emath::TSTransform,
    pub(crate) resolved: &'a Resolved<'a>,
    pub(crate) env: Env,
    pub(crate) open: HashMap<Id, bool>,
    /// Popups whose dismissal or body request was decided in this pass; step 15 writes their slots.
    #[doc(hidden)]
    pub closed: Vec<(Id, CloseReason)>,
    /// The rects of the owner's toggling parts, in the owner's layer space.
    pub(crate) parts: Vec<(PartRef, Rect)>,
}

/// The component whose overlays are shown: its id, its rect and the rects of its parts that toggle a popup.
pub(crate) struct OverlayOwner {
    pub(crate) id: Id,
    pub(crate) anchor: Rect,
    pub(crate) parts: Vec<(PartRef, Rect)>,
}

impl<'a> OverlayUi<'a> {
    pub(crate) fn new(ctx: &'a Context, owner: OverlayOwner, ui: &Ui, resolved: &'a Resolved<'a>, env: Env, open: HashMap<Id, bool>) -> Self {
        let to_screen = areas::layer_to_screen(ctx, ui.layer_id());
        let OverlayOwner { id, anchor, parts } = owner;
        OverlayUi { ctx, owner: id, anchor, to_screen, resolved, env, open, closed: Vec::new(), parts }
    }

    /// The anchor at the rect of the toggling part `part` of the owner. The layer's placement mirrors the side and
    /// alignment of an anchor in a right-to-left layout, so the same call serves both directions.
    pub fn anchor_part(&self, part: impl Into<PartRef>) -> Anchor {
        let part = part.into();
        let rect = self.parts.iter().find(|(declared, _)| *declared == part).map(|(_, rect)| *rect);
        debug_assert!(rect.is_some(), "`anchor_part` names a part that declared `toggles`");
        Anchor::Rect(rect.unwrap_or(self.anchor))
    }

    /// Shows the popup while its handle is open; the content is a body closure. `None` while closed.
    pub fn show<R>(&mut self, popup: PopupHandle, overlay: Overlay, mut body: impl FnMut(&mut Ui, &mut BodyCx<'_>) -> R) -> Option<R> {
        let Some(open) = self.open.get(&popup.id).copied() else {
            debug_assert!(false, "popup {:?} is shown but was not registered with `cx.popup` this pass", popup.name);
            return None;
        };
        let ctx = self.ctx.clone();
        with_popups(&ctx, |popups| popups.anchors.push((self.owner, PopupShown { id: popup.id, kind: overlay.popup, open })));
        let owner = overlay.opener.as_ref().map_or(self.owner, |opener| opener.id);
        let presence = motion::layer_presence(&ctx, popup.id, owner, self.env.motion, open, overlay.motion);
        if !presence.visible {
            forget_layer(&ctx, popup.id, owner);
            return None;
        }
        let anchor = resolve_anchor(&ctx, popup.id, &overlay, self.anchor, self.to_screen);
        let layer =
            Layer { id: popup.id, owner, overlay: &overlay, anchor, env: &self.env, exiting: !open, fade: presence.t, size: None, region: None };
        let resolved = self.resolved;
        let mut result = None;
        let mut requested = false;
        let out = host(&ctx, &layer, &mut |ui| {
            let mut stage = if ui.is_sizing_pass() { MemoryStage::open_peek(&ctx, popup.id) } else { MemoryStage::open_owned(&ctx, popup.id, owner) };
            let rect = ui.available_rect_before_wrap();
            let mut shown_open = open;
            let env = Env::of(ui);
            let mut bcx = BodyCx::new(popup.id, env.view(), resolved, rect, &mut stage, &mut shown_open, None);
            result = Some(body(ui, &mut bcx));
            requested = bcx.close_requested().is_some();
            stage.write_back();
        });
        if overlay.opener.is_none() && !self.parts.is_empty() {
            overlay::note_peers(&ctx, popup.id, self.parts.iter().map(|(_, rect)| self.to_screen * *rect).collect());
        }
        if open {
            with_popups(&ctx, |popups| popups.shown.push((self.owner, popup.id)));
            if requested {
                self.closed.push((popup.id, CloseReason::Requested));
            } else if let Some(reason) = out.dismissed {
                self.closed.push((popup.id, reason));
            }
        }
        result.filter(|_| open)
    }
}
