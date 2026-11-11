//! The `Overlay` data, its presets and builders, its dismissal rules and the placement changes it reports.

use super::opener::{Opener, Trigger};
use super::place::{Anchor, FloatPlace, OverlayFit, Place, Resize, SizeLimits, ViewportShare};
use crate::runtime::access::PopupKind;
use crate::runtime::focus::{FocusPolicy, Initial};
use crate::runtime::motion::{MotionSpec, spec};
use crate::runtime::overlay::{Backdrop, Rank};
use crate::runtime::windows::ResizeEdge;
use craft_behavior::overlay::{Align, MinWidth, Side};
use craft_behavior::{GesturePhase, OwnerClass};
use egui::{Align2, Pos2, Rect, Response, Vec2};

/// What a press outside the layer and its anchor does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Outside {
    /// Closes the layer; the press reaches the widget below.
    #[default]
    Close,
    /// Nothing.
    Keep,
    /// Closes the layer and the press reaches nothing below.
    Absorb,
}

/// When a layer closes by itself. Escape goes through the focus policy's `CancelRule`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Dismiss {
    /// A press outside.
    pub outside: Outside,
    /// Enter pressed inside and left untaken.
    pub enter: bool,
    /// Focus moved out of the layer's scope.
    pub focus_left: bool,
    /// A component inside was activated. An opener whose activation toggled its popup (a trigger, `Popup::toggle`) and
    /// a row that keeps the chain open (`Cx::keep_open`) do not count.
    pub on_activate: bool,
    /// The anchor is no longer shown.
    pub anchor_gone: bool,
    /// Seconds after opening.
    pub timeout: Option<f32>,
}

impl Dismiss {
    /// An outside press closes; nothing else does.
    pub const fn new() -> Self {
        Dismiss { outside: Outside::Close, enter: false, focus_left: false, on_activate: false, anchor_gone: true, timeout: None }
    }

    /// Sets what an outside press does.
    pub const fn outside(mut self, outside: Outside) -> Self {
        self.outside = outside;
        self
    }

    /// Closes on an untaken Enter.
    pub const fn enter(mut self, on: bool) -> Self {
        self.enter = on;
        self
    }

    /// Closes when focus leaves the scope.
    pub const fn focus_left(mut self, on: bool) -> Self {
        self.focus_left = on;
        self
    }

    /// Closes when a component inside is activated, except an opener toggling its popup.
    pub const fn on_activate(mut self, on: bool) -> Self {
        self.on_activate = on;
        self
    }

    /// Closes after `seconds`.
    pub const fn timeout(mut self, seconds: Option<f32>) -> Self {
        self.timeout = seconds;
        self
    }
}

impl Default for Dismiss {
    fn default() -> Self {
        Dismiss::new()
    }
}

/// Data describing an egui layer. Presets fill focus and dismissal; builders change single fields.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Overlay {
    /// Where it sits.
    pub place: Place,
    /// How it sizes.
    pub fit: OverlayFit,
    /// The narrowest it may be.
    pub min_width: MinWidth,
    /// Whether the user may move it.
    pub movable: bool,
    /// How the user may resize it.
    pub resize: Resize,
    /// Its rank.
    pub rank: Rank,
    /// The layer's key class, for both popup forms.
    pub keys: OwnerClass,
    /// Its focus scope.
    pub focus: FocusPolicy,
    /// Its dismissal rules.
    pub dismiss: Dismiss,
    /// Open and close motion.
    pub motion: MotionSpec,
    /// The component that opens it when it is free-standing.
    pub opener: Option<Opener>,
    /// What the anchor tells assistive technology it opens.
    pub popup: PopupKind,
    /// The size an `OverlayFit::Resizable` layer has before the user resizes it; `None` sizes it from its content.
    pub size: Option<Vec2>,
    /// The least an `OverlayFit::Resizable` layer and the most a movable or resizable layer may be resized to.
    pub size_limits: SizeLimits,
    /// The rect the caller places the layer at, in the layer's parent space. The container reports the user's moves
    /// and resizes as `PlacementChange`s and keeps no placement of its own.
    pub placed: Option<Rect>,
    /// The position the caller places the layer's top left at, in the layer's parent space; its size comes from its
    /// content and size limits. Reported and owned like `placed`.
    pub placed_at: Option<Pos2>,
    /// The rect a move or resize keeps the layer inside.
    pub bounds: Option<Rect>,
    /// The layer's own opacity, below the open and close motion's.
    pub opacity: f32,
    /// Whether a container mounted as this layer scrolls its body vertically. Off for a layer whose body fills it and
    /// manages its own scrolling (a dock tree); a body that is one collection uses `collection_body` instead.
    pub body_scroll: bool,
    /// Whether the layer's move band moves the OS window the layer is shown in, in place of the layer: the chrome a
    /// modal puts over the part of a title bar it covers. The layer stays where it is placed.
    pub moves_window: bool,
}

