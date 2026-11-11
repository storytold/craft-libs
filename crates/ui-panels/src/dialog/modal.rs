//! `Dialog` (16.5): `DialogFrame`'s chrome on an egui layer: modal or modeless, movable by its header, with a
//! focus trap, placement and size remembered under a `SaveKey`.

use super::frame::{self, DialogOutput, FOOTER, FooterCx, FooterFn, FooterHooks};
use crate::contract::overlay::{FloatPlace, OverlayFit, Place};
use crate::contract::{
    Bind, BindMut, BodyCx, Common, Component, Container, ContainerProps, Cx, DragBand, HeaderRegion, Mount, Overlay, Pad, Props as _, Role, SaveKey,
    ShowContainer as _,
};
use crate::edit::outer::{self, OuterEnd};
use crate::runtime::access::Access;
use crate::runtime::env::Label;
use crate::runtime::focus::Initial;
use crate::runtime::memory::sub;
use crate::runtime::overlay::{self, Backdrop, CloseReason, Rank};
use crate::runtime::windows::{self, WindowHost};
use crate::sections::{DialogRole, FooterButton};
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{Metrics, Sense};
use craft_host::target::WindowState;
use egui::{Context, Id, Pos2, Rangef, Rect, Response, Ui, ViewportId};

/// The style of a dialog: the surface and padding come from the container.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct DialogStyle {
    #[style(metrics)]
    metrics: Metrics,
}

type GuardFn<'a> = Box<dyn FnMut(&FooterButton, &mut BodyCx<'_>) -> bool + 'a>;

type OnPickFn<'a> = Box<dyn FnMut(&FooterButton, &mut BodyCx<'_>) + 'a>;

type SettleFn<'a> = Box<dyn FnMut(&mut BodyCx<'_>, &mut [FooterButton]) + 'a>;

/// A dialog on its own layer. It opens in the middle of the window, moves by its title band, scrolls its body when the
/// window is smaller and traps the keyboard focus while it is open.
///
/// - Open state: bound with [`opened`](Dialog::opened) to the app's value or the host's entry, else kept in a
///   session slot that starts open. The close action, Escape and `BodyCx::request_close` write `false`.
/// - Placement and size: saved across runs under [`remember`](Dialog::remember); a resizable dialog also saves the
///   size the user left it at.
/// - A modal dialog blocks input below it in every window; a press outside does nothing.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub struct Dialog<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    footer: Option<FooterFn<'a>>,
    /// Lets the host refuse the close of a picked button: `allows` gets every picked button but `Cancel`
    /// and the dialog's body context, so a host reads its kept state, and returns whether a button that closes the
    /// dialog may. It is not the place to act on a pick (see `on_pick`). On `false` the dialog stays open, with `DialogOutput::picked` still reporting the pick; the host shows why. A button marked `stays_open` never closes, whatever it returns.
    #[prop(boxed = "FnMut(&FooterButton, &mut BodyCx<'_>) -> bool + 'a", name = "close_guard")]
    guard: Option<GuardFn<'a>>,
    /// Sets the buttons' state each pass: `settle` gets the dialog's body context and the buttons when the footer is
    /// shown, after the body ran, so a button's `enabled` can follow what the body wrote in the same pass (through
    /// `BodyCx::memory`, say). Enter's routing is decided from the buttons given to `buttons`.
    #[prop(boxed = "FnMut(&mut BodyCx<'_>, &mut [FooterButton]) + 'a", name = "settle_buttons")]
    settle: Option<SettleFn<'a>>,
    /// Acts on a picked footer button in the footer step, before the close is decided: `act` gets the button and the
    /// dialog's body context. `ResetFooter::install` sets it.
    #[prop(boxed = "FnMut(&FooterButton, &mut BodyCx<'_>) + 'a", name = "on_pick")]
    on_pick: Option<OnPickFn<'a>>,
    buttons: Vec<FooterButton>,
    /// Whether Enter closes the dialog as its OK.
    enter: bool,
    /// Whether the title row has a close action.
    #[prop]
    closable: bool,
    closed: Option<CloseReason>,
    picked: Option<usize>,
    /// The body ran in this pass: the header region marks it.
    body_shown: bool,
    alert: bool,
}

impl<'a> Dialog<'a> {
    /// A modal dialog titled `title`, with a close action; it moves by its title band and is not resizable until
    /// `resizable` says so.
    pub fn new(title: impl Into<Label>) -> Self {
        let overlay = Overlay::dialog().resizable(false);
        Dialog::on(title, overlay, HeaderRegion::new().band(DragBand::Move).rule(true))
    }

    /// A prompt titled `title`: a dialog on the modal preset, centred with its content capped, fixed in place and
    /// not remembered.
    pub fn prompt(title: impl Into<Label>) -> Self {
        Dialog::on(title, Overlay::modal(), HeaderRegion::new().rule(true))
    }

