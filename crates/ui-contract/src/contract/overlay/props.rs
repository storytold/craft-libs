//! The `Overlay` data, its presets and builders, its dismissal rules and the placement changes it reports.

use super::opener::{Opener, Trigger};
use super::place::{Anchor, FloatPlace, OverlayFit, Place, Resize, ResizeEdges, SizeLimits, ViewportShare};
use crate::runtime::access::PopupKind;
use crate::runtime::focus::{FocusPolicy, Initial};
use crate::runtime::motion::{MotionSpec, spec};
use crate::runtime::overlay::{Backdrop, LayerRole, Rank};
use crate::runtime::windows::ResizeEdge;
use craft_behavior::overlay::{Align, MinWidth, Side};
use craft_behavior::{GesturePhase, OwnerClass};
use egui::emath::TSTransform;
use egui::{Align2, Id, Pos2, Rect, Response, Vec2};
use std::borrow::Cow;

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
    /// A command a key, a native menu pick or the palette ran, from outside the layer and the layers opened from it.
    pub commands: bool,
    /// Seconds after opening.
    pub timeout: Option<f32>,
}

impl Dismiss {
    /// An outside press or a command closes; nothing else does.
    pub const fn new() -> Self {
        Dismiss { outside: Outside::Close, enter: false, focus_left: false, on_activate: false, anchor_gone: true, commands: true, timeout: None }
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

    /// Closes when a key, a native menu pick or the palette runs a command from outside it; off for a layer its host
    /// opens and closes, as a dock panel's flyout.
    pub const fn commands(mut self, on: bool) -> Self {
        self.commands = on;
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
#[derive(Clone, Debug, PartialEq)]
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
    /// The rect the layer is kept inside: where it is placed, hung or floated, and through the user's moves and
    /// resizes. In the layer's parent space.
    pub bounds: Option<Rect>,
    /// The layer's own opacity, below the open and close motion's.
    pub opacity: f32,
    /// Whether a container mounted as this layer scrolls its body vertically. Off for a layer whose body fills it and
    /// manages its own scrolling (a dock tree); a body that is one collection uses `collection_body` instead.
    pub body_scroll: bool,
    /// Whether the layer's move band moves the OS window the layer is shown in, in place of the layer: the chrome a
    /// modal puts over the part of a title bar it covers. The layer stays where it is placed.
    pub moves_window: bool,
    /// Whether the app moves the OS window a viewport mount shows in from the move band, never the operating system's
    /// move loop: a press on the band reaches the container, which reports each step of the move (a dock window's
    /// title row, whose move docks the window). The window is opened knowing it, so its hit test never hands the band
    /// to the system. A layer is always moved by the app.
    pub moved_by_app: bool,
    /// A request of the pass for a movable layer: its move band takes over the primary press that is held, with the
    /// pointer at this point inside the layer, so the layer follows a press that began elsewhere (a group torn off a
    /// tool rail's flyout). With no press held it does nothing.
    // PLAN: the sheet's Files table has no row for this file; `FloatingGroup::continue_move` (ch.K16) reaches the move
    // band only through the layer it mounts, as sheet note 6 says ("sets that state through the overlay").
    pub continue_move: Option<Vec2>,
    /// What the layer is for; a progress window is left out of `any_open`.
    pub role: LayerRole,
    /// Set only by [`Overlay::full_window`].
    full_window: bool,
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
            moved_by_app: false,
            continue_move: None,
            role: LayerRole::default(),
            full_window: false,
        }
    }