/// The share of the viewport a dropdown list may take. Half the width leaves room for the list to hang from the end
/// edge of its anchor in a right-to-left layout.
const DROPDOWN_SHARE: ViewportShare = ViewportShare::new(0.5, 0.4);

/// The share of the viewport a dialog or modal may take.
const DIALOG_SHARE: ViewportShare = ViewportShare::new(0.8, 0.9);

/// The share of the viewport a tooltip or a toast card may take before its text wraps.
const NOTE_SHARE: ViewportShare = ViewportShare::new(0.4, 1.0);

impl Overlay {
    fn base(rank: Rank, place: Place, fit: OverlayFit, keys: OwnerClass, focus: FocusPolicy) -> Overlay {
        Overlay {
            place,
            fit,
            min_width: MinWidth::None,
            movable: false,
            resize: Resize::NONE,
            rank,
            keys,
            focus,
            dismiss: Dismiss::new(),
            motion: spec::OPEN_FADE,
            opener: None,
            popup: PopupKind::Dialog,
            size: None,
            size_limits: SizeLimits::default(),
            placed: None,
            placed_at: None,
            bounds: None,
            opacity: 1.0,
            body_scroll: true,
            moves_window: false,
        }
    }

    fn below() -> Place {
        Place::anchored(Anchor::Response, Side::Below, Align::Start)
    }

    /// A popover: a popup anchored below, taking focus in. It closes when the focus moves out of it.
    pub fn popover() -> Overlay {
        let mut overlay = Overlay::base(Rank::Popup, Overlay::below(), OverlayFit::Content, OwnerClass::Popup, FocusPolicy::POPUP);
        overlay.dismiss = Dismiss::new().focus_left(true);
        overlay
    }

    /// A dropdown list: from its content, at least as wide as its anchor and capped, focus stays on the anchor. It
    /// closes when a row is activated.
    pub fn dropdown() -> Overlay {
        let focus = FocusPolicy::POPUP.trap(false).restore(false).initial(Initial::KeepAnchor);
        let mut overlay = Overlay::base(Rank::Popup, Overlay::below(), OverlayFit::ContentCapped(DROPDOWN_SHARE), OwnerClass::PassivePopup, focus);
        overlay.dismiss = Dismiss::new().on_activate(true);
        overlay.min_width = MinWidth::Anchor;
        overlay.popup = PopupKind::Listbox;
        overlay
    }

    /// A menu: a popup the arrow keys move through. A press outside closes it and goes no further.
    pub fn menu() -> Overlay {
        let mut overlay = Overlay::base(Rank::Popup, Overlay::below(), OverlayFit::Content, OwnerClass::Menu, FocusPolicy::MENU);
        overlay.dismiss = Dismiss::new().outside(Outside::Absorb);
        overlay.popup = PopupKind::Menu;
        overlay
    }

    /// A submenu: beside its parent entry.
    pub fn submenu() -> Overlay {
        Overlay::menu().place(Place::anchored(Anchor::Response, Side::End, Align::Start))
    }

    /// A tooltip: below the control, no focus, never takes input.
    pub fn tooltip() -> Overlay {
        let mut overlay =
            Overlay::base(Rank::Tooltip, Overlay::below(), OverlayFit::ContentCapped(NOTE_SHARE), OwnerClass::PassivePopup, FocusPolicy::NONE);
        overlay.dismiss = Dismiss::new().outside(Outside::Keep);
        overlay
    }