    fn on(title: impl Into<Label>, overlay: Overlay, header: HeaderRegion) -> Self {
        // The dialog is an instance of the application scope, so its edit commands reach it while a modal layer blocks the
        // application's own.
        let props = frame::chrome_props().mount(Mount::Layer(overlay)).header(header).command_scope(crate::runtime::commands::CommandScope::APP);
        Dialog {
            common: Common::new(),
            props,
            open: None,
            footer: None,
            guard: None,
            on_pick: None,
            settle: None,
            buttons: Vec::new(),
            enter: false,
            closable: true,
            closed: None,
            picked: None,
            body_shown: false,
            alert: false,
        }
        .label(title)
    }

    /// Changes the layer the dialog shows in, for the settings without a setter of their own: the size a resizable
    /// dialog has until the user resizes it (`Overlay::size`) and the least it may be resized to (`Overlay::min_size`).
    #[must_use]
    pub fn overlay(mut self, change: impl FnOnce(Overlay) -> Overlay) -> Self {
        self.props = match std::mem::take(&mut self.props.mount) {
            Mount::Layer(overlay) => self.props.mount(Mount::Layer(change(overlay))),
            Mount::Viewport { fallback, spec } => self.props.mount(Mount::Viewport { fallback: change(fallback), spec }),
            _ => self.props.mount(Mount::Layer(change(Overlay::dialog().resizable(false)))),
        };
        self
    }

    /// Shows the dialog in its own OS window, with the layer it has so far as the fallback where windows are
    /// unavailable. Call it after the settings that shape that layer, or before: later settings shape the fallback.
    #[must_use]
    pub fn viewport(self) -> Self {
        let Mount::Layer(overlay) = self.props.mount.clone() else { return self };
        let mount = Mount::viewport(overlay);
        Dialog { props: self.props.mount(mount), ..self }
    }

    /// A tool window: no modal layer and no backdrop; its edits are the app's normal edits.
    #[must_use]
    pub fn modeless(self) -> Self {
        self.overlay(Overlay::modeless)
    }

    /// What is drawn behind a modal dialog.
    #[must_use]
    pub fn backdrop(self, backdrop: Backdrop) -> Self {
        self.overlay(|mut overlay| {
            if overlay.rank.is_modal() {
                overlay.rank = Rank::modal(backdrop);
            }
            overlay
        })
    }

    /// Whether edge grips resize the dialog, and the size the user leaves it at is saved.
    #[must_use]
    pub fn resizable(self, on: bool) -> Self {
        self.overlay(|overlay| if on { overlay.resizable(true).fit(OverlayFit::Resizable) } else { overlay.resizable(false) })
    }

    /// What takes the keyboard focus when the dialog opens.
    #[must_use]
    pub fn initial_focus(self, initial: Initial) -> Self {
        self.overlay(|overlay| {
            let focus = overlay.focus.initial(initial);
            overlay.focus(focus)
        })
    }

    /// Where the dialog first opens, as a screen position of its top-left corner; the middle of the window without
    /// it. A saved placement comes first.
    #[must_use]
    pub fn first_place(self, at: Pos2) -> Self {
        self.overlay(|overlay| overlay.place(Place::Float(FloatPlace::at(at))))
    }

    /// Saves the placement and size under `key`, so a reopened dialog sits where the user left it.
    #[must_use]
    pub fn remember(mut self, key: SaveKey) -> Self {
        self.props = self.props.save(key);
        self
    }

    /// Whether Enter, pressed with the focus inside and left untaken by the focused part, is the OK of the dialog.
    /// With an enabled `Accept` button in the footer, Enter activates that button, so a click and Enter take the same
    /// way (`close_guard` included) and `DialogOutput::picked` says so. Without one, Enter closes the dialog with
    /// `CloseReason::Enter`.
    #[must_use]
    pub fn close_on_enter(mut self, on: bool) -> Self {
        self.enter = on;
        self.route_enter()
    }

    /// Sets the overlay's own Enter dismissal for the dialogs whose footer has no default button to take it.
    fn route_enter(self) -> Self {
        let dismiss = self.enter && !self.buttons.iter().any(|button| button.role() == DialogRole::Accept && button.is_enabled());
        self.overlay(|mut overlay| {
            overlay.dismiss = overlay.dismiss.enter(dismiss);
            overlay
        })
    }

    /// Whether the dialog holds one outer gesture over its body's fields, from open to close: the app's `GestureHost`
    /// keeps one undo step for the dialog, named by its title. OK commits it; Cancel, Escape, the close action and the
    /// app closing the dialog take its edits back. The host folds every gesture it handles while the dialog is open
    /// into the step, so it takes effect for a modal dialog only; a modeless dialog holds none.
    #[must_use]
    pub fn outer_gesture(mut self, on: bool) -> Self {
        self.props = self.props.outer_edit(on);
        self
    }

