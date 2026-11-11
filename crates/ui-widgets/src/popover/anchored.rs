//! `Popover` (15, 16): a non-modal popup anchored to the control that opened it, holding a body the app builds.

use crate::contract::overlay::{Anchor, Dismiss, OverlayFit};
use crate::contract::{
    Bind, BindMut, Common, Component, Container, ContainerProps, Cx, Mount, NoValue, Outcome, Overlay, Pad, Props as _, Role, SaveKey, Trigger,
};
use crate::runtime::env::Label;
use crate::runtime::focus::{FocusPolicy, Initial};
use crate::runtime::overlay::CloseReason;
use crate::style::{ComponentStyle, StyleCx, SurfaceKind};
use craft_behavior::{CancelRule, Metrics, Sense};
use egui::{Pos2, Response};

/// The style of a popover: the surface and padding come from the container.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PopoverStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What a popover or popup list returns: its response and how it closed in this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct PopoverOutput {
    /// The popover's response.
    pub response: Response,
    /// How the popover closed in this pass; `None` while it stays open. Reported once for each close.
    pub closed: Option<CloseReason>,
    /// A press landed outside the popover, its opener and the popups opened from it while an
    /// [`Outside::Keep`] rule kept it open: the caller decides whether that closes it.
    pub pressed_outside: bool,
}

impl PopoverOutput {
    /// The output of the container `cx` that closed as `closed` in this pass.
    pub(super) fn of<C: Component>(cx: &Cx<'_, C>, closed: Option<CloseReason>) -> Self {
        let pressed_outside = crate::runtime::overlay::pressed_outside(&cx.core.ctx, cx.core.id);
        PopoverOutput { response: cx.response.clone(), closed, pressed_outside }
    }
}

impl Outcome for PopoverOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A non-modal popup that hangs from the control that opened it and holds the app's body (a colour picker, a stroke
/// editor, a settings form).
///
/// - Open state: the caller's binding. Escape, a press outside, the focus leaving and the other close rules write
///   `false`, and the output's `closed` says which one.
/// - Opener: [`opened_by`](Popover::opened_by) names the control it hangs from; a press on it is not outside, and
///   focus returns to it on a close by key.
/// - Focus moves into the popover on open, to [`initial_focus`](Popover::initial_focus) or the first stop.
/// - Popups opened from the body (selects, menus, other popovers) belong to the popover: a press in one is not
///   outside, and a close closes them.
/// - Size: fits the body; a [resizable](Popover::resizable) popover saves the size the user leaves it at under
///   [`remember`](Popover::remember).
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub struct Popover<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    open: Bind<'a, bool>,
    closed: Option<CloseReason>,
}

impl<'a> Popover<'a> {
    /// A popover named `label`, open while `open` is true.
    pub fn new(label: impl Into<Label>, open: Bind<'a, bool>) -> Self {
        let props = ContainerProps::new().surface(SurfaceKind::Popup).pad(Pad::Regular).mount(Mount::Layer(Overlay::popover()));
        Popover { common: Common::new(), props, open, closed: None }.label(label)
    }

    /// Changes the layer the popover shows in, for the settings without a setter of their own: the size a
    /// [resizable](Popover::resizable) popover has until the user resizes it (`Overlay::size`) and the least it may be
    /// resized to (`Overlay::min_size`).
    #[must_use]
    pub fn overlay(mut self, change: impl FnOnce(Overlay) -> Overlay) -> Self {
        let overlay = match std::mem::take(&mut self.props.mount) {
            Mount::Layer(overlay) => overlay,
            _ => Overlay::popover(),
        };
        self.props = self.props.mount(Mount::Layer(change(overlay)));
        self
    }

    /// The control the popover hangs from. The popover closes when the opener is no longer shown.
    #[must_use]
    pub fn opened_by(self, opener: &Response) -> Self {
        self.overlay(|overlay| overlay.opened_by(opener, Trigger::Manual))
    }

    /// Whether edge grips resize the popover. The size the user leaves it at is saved under
    /// [`remember`](Popover::remember).
    #[must_use]
    pub fn resizable(self, on: bool) -> Self {
        self.overlay(|overlay| {
            if on {
                overlay.resizable(true).fit(OverlayFit::Resizable)
            } else if overlay.fit == OverlayFit::Resizable {
                overlay.resizable(false).fit(OverlayFit::Content)
            } else {
                overlay.resizable(false)
            }
        })
    }

    /// Hangs the popover from `at`, a point in the parent's layer space, instead of from an opener.
    #[must_use]
    pub fn at_point(self, at: Pos2) -> Self {
        self.overlay(|overlay| overlay.anchor(Anchor::Point(at)))
    }

    /// Hangs the popover from where the pointer was when it opened, instead of from an opener.
    #[must_use]
    pub fn at_pointer(self) -> Self {
        self.overlay(|overlay| overlay.anchor(Anchor::Pointer))
    }

    /// Replaces the close rules the popover starts with, for callers that close it themselves: `Outside::Keep` with
    /// [`PopoverOutput::pressed_outside`], or no close when the focus leaves.
    #[must_use]
    pub fn dismiss(self, dismiss: Dismiss) -> Self {
        self.overlay(|overlay| overlay.dismiss(dismiss))
    }

    /// Whether Escape, with the focus inside, closes the popover. Off, a popover of a field that handles Escape
    /// itself stays open.
    #[must_use]
    pub fn escape(self, on: bool) -> Self {
        self.overlay(|mut overlay| {
            overlay.focus.cancel = if on { FocusPolicy::POPUP.cancel } else { CancelRule::Ignore };
            overlay
        })
    }

    /// Saves the size under `key`, so a reopened resizable popover has the size the user left it at.
    #[must_use]
    pub fn remember(mut self, key: SaveKey) -> Self {
        self.props = self.props.save(key);
        self
    }

    /// What takes the keyboard focus when the popover opens, such as the hex field of a colour picker.
    #[must_use]
    pub fn initial_focus(self, initial: Initial) -> Self {
        self.overlay(|overlay| {
            let focus = overlay.focus.initial(initial);
            overlay.focus(focus)
        })
    }

    /// Whether Enter, pressed with the focus inside and left untaken by the focused part, closes the popover with
    /// `CloseReason::Enter`: pickers that commit by closing.
    #[must_use]
    pub fn close_on_enter(self, on: bool) -> Self {
        self.overlay(|mut overlay| {
            overlay.dismiss = overlay.dismiss.enter(on);
            overlay
        })
    }
}

impl Component for Popover<'_> {
    const ROLE: Role = Role::DIALOG;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = PopoverStyle;
    type Output = PopoverOutput;

    fn style(_: &StyleCx<'_>, _: PopoverStyleKey) -> PopoverStyle {
        PopoverStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> PopoverOutput {
        PopoverOutput::of(&cx, self.closed)
    }
}

impl Container for Popover<'_> {
    fn open(&mut self) -> Option<BindMut<'_, bool>> {
        Some(self.open.reborrow())
    }

    fn closed(&mut self, why: CloseReason) {
        self.closed.get_or_insert(why);
    }
}

#[cfg(feature = "testing")]
mod stories;