    /// A dialog: modal, floating, movable and resizable. `.modeless()` gives rank `Window`.
    pub fn dialog() -> Overlay {
        let mut overlay = Overlay::base(
            Rank::modal(Backdrop::Scrim),
            Place::Float(FloatPlace::centred()),
            OverlayFit::ContentCapped(DIALOG_SHARE),
            OwnerClass::Dialog,
            FocusPolicy::DIALOG,
        );
        overlay.movable = true;
        overlay.resize = Resize::GRIPS;
        overlay.dismiss = Dismiss::new().outside(Outside::Keep);
        overlay
    }

    /// A modal: centred, content capped.
    pub fn modal() -> Overlay {
        let mut overlay = Overlay::dialog();
        overlay.movable = false;
        overlay.resize = Resize::NONE;
        overlay
    }

    /// The chrome of a modal over a part of the window the modal covers, placed at `rect`: above the modal, with no
    /// focus scope and no dismissal of its own. Its move band moves the window the layer is shown in.
    pub fn modal_chrome(rect: Rect) -> Overlay {
        let mut overlay =
            Overlay::base(Rank::ModalChrome, Place::Float(FloatPlace::centred()), OverlayFit::Content, OwnerClass::Widget, FocusPolicy::NONE);
        overlay.dismiss = Dismiss::new().outside(Outside::Keep);
        overlay.placed = Some(rect);
        overlay.body_scroll = false;
        overlay.moves_window = true;
        overlay
    }

    /// A toast stack: the viewport's bottom end corner.
    pub fn toast() -> Overlay {
        let place = Place::anchored(Anchor::Viewport(Align2::RIGHT_BOTTOM), Side::Above, Align::End);
        let mut overlay = Overlay::base(Rank::Toast, place, OverlayFit::ContentCapped(NOTE_SHARE), OwnerClass::Widget, FocusPolicy::NONE);
        overlay.dismiss = Dismiss::new().outside(Outside::Keep);
        overlay
    }

    /// A floating panel or document window: a window-rank layer that is a pane of the app. F6 reaches it and Escape
    /// does not close it.
    pub fn floating() -> Overlay {
        let mut overlay =
            Overlay::base(Rank::Window, Place::Float(FloatPlace::centred()), OverlayFit::Content, OwnerClass::Widget, FocusPolicy::PANE);
        overlay.dismiss = Dismiss::new().outside(Outside::Keep);
        overlay
    }

    /// Whether edge grips resize the layer.
    pub fn resizable(mut self, on: bool) -> Overlay {
        self.resize = if on { Resize::GRIPS } else { Resize::NONE };
        self
    }

    /// Rank `Window` and no backdrop.
    pub fn modeless(mut self) -> Overlay {
        self.rank = Rank::Window;
        self
    }

    /// Sets where it sits.
    pub fn place(mut self, place: Place) -> Overlay {
        self.place = place;
        self
    }

    /// Sets how it sizes.
    pub fn fit(mut self, fit: OverlayFit) -> Overlay {
        self.fit = fit;
        self
    }

    /// Hangs the layer from `anchor` on the side and alignment its preset gave. Without an opener, a point is in the
    /// parent's layer space; a floating layer keeps its place.
    pub fn anchor(mut self, anchor: Anchor) -> Overlay {
        if let Place::Anchored { anchor: hung, .. } = &mut self.place {
            *hung = anchor;
        }
        self
    }

    /// Sets the size a resizable layer has before the user resizes it.
    pub fn size(mut self, size: Vec2) -> Overlay {
        self.size = Some(size);
        self
    }

    /// Sets the least a resizable layer may be resized to.
    pub fn min_size(mut self, min_size: Vec2) -> Overlay {
        self.size_limits.min = min_size;
        self
    }

    /// Sets the least and the most a resizable layer may be resized to.
    pub fn size_limits(mut self, limits: SizeLimits) -> Overlay {
        self.size_limits = limits;
        self
    }