    /// The larger frame margin.
    #[must_use]
    pub fn roomy(mut self, on: bool) -> Self {
        self.props = self.props.pad(if on { crate::contract::Pad::Roomy } else { crate::contract::Pad::Regular });
        self
    }

    /// The dialog reports the alert-dialog role instead of the dialog role.
    #[must_use]
    pub(super) fn alert(mut self) -> Self {
        self.alert = true;
        self
    }

    /// The dialog buttons in the footer, in the platform's order for their roles. A click closes the dialog: `Cancel`
    /// as `CloseReason::Requested`, every other role as `CloseReason::Activated` unless the button `stays_open`;
    /// `DialogOutput::picked` says which.
    #[must_use]
    pub fn buttons(mut self, buttons: impl IntoIterator<Item = FooterButton>) -> Self {
        self.props = self.props.footer(FOOTER);
        self.buttons = buttons.into_iter().collect();
        self.route_enter()
    }

    /// The footer's content: the closure fills a footer bar above the buttons, or alone.
    #[must_use]
    pub fn footer(mut self, content: impl FnMut(&mut Ui, &mut FooterCx<'_, '_>) + 'a) -> Self {
        self.props = self.props.footer(FOOTER);
        self.footer = Some(Box::new(content));
        self
    }
}

impl Dialog<'_> {
    /// Whether the outer gesture runs: it was asked for and the dialog is not a modeless layer, whose open time would
    /// fold edits made elsewhere into the step.
    fn holds_outer_gesture(&self) -> bool {
        self.props.outer_edit && !matches!(&self.props.mount, Mount::Layer(overlay) if !overlay.rank.is_modal())
    }

    /// Keeps the window draggable under a modal dialog: while the dialog is the top layer, each drag area of its
    /// window that the modal covers, less the caption buttons and the dialog itself, gets a [`ModalDragStrip`]. The
    /// dialog is the layer whose body runs, and its rect is the one the layer stack holds.
    fn keep_window_draggable(&self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        // A dialog in its own OS window covers no title bar of the window below.
        let covers_window = matches!(&self.props.mount, Mount::Layer(overlay) if overlay.rank.is_modal());
        let Some(dialog) = overlay::drawing_parent(&ctx).filter(|dialog| covers_window && on_top(&ctx, *dialog)) else { return };
        let Some(entry) = overlay::entry(&ctx, dialog) else { return };
        for (at, strip) in covered_strips(&ctx, WindowHost::of(&ctx).current(), entry.rect).into_iter().enumerate() {
            ModalDragStrip::new(strip).id(sub(dialog, ("drag_strip", at))).show(ui, |_, _| {});
        }
    }
}

/// Whether the dialog `id` is the layer that takes the keys: no popup of the dialog and no other modal is open above
/// it. Its modal chrome takes no keys, so the strips never hide the dialog from this test.
fn on_top(ctx: &Context, id: Id) -> bool {
    overlay::top(ctx) == Some(id)
}

/// The strips of `viewport`'s window that stay draggable under a modal whose dialog lies at `dialog`: each drag area
/// of the window's last complete pass that one of its own covers hides whole (the modal's cover), less the caption
/// buttons and the dialog, so a dialog moved into the title bar stays in front. A drag area that lies inside another
/// adds no strip. The strips themselves are drag areas no hit covers, so they never count.
fn covered_strips(ctx: &Context, viewport: ViewportId, dialog: Rect) -> Vec<Rect> {
    let Some(regions) = WindowHost::of(ctx).caption_regions(viewport) else { return Vec::new() };
    let covered: Vec<Rect> = regions
        .moves
        .iter()
        .zip(&regions.covers)
        .filter(|(area, covers)| covers.iter().any(|cover| cover.contains_rect(**area)))
        .map(|(area, _)| *area)
        .collect();
    let inside_another = |at: usize, area: Rect| {
        covered.iter().enumerate().any(|(other, wider)| other != at && wider.contains_rect(area) && (!area.contains_rect(*wider) || other < at))
    };
    let kept: Vec<Rect> = regions.buttons.iter().map(|(_, rect)| *rect).chain([dialog]).collect();
    covered.iter().enumerate().filter(|(at, area)| !inside_another(*at, **area)).flat_map(|(_, area)| free_spans(*area, &kept)).collect()
}

