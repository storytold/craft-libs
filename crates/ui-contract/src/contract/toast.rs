//! The toast host (15.5): one layer per window root (its id derives from the scope of the root's body) that lays out
//! the queued toasts and tracks their holds and ends. A component of
//! a higher layer draws each card (`notices::ToastStack`).

use super::overlay::{self, Anchor, Layer, Overlay, Place};
use crate::runtime::egui::memory as store;
use crate::runtime::env::Env;
use crate::runtime::memory::sub;
use crate::runtime::motion;
use crate::runtime::toast::{self, Toast, ToastEnd};
use craft_behavior::overlay::{Align, Side};
use egui::{Align2, IdSalt, Rect, Ui};

/// The toasts of the cards shown last, so the layer's exit fade still shows them after the queue emptied.
#[derive(Clone, Default)]
struct Shown(Vec<Toast>);

/// What a drawn card reports back to the host.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct CardOutcome {
    /// The pointer rests on the card; its timer stays paused.
    pub hovered: bool,
    /// How the user ended the card this pass.
    pub end: Option<ToastEnd>,
}

impl CardOutcome {
    /// A card the pointer rests on when `hovered`, which the user ended as `end`.
    pub fn new(hovered: bool, end: Option<ToastEnd>) -> Self {
        CardOutcome { hovered, end }
    }
}

/// The layer that shows the toast queue of a viewport. The window root calls `show` once per pass, from its own
/// body, so two roots in one viewport keep two layers.
#[derive(Clone, Copy, Debug, Default)]
pub struct ToastHost;

impl ToastHost {
    /// Lays out every queued toast of the running viewport, oldest at the top, in the viewport's end corner, and
    /// draws each with `card(ui, toast, salt)`, where `salt` is stable for the card. A card the pointer rests on keeps
    /// its timer paused; the first show of a toast announces it. When the queue empties the layer fades out with the
    /// last cards.
    ///
    /// The layer takes no room in the body that calls it, so a sizing pass of that body skips it and the real run
    /// opens the layer's stages once per pass.
    pub fn show(ui: &Ui, card: &mut dyn FnMut(&mut Ui, &Toast, IdSalt) -> CardOutcome) {
        let ctx = ui.ctx();
        if ui.is_sizing_pass() {
            return;
        }
        let cards = toast::cards(ctx);
        let env = Env::of(ui);
        let id = sub(ui.scope_id(), "toast_host");
        let open = !cards.is_empty();
        let presence = motion::layer_presence(ctx, id, id, env.motion, open, motion::spec::OPEN_FADE);
        if !presence.visible {
            overlay::forget_layer(ctx, id, id);
            let _ = store::take::<Shown>(ctx, sub(id, "shown"));
            return;
        }
        let toasts: Vec<Toast> = if open {
            cards.iter().map(|card| card.toast.clone()).collect()
        } else {
            store::peek_clone::<Shown>(ctx, sub(id, "shown")).unwrap_or_default().0
        };
        if open {
            store::put(ctx, sub(id, "shown"), Shown(toasts.clone()));
        }
        let overlay = Overlay::toast().place(Place::anchored(Anchor::Viewport(corner(&env)), Side::Above, Align::End));
        let layer =
            Layer { id, owner: id, overlay: &overlay, anchor: Rect::NOTHING, env: &env, exiting: !open, fade: presence.t, size: None, region: None };
        overlay::host(ctx, &layer, &mut |ui| {
            for (at, toast) in toasts.iter().enumerate() {
                let live = cards.get(at);
                let salt = IdSalt::new(("toast", live.map(|live| live.id), live.is_none().then_some(at)));
                let out = card(ui, toast, salt);
                if let Some(live) = live {
                    toast::shown(ctx, live.id, &announcement(&env, toast), out.hovered);
                    if let Some(end) = out.end {
                        toast::close(ctx, live.id, end);
                    }
                }
            }
        });
    }
}

/// The viewport's end-bottom corner: right in left-to-right, left in right-to-left.
fn corner(env: &Env) -> Align2 {
    if env.direction.is_rtl() { Align2::LEFT_BOTTOM } else { Align2::RIGHT_BOTTOM }
}

/// What the card announces when it first shows: the title and the message.
fn announcement(env: &Env, toast: &Toast) -> String {
    let view = env.view();
    let message = view.resolve(&toast.message);
    match &toast.title {
        Some(title) => format!("{}, {}", view.resolve(title).as_str(), message.as_str()),
        None => message.as_str().to_owned(),
    }
}
