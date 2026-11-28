//! `OverlayUi`: what the `overlay` hook gets for each anchored popup.

use super::host::forget_layer;
use super::place::{Anchor, Place};
use super::placement::PopupRegion;
use super::popups::{PopupHandle, PopupLayer, with_popups};
use super::props::Overlay;
use crate::contract::{
    BodyCx, Common, Component, Container, ContainerProps, Cx, Inner, Internal, Mount, NoValue, Outcome, POPUP_PAD, Props as _, Resolved, Role,
    ShowContainer,
};
use crate::runtime::access::PopupShown;
use crate::runtime::egui::areas::{self};
use crate::runtime::env::Env;
use crate::runtime::memory::sub;
use crate::runtime::motion::{self};
use crate::runtime::overlay::{self, CloseReason, LayerRole};
use crate::style::{ComponentStyle, StyleCx, SurfaceKind};
use craft_behavior::{Metrics, PartRef, Sense};
use egui::{Context, Id, IdMap, Rect, Response, Ui};

/// What the `overlay` hook gets: one call per anchored popup of this component, built at step 10 with the open state
/// of every popup `cx.popup` registered this pass.
pub struct OverlayUi<'a> {
    pub(crate) ctx: &'a Context,
    /// The `Ui` the owner was shown in: a container shown as a popup's layer runs in it.
    ui: &'a mut Ui,
    pub(crate) owner: Id,
    /// The owner's rect in its own layer space.
    pub(crate) anchor: Rect,
    /// The owner's layer to the screen: anchors are layer-local, popups sit in screen space.
    pub(crate) to_screen: egui::emath::TSTransform,
    pub(crate) resolved: &'a Resolved<'a>,
    pub(crate) env: Env,
    pub(crate) open: IdMap<bool>,
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
    pub(crate) fn new(ctx: &'a Context, owner: OverlayOwner, ui: &'a mut Ui, resolved: &'a Resolved<'a>, env: Env, open: IdMap<bool>) -> Self {
        let to_screen = areas::layer_to_screen(ctx, ui.layer_id());
        let OverlayOwner { id, anchor, parts } = owner;
        OverlayUi { ctx, ui, owner: id, anchor, to_screen, resolved, env, open, closed: Vec::new(), parts }
    }

    /// Why `popup` closes at the end of this pass, as its `show` decided (its dismissal, or a close its body asked for);
    /// `None` while it stays open. The component that opened it reads this to settle what the popup was proposing.
    pub fn closing(&self, popup: PopupHandle) -> Option<CloseReason> {
        self.closed.iter().find(|(id, _)| *id == popup.id).map(|(_, reason)| *reason)
    }

    /// Whether a layer the owner opens lays out right to left (`Scoped::layer_direction`): a layer its caller places by
    /// position mirrors its own geometry by it, as an anchored placement mirrors the side.
    pub fn is_rtl(&self) -> bool {
        self.env.layer_direction().is_rtl()
    }

    /// The anchor at the rect of the toggling part `part` of the owner. The layer's placement mirrors the side and
    /// alignment of an anchor in a right-to-left layout, so the same call serves both directions.
    pub fn anchor_part(&self, part: impl Into<PartRef>) -> Anchor {
        let part = part.into();
        let rect = self.parts.iter().find(|(declared, _)| *declared == part).map(|(_, rect)| *rect);
        debug_assert!(rect.is_some(), "`anchor_part` names a part that declared `toggles`");
        Anchor::Rect(rect.unwrap_or(self.anchor))
    }

    /// Keeps the popups this component shows from here on, and every popup opened from them, below the component:
    /// inside the layer room less the part above the component's bottom edge, and above the bottom of the inherited
    /// `PopupRegion` when one is set (the bottom slots of a shell). A menu bar's menus and their submenus never cover
    /// the bar.
    pub fn contain_popups_below(&mut self) {
        let room = super::layer_room(self.ctx);
        let bottom = self.env.get::<PopupRegion>().0.map_or(room.bottom(), |region| region.bottom().min(room.bottom())).max(room.top());
        let top = (self.to_screen * self.anchor).bottom().clamp(room.top(), bottom);
        let region = Rect::from_min_max(egui::pos2(room.left(), top), egui::pos2(room.right(), bottom));
        self.env = self.env.derive(|scoped| {
            scoped.inherited_mut().values.set(PopupRegion(Some(region)));
        });
    }

    /// Shows the popup while its handle is open, on a popup surface, inside `POPUP_PAD`; the content is a body closure.
    /// `None` while closed.
    ///
    /// The popup is a layer container sized from its content: its body is measured in this pass before the layer is
    /// placed, so a popup whose content changes is placed by its new size in the pass that shows it. A child that fills
    /// the width takes the width the popup settles on, not the window's. The popup's memory belongs to the component
    /// that shows it (or to the opener its overlay names), so what its body keeps lasts while that component shows.
    pub fn show<R>(&mut self, popup: PopupHandle, overlay: Overlay, mut body: impl FnMut(&mut Ui, &mut BodyCx<'_>) -> R) -> Option<R> {
        self.show_with(popup, overlay, &mut body)
    }

    fn show_with<R>(&mut self, popup: PopupHandle, overlay: Overlay, body: &mut dyn FnMut(&mut Ui, &mut BodyCx<'_>) -> R) -> Option<R> {
        let Some(open) = self.open.get(&popup.id).copied() else {
            debug_assert!(false, "popup {:?} is shown but was not registered with `cx.popup` this pass", popup.name);
            return None;
        };
        let owner = overlay.opener.as_ref().map_or(self.owner, |opener| opener.id);
        // A closed popup that no longer fades runs nothing: no container run for each closed popup of every pass.
        if !open && !motion::layer_fading(self.ctx, sub(popup.id, "presence")) {
            if overlay.role != LayerRole::InPlaceEditor {
                with_popups(self.ctx, |popups| popups.anchors.push((self.owner, PopupShown { id: popup.id, kind: overlay.popup, open })));
            }
            forget_layer(self.ctx, popup.id, owner);
            return None;
        }
        // A popup with no opener hangs from the owner's rect, which the container's layer reads as a rect anchor. Its
        // body does not scroll: a list in it (a menu's, a dropdown's) scrolls itself within the room.
        let overlay = match (&overlay.opener, &overlay.place) {
            (None, Place::Anchored { anchor: Anchor::Response, .. }) => overlay.anchor(Anchor::Rect(self.anchor)),
            _ => overlay,
        }
        .body_scroll(false);
        let label = self.resolved.label_value.clone();
        let props = ContainerProps::new().surface(SurfaceKind::Popup).pad(POPUP_PAD).mount(Mount::Layer(overlay));
        let container = PopupBody { common: Common::new(), props: ContainerProps { owner: Some(owner), ..props } }.label(label);
        self.show_container_with(popup, container, body).and_then(|shown| shown.inner)
    }

    /// Shows `container` as the layer of `popup`, through the container pipeline: its body step with the move band
    /// and grips, its `placement_changed` and its close. The container is mounted as `Mount::Layer`; it takes the
    /// popup's id as its own and the popup's open state as its open state, so it shows while the handle is open and
    /// fades out after. A rect anchor, a float position, the placed rect or position and the bounds are in the owner's
    /// layer space, and the container's placement changes come back in it. A close the container decides (its dismissal
    /// rules, Escape, a close its body asked for) closes the popup, and [`closing`](Self::closing) reports it, as for
    /// [`show`](Self::show). `None` while the popup is closed.
    pub fn show_container<C: ShowContainer, R>(
        &mut self,
        popup: PopupHandle,
        container: C,
        mut body: impl FnMut(&mut Ui, &mut BodyCx<'_>) -> R,
    ) -> Option<Inner<C::Output, R>> {
        self.show_container_with(popup, container, &mut body)
    }

    fn show_container_with<C: ShowContainer, R>(
        &mut self,
        popup: PopupHandle,
        container: C,
        body: &mut dyn FnMut(&mut Ui, &mut BodyCx<'_>) -> R,
    ) -> Option<Inner<C::Output, R>> {
        let Some(open) = self.open.get(&popup.id).copied() else {
            debug_assert!(false, "popup {:?} is shown but was not registered with `cx.popup` this pass", popup.name);
            return None;
        };
        let Mount::Layer(overlay) = &container.props(Internal::new()).mount else {
            debug_assert!(false, "`show_container` shows a container mounted as `Mount::Layer`");
            return None;
        };
        // An in-place editor is part of its owner: the owner's node reports no popup of it.
        let (kind, peers, reported) = (overlay.popup, overlay.opener.is_none(), overlay.role != LayerRole::InPlaceEditor);
        // A popup with no recorded opener hangs from this component: when the component goes disabled, its anchor is
        // gone, as for an explicit opener that goes disabled (`dismissal`).
        if open && peers && overlay.dismiss.anchor_gone && !self.resolved.enabled {
            crate::runtime::overlay::request_close(self.ctx, popup.id, CloseReason::AnchorGone);
        }
        let ctx = self.ctx.clone();
        with_popups(&ctx, |popups| {
            if reported {
                popups.anchors.push((self.owner, PopupShown { id: popup.id, kind, open }));
            }
            popups.layers.push(PopupLayer { id: popup.id, open, closed: None });
        });
        // The container runs in the env the owner's popups take (`contain_popups_below`), not the owner's parent's.
        let installed = Env::install_for(&*self.ui, self.env.clone());
        let shown = container.id(popup.id).show(&mut *self.ui, body);
        Env::restore(&ctx, installed);
        let closed = with_popups(&ctx, |popups| {
            let at = popups.layers.iter().rposition(|layer| layer.id == popup.id)?;
            popups.layers.remove(at).closed
        });
        self.record_shown(popup, open, peers, reported, closed);
        open.then_some(shown)
    }

    /// What every shown popup records after its layer ran: the owner's toggling parts as the layer's peers (a press on
    /// them is not outside it) unless openers name its anchor, the owner's `OPEN` flag unless the layer is not
    /// `reported` (an in-place editor), and the close the layer decided.
    fn record_shown(&mut self, popup: PopupHandle, open: bool, peers: bool, reported: bool, closed: Option<CloseReason>) {
        if peers && !self.parts.is_empty() {
            overlay::note_peers(self.ctx, popup.id, self.parts.iter().map(|(_, rect)| self.to_screen * *rect).collect());
        }
        if open {
            if reported {
                with_popups(self.ctx, |popups| popups.shown.push((self.owner, popup.id)));
            }
            if let Some(reason) = closed {
                self.closed.push((popup.id, reason));
            }
        }
    }
}

/// The style of a popup body: its surface and padding come from the container.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
pub struct PopupBodyStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What a popup body reports: its response.
#[derive(Debug)]
pub struct PopupBodyOutput {
    response: Response,
}

impl Outcome for PopupBodyOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }

    fn unshown(response: Response) -> Option<Self> {
        Some(PopupBodyOutput { response })
    }
}

/// The layer container [`OverlayUi::show`] shows a popup's body closure in: content-sized, on a popup surface inside
/// `POPUP_PAD`, its memory owned by the component that shows the popup. It adds no node of its own: the layer's node
/// is the popup's.
#[derive(crate::contract::Component)]
#[component(container, unregistered)]
pub struct PopupBody {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl Component for PopupBody {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = PopupBodyStyle;
    type Output = PopupBodyOutput;

    fn style(_: &StyleCx<'_>, _: PopupBodyStyleKey) -> PopupBodyStyle {
        PopupBodyStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> PopupBodyOutput {
        PopupBodyOutput { response: cx.response.clone() }
    }
}

impl Container for PopupBody {}