/// The parts of the horizontal strip `area` that lie beside every rect of `kept` that meets it.
fn free_spans(area: Rect, kept: &[Rect]) -> Vec<Rect> {
    let mut cuts: Vec<Rangef> = kept
        .iter()
        .filter(|rect| rect.intersects(area))
        .map(|rect| Rangef::new(rect.left().max(area.left()), rect.right().min(area.right())))
        .collect();
    cuts.sort_by(|a, b| a.min.total_cmp(&b.min));
    let mut spans = Vec::new();
    let mut from = area.left();
    for cut in cuts {
        if cut.min > from {
            spans.push(Rect::from_x_y_ranges(Rangef::new(from, cut.min), area.y_range()));
        }
        from = from.max(cut.max);
    }
    if area.right() > from {
        spans.push(Rect::from_x_y_ranges(Rangef::new(from, area.right()), area.y_range()));
    }
    spans
}

/// The style of a modal drag strip: it paints nothing.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(crate) struct ModalDragStripStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The chrome a modal dialog puts over a drag area of the window it covers, so that the window still moves by it.
/// It sits at the strip on the modal-chrome layer (`Overlay::modal_chrome`), paints nothing, takes no keys and is no
/// Tab stop: a drag moves the window, and a double click maximizes or restores it as the title bar does.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub(crate) struct ModalDragStrip {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    rect: Rect,
}

impl ModalDragStrip {
    /// A strip over `rect`, in the coordinates of the window's layers.
    pub(crate) fn new(rect: Rect) -> Self {
        let props =
            ContainerProps::new().pad(Pad::None).mount(Mount::Layer(Overlay::modal_chrome(rect))).header(HeaderRegion::new().band(DragBand::Move));
        ModalDragStrip { common: Common::new(), props, rect }
    }
}

impl Component for ModalDragStrip {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = ModalDragStripStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: ModalDragStripStyleKey) -> ModalDragStripStyle {
        ModalDragStripStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for ModalDragStrip {
    /// The header is the strip, so the move band covers it. A double click on it acts on the window as one on the
    /// title bar under it does.
    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        if cx.band_double_pressed() {
            let host = WindowHost::of(ui.ctx());
            let viewport = host.current();
            let shell = windows::shell_of(ui.ctx(), viewport);
            if let Some(action) = windows::double_click_action(&host.frame(), shell.double_click, shell.window.state == WindowState::Maximized) {
                host.act(viewport, action);
            }
        }
        ui.set_min_size(self.rect.size());
    }
}

impl Component for Dialog<'_> {
    const ROLE: Role = Role::DIALOG;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const ALT_ROLES: &'static [Role] = &[Role::ALERT_DIALOG];
    type Style = DialogStyle;
    type Output = DialogOutput;

    fn style(_: &StyleCx<'_>, _: DialogStyleKey) -> DialogStyle {
        DialogStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        if self.alert {
            access.role(Role::ALERT_DIALOG);
        }
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> DialogOutput {
        let output = DialogOutput { response: cx.response.clone(), closed: self.closed, picked: self.picked };
        if self.holds_outer_gesture() {
            // The dialog's answer decides the end: an OK commits, every other close cancels.
            let end = output.result(|| OuterEnd::Commit).map(|answer| answer.ok().unwrap_or(OuterEnd::Cancel));
            let core = &mut cx.core;
            outer::settle(&core.ctx, core.memory, core.id, self.body_shown, end);
        }
        output
    }
}

impl Container for Dialog<'_> {
    fn open(&mut self) -> Option<BindMut<'_, bool>> {
        self.open.as_mut().map(Bind::reborrow)
    }

    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        self.body_shown = true;
        if self.holds_outer_gesture() {
            outer::keep(ui.ctx(), cx);
        }
        frame::header(ui, cx, frame::close_action(self.closable));
        self.keep_window_draggable(ui);
    }

    fn footer(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        frame::footer(
            ui,
            cx,
            &mut self.footer,
            &self.buttons,
            self.enter,
            frame::FooterHost { picked: &mut self.picked, guard: &mut self.guard, on_pick: &mut self.on_pick, settle: &mut self.settle },
        );
    }

    fn closed(&mut self, why: CloseReason) {
        self.closed.get_or_insert(why);
    }
}

impl<'a> FooterHooks<'a> for Dialog<'a> {
    fn buttons(self, buttons: impl IntoIterator<Item = FooterButton>) -> Self {
        Dialog::buttons(self, buttons)
    }

    fn settle_buttons<T: FnMut(&mut BodyCx<'_>, &mut [FooterButton]) + 'a>(self, settle: T) -> Self {
        Dialog::settle_buttons(self, settle)
    }

    fn on_pick<T: FnMut(&FooterButton, &mut BodyCx<'_>) + 'a>(self, act: T) -> Self {
        Dialog::on_pick(self, act)
    }

    fn close_guard<T: FnMut(&FooterButton, &mut BodyCx<'_>) -> bool + 'a>(self, allows: T) -> Self {
        Dialog::close_guard(self, allows)
    }
}