    /// Places the layer at `rect` in its parent's space. The container reports the moves and resizes the user makes
    /// as `PlacementChange`s to `Container::placement_changed`; the caller decides the next rect. A placed rect wins
    /// over a saved placement.
    pub fn placed(mut self, rect: Rect) -> Overlay {
        self.placed = Some(rect);
        self
    }

    /// Places the layer's top left at `pos` in its parent's space and sizes it from its content and size limits, so a
    /// layer whose content grows grows with it. Moves and resizes are reported and owned as for
    /// [`placed`](Self::placed); `placed` wins when both are set.
    pub fn placed_at(mut self, pos: Pos2) -> Overlay {
        self.placed_at = Some(pos);
        self
    }

    /// Sets the rect a move or resize keeps the layer inside.
    pub fn bounds(mut self, bounds: Rect) -> Overlay {
        self.bounds = Some(bounds);
        self
    }

    /// Whether a container mounted as this layer scrolls its body vertically (default on).
    pub fn body_scroll(mut self, on: bool) -> Overlay {
        self.body_scroll = on;
        self
    }

    /// Sets the layer's opacity, from 0 (invisible) to 1.
    pub fn opacity(mut self, opacity: f32) -> Overlay {
        self.opacity = opacity;
        self
    }

    /// Sets the narrowest width.
    pub fn min_width(mut self, min_width: MinWidth) -> Overlay {
        self.min_width = min_width;
        self
    }

    /// Sets whether the user may move it.
    pub fn movable(mut self, on: bool) -> Overlay {
        self.movable = on;
        self
    }

    /// Sets whether the move band moves the OS window the layer is shown in, in place of the layer.
    pub fn moves_window(mut self, on: bool) -> Overlay {
        self.moves_window = on;
        self
    }

    /// Sets the focus scope.
    pub fn focus(mut self, focus: FocusPolicy) -> Overlay {
        self.focus = focus;
        self
    }

    /// Sets the dismissal rules.
    pub fn dismiss(mut self, dismiss: Dismiss) -> Overlay {
        self.dismiss = dismiss;
        self
    }

    /// Sets the open and close motion.
    pub fn motion(mut self, motion: MotionSpec) -> Overlay {
        self.motion = motion;
        self
    }

    /// Records an opener. An anchored layer hangs from the rect of the opener that opened it, on the side and
    /// alignment its preset gave; a floating layer keeps its place, and the opener serves focus return and the
    /// trigger. Called again, it adds an opener: the layer opens from any of them, and the opener that opened it
    /// decides the anchor.
    pub fn opened_by(mut self, anchor: &Response, trigger: Trigger) -> Overlay {
        let opener = Opener::new(anchor, trigger);
        match &mut self.opener {
            Some(first) => first.others.push(opener),
            None => self.opener = Some(opener),
        }
        self.anchor(Anchor::Rect(anchor.rect))
    }

    /// Sets where the layer hangs when the opener named last opened it. A rect or point is in that opener's layer
    /// space.
    pub fn opener_anchor(mut self, anchor: Anchor) -> Overlay {
        debug_assert!(self.opener.is_some(), "`opener_anchor` follows `opened_by`");
        if let Some(first) = &mut self.opener {
            match first.others.last_mut() {
                Some(last) => last.anchor = Some(anchor),
                None => first.anchor = Some(anchor),
            }
        }
        self
    }
}

/// What kind of gesture a `PlacementChange` reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlacementKind {
    /// The move band was dragged.
    Move,
    /// A grip was dragged: the edge or corner it sits on.
    Size(ResizeEdge),
    /// The keyboard Move or Size mode (`window.Move`, `window.Size`) changed the rect.
    Keyboard,
}

/// A move or resize of a layer the user made, or of the OS window a viewport mount shows in, reported to
/// `Container::placement_changed`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PlacementChange {
    /// The rect after the change: a layer's in its parent space, kept inside the overlay's `bounds` and `size_limits`;
    /// an OS window's outer rect in desktop points. On `Cancel` the rect from before the gesture.
    pub rect: Rect,
    /// Where the gesture stands.
    pub phase: GesturePhase,
    /// The pointer position of a pointer gesture.
    pub pointer: Option<Pos2>,
    /// What changed the rect.
    pub kind: PlacementKind,
}