    /// The overlay with what its caller gives in the parent's layer space (`placed`, `placed_at`, a float position
    /// and `bounds`) in screen space, by the parent layer's map `to_screen`: where the layer host places the layer and
    /// its body moves and resizes it.
    pub(crate) fn in_screen(&self, to_screen: TSTransform) -> Cow<'_, Overlay> {
        if to_screen == TSTransform::IDENTITY {
            return Cow::Borrowed(self);
        }
        let mut mapped = self.clone();
        mapped.placed = mapped.placed.map(|rect| to_screen * rect);
        mapped.placed_at = mapped.placed_at.map(|pos| to_screen * pos);
        mapped.bounds = mapped.bounds.map(|rect| to_screen * rect);
        match &mut mapped.place {
            Place::Float(FloatPlace { pos: Some(pos), .. }) => *pos = to_screen * *pos,
            // An offset is a vector: it scales with the layer's space and does not translate.
            Place::Anchored { offset, clear, .. } => {
                *offset = to_screen.scaling * *offset;
                *clear = clear.map(|rect| to_screen * rect);
            }
            _ => {}
        }
        Cow::Owned(mapped)
    }

    fn below() -> Place {
        Place::anchored(Anchor::Response, Side::Below, Align::Start)
    }

    /// A popover: a popup anchored below, from its content, at least as wide as its anchor and capped by the window,
    /// taking focus in. It closes when the focus moves out of it.
    pub fn popover() -> Overlay {
        let mut overlay = Overlay::base(Rank::Popup, Overlay::below(), OverlayFit::Content, OwnerClass::Popup, FocusPolicy::POPUP);
        overlay.dismiss = Dismiss::new().focus_left(true);
        overlay.min_width = MinWidth::Anchor;
        overlay
    }

    /// A dropdown list: from its content, at least as wide as its anchor and capped, focus stays on the anchor. It
    /// closes when a row is activated or the focus leaves the anchor. The opener's list keys go to the collection
    /// [`Overlay::list`] names.
    pub fn dropdown() -> Overlay {
        let focus = FocusPolicy::POPUP.trap(false).restore(false).initial(Initial::KeepAnchor);
        let mut overlay = Overlay::base(Rank::Popup, Overlay::below(), OverlayFit::ContentCapped(DROPDOWN_SHARE), OwnerClass::PassivePopup, focus);
        overlay.dismiss = Dismiss::new().on_activate(true).focus_left(true);
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

    /// A submenu: beside its parent entry, opening upward from a low entry when it fits there and not below.
    pub fn submenu() -> Overlay {
        Overlay::menu().place(Place::anchored(Anchor::Response, Side::End, Align::Start).flipping_across())
    }

    /// A tooltip: below the control, no focus, never takes input.
    pub fn tooltip() -> Overlay {
        let mut overlay =
            Overlay::base(Rank::Tooltip, Overlay::below(), OverlayFit::ContentCapped(NOTE_SHARE), OwnerClass::PassivePopup, FocusPolicy::NONE);
        overlay.dismiss = Dismiss::new().outside(Outside::Keep);
        overlay
    }

    /// A dialog: modal with a dimmed backdrop, floating, movable and resizable.
    /// `.modeless()` gives rank `Window`.
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

    /// A modal over the whole window: a modal rank with no backdrop, from its content, with `focus` as its scope.
    pub fn full_window(focus: FocusPolicy) -> Overlay {
        let mut overlay = Overlay::modal().fit(OverlayFit::Content).focus(focus);
        overlay.rank = Rank::modal(Backdrop::None);
        overlay.full_window = true;
        overlay
    }

    /// Whether the layer was made by [`Overlay::full_window`]; the rank and backdrop are not read.
    pub fn is_full_window(&self) -> bool {
        self.full_window
    }

    /// A modal: centred, content capped.
    pub fn modal() -> Overlay {
        let mut overlay = Overlay::dialog();
        overlay.rank = Rank::modal(Backdrop::None);
        overlay.movable = false;
        overlay.resize = Resize::NONE;
        overlay
    }

    /// The chrome of a modal over a part of the window the modal covers, placed at `rect`: above the modal, with no
    /// focus policy and no dismissal of its own. Its move band moves the window the layer is shown in.
    pub fn modal_chrome(rect: Rect) -> Overlay {
        let mut overlay =
            Overlay::base(Rank::ModalChrome, Place::Float(FloatPlace::centred()), OverlayFit::Content, OwnerClass::Widget, FocusPolicy::NONE);
        overlay.dismiss = Dismiss::new().outside(Outside::Keep);
        overlay.placed = Some(rect);
        overlay.body_scroll = false;
        overlay.moves_window = true;
        overlay
    }

    /// A toast stack at `corner` of `body`, a rect in the opener's layer space: the end-bottom corner of a window's body.
    /// Tab reaches its cards, and focus returns from them to where it came from when the last one ends.
    pub fn toast(body: Rect, corner: Align2) -> Overlay {
        let align = if corner == Align2::CENTER_BOTTOM { Align::Center } else { Align::End };
        let place = Place::anchored(Anchor::Corner(body, corner), Side::Above, align);
        let mut overlay = Overlay::base(Rank::Toast, place, OverlayFit::ContentCapped(NOTE_SHARE), OwnerClass::Widget, FocusPolicy::TAB_LAYER);
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

    /// Grips on the given edges only; a placement that holds an edge keeps its grip away.
    pub fn resize_edges(mut self, edges: ResizeEdges) -> Overlay {
        self.resize = Resize::edges(edges);
        self
    }

    /// The caller sizes the layer or OS window from the resizes its grips report ([`Resize::by_caller`]).
    pub fn sized_by_caller(mut self) -> Overlay {
        self.resize.by_caller = true;
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

    /// Sets the rect the layer is kept inside: where it is placed, hung or floated, and through the user's moves and
    /// resizes.
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

    /// The narrowest the layer may be beside an anchor `anchor_width` wide, on the side its place prefers (below for
    /// a layer that is not anchored).
    pub(crate) fn least_width(&self, anchor_width: f32) -> f32 {
        let side = match &self.place {
            Place::Anchored { side, .. } => *side,
            _ => Side::Below,
        };
        self.min_width.points(anchor_width, side)
    }

    /// Sets whether the user may move it.
    pub fn movable(mut self, on: bool) -> Overlay {
        self.movable = on;
        self
    }

    /// Sets what the layer is for ([`Overlay::role`]).
    pub fn role(mut self, role: LayerRole) -> Overlay {
        self.role = role;
        self
    }

    /// Sets whether the move band moves the OS window the layer is shown in, in place of the layer.
    pub fn moves_window(mut self, on: bool) -> Overlay {
        self.moves_window = on;
        self
    }

    /// The app moves the OS window from the move band ([`Overlay::moved_by_app`]).
    pub fn moved_by_app(mut self) -> Overlay {
        self.moved_by_app = true;
        self
    }

    /// The move band takes over the held primary press in this pass, with the pointer at `grab` inside the layer
    /// ([`Overlay::continue_move`]).
    pub fn continue_move(mut self, grab: Vec2) -> Overlay {
        self.continue_move = Some(grab);
        self
    }

    /// Sets the focus scope.
    pub fn focus(mut self, focus: FocusPolicy) -> Overlay {
        self.focus = focus;
        self
    }

    /// The collection in a dropdown that takes its opener's list keys (`FocusPolicy::list`). A dropdown that holds a
    /// list names it each pass it is shown; the opener keeps the keys while the named list is not shown, and in a
    /// dropdown that names none.
    pub fn list(mut self, list: Id) -> Overlay {
        self.focus = self.focus.list(list);
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
    /// The move continues a press that went down outside the overlay and was handed to its move band
    /// ([`Overlay::continue_move`]): a drag the host began elsewhere (a panel's tab) that the layer carries on.
    pub continued: bool,
}
