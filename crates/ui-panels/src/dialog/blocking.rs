//! `BlockingOverlay` (16.5): a modal layer with no title, for work the window must not interrupt.

use super::frame::{self, DialogOutput, FOOTER};
use crate::contract::{
    Bind, BindMut, BodyCx, Common, Component, Container, ContainerProps, Cx, Mount, Overlay, Pad, Pointer as _, Props as _, Role, SaveKey,
};
use crate::runtime::env::Label;
use crate::runtime::focus::{FocusPolicy, Initial};
use crate::runtime::overlay::{Backdrop, CloseReason, Rank};
use crate::sections::{DialogRole, FooterButton};
use crate::style::{ComponentStyle, StyleCx, SurfaceKind};
use craft_behavior::{CancelRule, CursorIcon, CursorPriority, Metrics, Sense};
use egui::Ui;

/// The style of a blocking overlay: the card's surface and padding come from the container.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct BlockingOverlayStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// A modal layer that takes the window's input while it is shown, with the wait cursor. Its label is the accessible
/// name. Without a card the layer has no look at all; with one, the body sits on the dialog surface, centred.
///
/// - Open state: bound with [`opened`](BlockingOverlay::opened), else a session slot that starts open. Show it
///   every pass the work lasts, or bind the state to the work.
/// - Cancel: Escape closes it when [`cancellable`](BlockingOverlay::cancellable) says so, and a footer button
///   named by [`cancel`](BlockingOverlay::cancel) closes it as `CloseReason::Requested`; the open state then reads
///   `false`.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub struct BlockingOverlay<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    /// The cursor while the overlay is shown, over the whole window.
    #[prop]
    cursor: CursorIcon,
    cancel: Option<Label>,
    closed: Option<CloseReason>,
    picked: Option<usize>,
}

impl<'a> BlockingOverlay<'a> {
    /// An overlay named `name`, with the wait cursor, no backdrop, no card and no cancel.
    pub fn new(name: impl Into<Label>) -> Self {
        let mut overlay = Overlay::modal();
        overlay.rank = Rank::modal(Backdrop::None);
        overlay.focus = FocusPolicy::DIALOG.initial(Initial::Keep).cancel(CancelRule::Ignore);
        let props = ContainerProps::new().mount(Mount::Layer(overlay));
        BlockingOverlay { common: Common::new(), props, open: None, cursor: CursorIcon::Wait, cancel: None, closed: None, picked: None }.label(name)
    }

    fn with_overlay(mut self, change: impl FnOnce(&mut Overlay)) -> Self {
        if let Mount::Layer(overlay) = &mut self.props.mount {
            change(overlay);
        }
        self
    }

    /// Dims the window behind the overlay.
    #[must_use]
    pub fn scrim(self, on: bool) -> Self {
        self.with_overlay(|overlay| overlay.rank = Rank::modal(if on { Backdrop::Scrim } else { Backdrop::None }))
    }

    /// A card on the dialog surface around the body.
    #[must_use]
    pub fn card(mut self, on: bool) -> Self {
        self.props.surface = on.then_some(SurfaceKind::Dialog);
        self.props.pad = if on { Pad::Roomy } else { Pad::None };
        self
    }

    /// Whether Escape closes the overlay.
    #[must_use]
    pub fn cancellable(self, on: bool) -> Self {
        self.with_overlay(|overlay| overlay.focus = overlay.focus.cancel(if on { CancelRule::Close } else { CancelRule::Ignore }))
    }

    /// A footer with a Cancel button named `label`; a click closes the overlay as `CloseReason::Requested`.
    #[must_use]
    pub fn cancel(mut self, label: impl Into<Label>) -> Self {
        self.props = self.props.footer(FOOTER);
        self.cancel = Some(label.into());
        self
    }

    /// Saves the open state under `key`.
    #[must_use]
    pub fn remember(mut self, key: SaveKey) -> Self {
        self.props = self.props.save(key);
        self
    }
}

impl Component for BlockingOverlay<'_> {
    const ROLE: Role = Role::DIALOG;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = BlockingOverlayStyle;
    type Output = DialogOutput;

    fn style(_: &StyleCx<'_>, _: BlockingOverlayStyleKey) -> BlockingOverlayStyle {
        BlockingOverlayStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        // The overlay blocks every window it covers, so its cursor holds wherever the pointer is: it outranks the
        // hover cursors of what lies below.
        cx.cursor(self.cursor, CursorPriority::Part);
    }

    fn finish(self, cx: Cx<'_, Self>) -> DialogOutput {
        DialogOutput { response: cx.response.clone(), closed: self.closed, picked: self.picked }
    }
}

impl Container for BlockingOverlay<'_> {
    fn open(&mut self) -> Option<BindMut<'_, bool>> {
        self.open.as_mut().map(Bind::reborrow)
    }

    fn footer(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        let buttons: Vec<FooterButton> = self.cancel.iter().map(|label| FooterButton::new(label.clone(), DialogRole::Cancel)).collect();
        frame::footer(
            ui,
            cx,
            &mut None,
            &buttons,
            false,
            frame::FooterHost { picked: &mut self.picked, guard: &mut None, on_pick: &mut None, settle: &mut None },
        );
    }

    fn closed(&mut self, why: CloseReason) {
        self.closed.get_or_insert(why);
    }
}
