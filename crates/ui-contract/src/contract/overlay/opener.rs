//! What opens a free-standing layer: triggers, openers, and the anchor and least width an opener gives.

use super::place::{Anchor, Place};
use super::popups::with_popups;
use super::props::Overlay;
use crate::runtime::egui::areas::{self};
use crate::runtime::egui::response;
use crate::runtime::{keys, pointer};
use craft_behavior::PointerButton;
use egui::{Context, Id, Rect, Response};

/// How a free-standing popup's trigger fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Trigger {
    /// A click on the opener toggles. A root opener (`Overlay::opened_by`) toggles on the primary press inside it, as
    /// pop-up buttons and dropdowns do, so a press held past the click time still opens it; the release that
    /// completes the click is part of that opening.
    Click,
    /// A press on the opener toggles.
    Press,
    /// A context request (right click, the Menu key, AccessKit `ShowContextMenu`) opens.
    Context,
    /// The pointer resting on the opener opens, after the tooltip delay.
    Hover,
    /// A press held for `HOLD` seconds opens.
    Hold,
    /// Only `PopupHandle::open` and the binding open it.
    Manual,
}

/// A component that opens a free-standing popup, and how. A layer with several openers keeps the first as its
/// `Overlay.opener`, which keys the layer's open state, and the others in `others`.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Opener {
    /// The opener's id.
    pub id: Id,
    /// The opener's rect in its layer space.
    pub rect: Rect,
    /// The opener's egui layer: its rect maps to the screen through this layer's transform.
    pub layer: areas::Layer,
    /// The trigger.
    pub trigger: Trigger,
    /// Whether the opener was enabled in the pass that recorded it; a disabled opener closes the layer.
    pub enabled: bool,
    /// Where the layer hangs when this opener opened it. `None` hangs it at the opener's rect, or, for a
    /// `Trigger::Context` opener, at the pointer a release opened it at or the rect its key request named.
    pub anchor: Option<Anchor>,
    /// The other openers of the same layer.
    pub others: Vec<Opener>,
    /// Whether the opener's response fired `trigger` in the pass that recorded it. A container with no `open`
    /// binding applies it to the layer's open slot.
    pub(crate) fired: bool,
    /// How the opener fired, for where the layer hangs.
    pub(crate) opening: Opening,
    /// The opener is pressed with the primary button, for `Trigger::Hold`.
    pub(crate) pressed: bool,
    /// A `Trigger::Click` opener's primary click completed in this pass: the release of the press that opened the
    /// layer, whose activation is that opening and no pick.
    pub(crate) clicked: bool,
}

impl Opener {
    pub(super) fn new(anchor: &Response, trigger: Trigger) -> Opener {
        // The opener's `Response` carries the press path's verdict, for a root with its own egui widget and for an
        // arena member alike: whether the open press is on it and whether a click released on it.
        let pressed_on = response::is_pressed(anchor);
        let request = keys::context_request(&anchor.ctx, anchor.id);
        let (fired, opening, pressed, clicked) = pointer::with_pass_pointer(&anchor.ctx, |pass| {
            let Some(frame) = pass.frame.as_ref() else { return (false, Opening::Plain, false, false) };
            let inside = frame
                .pos
                .is_some_and(|pos| anchor.rect.contains(areas::layer_map(&anchor.ctx, anchor.layer_id).map_or(pos, |map| map.to_layer(pos))));
            let by_pointer = inside && frame.release == Some(PointerButton::Secondary);
            // A root opener opens on the primary press inside it, as a pop-up button does, however long the press is
            // then held; a `Click` opener's release completes the click that press began.
            let fired = match trigger {
                Trigger::Click | Trigger::Press => pressed_on && frame.press.is_some_and(|press| press.button == PointerButton::Primary),
                Trigger::Context => by_pointer || request.is_some(),
                Trigger::Hover | Trigger::Hold | Trigger::Manual => false,
            };
            let opening = match (trigger, request) {
                (Trigger::Context, _) if fired && by_pointer => Opening::Pointer,
                (Trigger::Context, Some(rect)) if fired => Opening::At(rect),
                // Opened through its open binding a pass after the press: it still hangs at the pointer over the opener.
                (Trigger::Context, None) if inside => Opening::Pointer,
                _ => Opening::Plain,
            };
            let clicked = trigger == Trigger::Click && frame.release == Some(PointerButton::Primary) && response::pointer_clicked(anchor);
            (fired, opening, pressed_on && pass.primary_down, clicked)
        });
        Opener {
            id: anchor.id,
            rect: anchor.rect,
            layer: anchor.layer_id,
            trigger,
            enabled: anchor.enabled(),
            anchor: None,
            others: Vec::new(),
            fired,
            opening,
            pressed,
            clicked,
        }
    }

    /// This opener and the others, first first.
    pub(super) fn all(&self) -> impl Iterator<Item = &Opener> {
        std::iter::once(self).chain(&self.others)
    }

    /// The opener the layer's open slot says opened it, else the first.
    /// A layer decides its opening in the pass it opens and keeps it (`stored`, from its `LayerMemory`) while it stays open.
    pub(super) fn active(&self, ctx: &Context, stored: Option<Opening>) -> (&Opener, Opening) {
        let by = with_popups(ctx, |popups| popups.opened_by.get(&self.id).copied());
        let found = by.and_then(|(id, trigger, opening)| {
            self.all().find(|opener| opener.id == id && opener.trigger == trigger).map(|opener| (opener, opening))
        });
        let (opener, opening) = found.unwrap_or((self, self.opening));
        (opener, stored.unwrap_or(opening))
    }
}

/// How an opener opened its layer: what decides where a `Trigger::Context` layer hangs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum Opening {
    /// A press, click, hold or the binding: the layer hangs from the opener.
    #[default]
    Plain,
    /// A secondary-button release: the layer hangs at the pointer.
    Pointer,
    /// A context request by key or AccessKit at this rect in the opener's layer space: a collection's lead row.
    At(Rect),
}

/// The least width a layer asks of its content: its `min_width` rule applied to the width of the opener it hangs
/// from, 0 for a layer with no opener or an anchor that is not a rect.
pub(crate) fn opener_floor(overlay: &Overlay) -> f32 {
    let Some(opener) = &overlay.opener else { return 0.0 };
    let width = match &opener.anchor {
        Some(Anchor::Rect(rect)) => rect.width(),
        Some(Anchor::Response) | None => opener.rect.width(),
        Some(_) => return 0.0,
    };
    overlay.min_width.points(width)
}

/// The anchor a layer opened by `opener` hangs from: the opener's own, else the layer's `Pointer` or `Viewport`
/// place, else the pointer for a context request a secondary release made, else the rect a key's context request
/// named, else the opener's rect.
pub(super) fn opener_anchor(opener: &Opener, opening: Opening, place: &Place) -> Anchor {
    if let Some(anchor) = &opener.anchor {
        return anchor.clone();
    }
    match (place, opening) {
        (Place::Anchored { anchor: anchor @ (Anchor::Pointer | Anchor::Viewport(_)), .. }, _) => anchor.clone(),
        (_, Opening::Pointer) => Anchor::Pointer,
        (_, Opening::At(rect)) => Anchor::Rect(rect),
        (_, Opening::Plain) => Anchor::Response,
    }
}
