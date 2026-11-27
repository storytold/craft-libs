//! `FullWindowDialog` (16.5): a dialog over the whole window, on the panel surface, with the dialog focus trap.

use super::full_window_parts::SideFooter;
use crate::canvas::ViewCommands;
use crate::contract::{
    Bind, BindMut, BodyCx, Common, Component, Container, ContainerProps, Cx, EndRegion, FooterPlace, FooterRegion, HeaderRegion, Mount, Overlay, Pad,
    Props as _, Role, SaveKey,
};
use crate::dialog::frame::{self, DialogOutput, FOOTER, FooterCx, FooterFn, FooterHooks};
use crate::edit::outer::{self, OuterEnd};
use crate::runtime::commands::{self, CommandScope, CommandSet as _};
use crate::runtime::env::Label;
use crate::runtime::focus::{EnterRule, FocusPolicy, Initial};
use crate::runtime::overlay::CloseReason;
use crate::runtime::windows::{WindowHost, WindowSpec};
use crate::sections::FooterButton;
use crate::style::{ComponentStyle, StyleCx, SurfaceKind};
use craft_behavior::{Metrics, Sense};
use egui::{Ui, UiBuilder, Vec2, vec2};

pub use hosted::{FullWindowWorkspace, HostedWorkspace, WorkspaceCx, WorkspacePass};

mod hosted;
#[cfg(feature = "suite")]
pub(crate) mod stories;
#[cfg(feature = "suite")]
pub(crate) mod trials;

/// The least a full-window dialog's own window may be, in points: the OS window's minimum, and the floor a restored
/// placement is fitted to. A placement saved while photocraft#16595 shrank the window could be 1 pt wide; without
/// a floor it opened that small and nothing grew it again.
pub const OWN_WINDOW_MIN_SIZE: Vec2 = vec2(640.0, 480.0);

/// The style of a full-window dialog: the surface comes from the container.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FullWindowDialogStyle {
    #[style(metrics)]
    metrics: Metrics,
}

type GuardFn<'a> = Box<dyn FnMut(&FooterButton, &mut BodyCx<'_>) -> bool + 'a>;

type OnPickFn<'a> = Box<dyn FnMut(&FooterButton, &mut BodyCx<'_>) + 'a>;

type SettleFn<'a> = Box<dyn FnMut(&mut BodyCx<'_>, &mut [FooterButton]) + 'a>;

/// A dialog that fills the window: a title row, a body that takes the room between the header and the footer, and an
/// optional footer. The areas the body shows inside it are panels at rects.
///
/// - Side columns: [`sides`](FullWindowDialog::sides) fills a region at the end edge, as high as the dialog under the
///   title row and as wide as its content; the body (the canvas) takes the rest of the width. With side columns the
///   footer sits among them, at their bottom unless [`side_footer`](FullWindowDialog::side_footer) says otherwise.
/// - Buttons: the dialog's own, wherever the footer sits: Cancel and Escape close it as `CloseReason::Requested`,
///   Enter activates the first enabled `Accept` button while the dialog's Enter rule is `EnterRule::Submit`, and
///   `close_guard` and `settle_buttons` see every pick.
/// - Open state: bound with [`opened`](FullWindowDialog::opened), else a session slot that starts open.
/// - Hosting: a workspace the app's `DialogHost` owns implements [`FullWindowWorkspace`] and opens as a
///   [`HostedWorkspace`], which binds the open state to its own field.
/// - View commands: the dialog is an instance of the [`ViewCommands`] scope, so the zoom chords act on the view the
///   dialog's zoom bar is bound to, not on a canvas behind it.
/// - Focus: the dialog trap, with nothing focused at first unless [`initial_focus`](FullWindowDialog::initial_focus)
///   says otherwise.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub struct FullWindowDialog<'a> {
    #[common]
    common: Common,
    #[container(surface, live, end, position, collection_body)]
    props: ContainerProps,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    footer: Option<FooterFn<'a>>,
    /// Lets the host act on a picked button and refuse its close: `allows` gets every picked button but `Cancel`
    /// and the dialog's body context, so a host reads and writes its kept state, and returns whether a button that
    /// closes the dialog may. On `false` the dialog stays open, with `DialogOutput::picked` still
    /// reporting the pick; the host shows why. A button marked `stays_open` never closes, whatever it returns.
    #[prop(boxed = "FnMut(&FooterButton, &mut BodyCx<'_>) -> bool + 'a", name = "close_guard")]
    guard: Option<GuardFn<'a>>,
    /// Sets the buttons' state each pass; see `Dialog::settle_buttons`.
    #[prop(boxed = "FnMut(&mut BodyCx<'_>, &mut [FooterButton]) + 'a", name = "settle_buttons")]
    settle: Option<SettleFn<'a>>,
    /// Acts on a picked footer button in the footer step, before the close is decided: `act` gets the button and the
    /// dialog's body context. `ResetFooter::install` sets it.
    #[prop(boxed = "FnMut(&FooterButton, &mut BodyCx<'_>) + 'a", name = "on_pick")]
    on_pick: Option<OnPickFn<'a>>,
    buttons: Vec<FooterButton>,
    sides: Option<SidesFn<'a>>,
    side_footer: SideFooter<'a>,
    /// Whether the title row has a close action.
    #[prop]
    closable: bool,
    closed: Option<CloseReason>,
    picked: Option<usize>,
    /// The body ran in this pass: the header region marks it.
    body_shown: bool,
}

impl<'a> FullWindowDialog<'a> {
    /// A dialog titled `title`, with no close action.
    pub fn new(title: impl Into<Label>) -> Self {
        let props = ContainerProps::new()
            .mount(Mount::FullWindow)
            .surface(SurfaceKind::Panel)
            .pad(Pad::None)
            .header(HeaderRegion::new().rule(true).inset(Pad::Regular))
            .focus(FocusPolicy::DIALOG.initial(Initial::Keep))
            .command_scope(ViewCommands::SCOPE);
        FullWindowDialog {
            common: Common::new(),
            props,
            open: None,
            footer: None,
            guard: None,
            settle: None,
            on_pick: None,
            buttons: Vec::new(),
            sides: None,
            side_footer: SideFooter::bottom(),
            closable: false,
            closed: None,
            picked: None,
            body_shown: false,
        }
        .label(title)
    }

    /// Shows the dialog in its own OS window described by `spec`, resizable down to [`OWN_WINDOW_MIN_SIZE`], opening at
    /// the saved placement; where viewports are unavailable it fills the main window. The dialog's focus policy moves
    /// into the fallback overlay.
    #[must_use]
    pub fn own_window(mut self, spec: WindowSpec) -> Self {
        // A viewport mount takes its scope from the fallback overlay and holds `props.focus` at none.
        let focus = std::mem::replace(self.focus_mut(), FocusPolicy::NONE);
        self.props.mount = Mount::viewport_with(Overlay::full_window(focus).min_size(OWN_WINDOW_MIN_SIZE), spec);
        self
    }

    /// Who gets Enter: the dialog's submit, or the body.
    #[must_use]
    pub fn enter(mut self, rule: EnterRule) -> Self {
        let focus = self.focus_mut();
        *focus = FocusPolicy { enter: rule, ..*focus };
        self
    }

    /// What takes the keyboard focus when the dialog opens.
    #[must_use]
    pub fn initial_focus(mut self, initial: Initial) -> Self {
        let focus = self.focus_mut();
        *focus = focus.initial(initial);
        self
    }

    /// The dialog's focus policy, wherever its mount keeps it: the fallback overlay's in its own window, else the
    /// container's. Every builder and the footer go through it, so the order of `own_window` and the others does not
    /// matter.
    fn focus_mut(&mut self) -> &mut FocusPolicy {
        match &mut self.props.mount {
            Mount::Viewport { fallback, .. } => &mut fallback.focus,
            _ => &mut self.props.focus,
        }
    }

    /// The dialog's own command scope, a child of [`ViewCommands::SCOPE`]: the dialog is an instance of `scope` and still
    /// of the view scope. Both scopes' chords act wherever the focus is inside the dialog, and the view commands still
    /// reach the zoom bar; `scope`'s commands reach the dialog's body by `take_command` on its context.
    #[must_use]
    pub fn command_scope(mut self, scope: CommandScope) -> Self {
        self.props = self.props.command_scope(scope);
        self
    }

    /// Whether the dialog holds one outer gesture over its body's fields, from open to close: the app's `GestureHost`
    /// keeps one undo step for the dialog, named by its title. OK commits it; Cancel, Escape, the close action and the
    /// app closing the dialog take its edits back. The host folds every gesture it handles while the dialog is open
    /// into the step, so it suits a modal dialog.
    #[must_use]
    pub fn outer_gesture(mut self, on: bool) -> Self {
        self.props = self.props.outer_edit(on);
        self
    }

    /// Saves the open state under `key`.
    #[must_use]
    pub fn remember(mut self, key: SaveKey) -> Self {
        self.props = self.props.save(key);
        self
    }

    /// The dialog buttons in the footer, in the platform's order for their roles. A click closes the dialog: `Cancel`
    /// as `CloseReason::Requested`, every other role as `CloseReason::Activated` unless the button `stays_open`;
    /// `DialogOutput::picked` says which.
    #[must_use]
    pub fn buttons(mut self, buttons: impl IntoIterator<Item = FooterButton>) -> Self {
        let region = self.footer_region();
        self.props = self.props.footer(region);
        self.buttons = buttons.into_iter().collect();
        self
    }

    /// The footer's content: the closure fills a footer bar above the buttons, or alone.
    #[must_use]
    pub fn footer(mut self, content: impl FnMut(&mut Ui, &mut FooterCx<'_, '_, '_>) + 'a) -> Self {
        let region = self.footer_region();
        self.props = self.props.footer(region);
        self.footer = Some(Box::new(content));
        self
    }

    /// The side columns: `content` fills the region at the end edge, as high as the dialog under the title row and
    /// as wide as what it shows, before the body takes the rest of the width. It shows the columns
    /// ([`FullWindowSide`](crate::dialog::FullWindowSide)) side by side in reading order, and the dialog's footer
    /// sits among them.
    #[must_use]
    pub fn sides(mut self, content: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'a) -> Self {
        self.props = self.props.end(EndRegion::new().full_height(true).rule(true));
        self.sides = Some(Box::new(content));
        self.place_footer()
    }

    /// Where the footer sits among the side columns, and its leading slot; a dialog without side columns ignores it.
    #[must_use]
    pub fn side_footer(mut self, footer: SideFooter<'a>) -> Self {
        self.side_footer = footer;
        self.place_footer()
    }

    /// The footer region: among the side columns when there are any, else across the dialog's foot.
    fn footer_region(&self) -> FooterRegion {
        FOOTER.inset(Pad::Regular).place(if self.sides.is_some() { self.side_footer.place() } else { FooterPlace::Across })
    }

    fn place_footer(mut self) -> Self {
        if self.props.footer.is_some() {
            let region = self.footer_region();
            self.props = self.props.footer(region);
        }
        self
    }
}

/// Fills the side columns' region.
type SidesFn<'a> = Box<dyn FnMut(&mut Ui, &mut BodyCx<'_>) + 'a>;

impl Component for FullWindowDialog<'_> {
    const ROLE: Role = Role::DIALOG;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = FullWindowDialogStyle;
    type Output = DialogOutput;

    fn style(_: &StyleCx<'_>, _: FullWindowDialogStyleKey) -> FullWindowDialogStyle {
        FullWindowDialogStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> DialogOutput {
        let output = DialogOutput { response: cx.response.clone(), closed: self.closed, picked: self.picked };
        if self.props.outer_edit {
            // The dialog's answer decides the end: an OK commits, every other close cancels.
            let end = output.result(|| OuterEnd::Commit).map(|answer| answer.ok().unwrap_or(OuterEnd::Cancel));
            let core = &mut cx.core;
            outer::settle(&core.ctx, core.memory, core.id, self.body_shown, end);
        }
        output
    }
}

impl Container for FullWindowDialog<'_> {
    fn open(&mut self) -> Option<BindMut<'_, bool>> {
        self.open.as_mut().map(Bind::reborrow)
    }

    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        self.body_shown = true;
        if let Some(scope) = self.props.command_scope.filter(|scope| *scope != ViewCommands::SCOPE) {
            commands::declare_child(ui.ctx(), scope, ViewCommands::SCOPE);
        }
        if self.props.outer_edit {
            outer::keep(ui.ctx(), cx);
        }
        // The dialog covers the window's top, where macOS draws the traffic lights over the content: the title row
        // starts clear of them, as the shell's title bar does.
        let mut room = ui.available_rect_before_wrap();
        let Some(lights) = WindowHost::of(ui.ctx()).lights_room().filter(|lights| lights.intersects(room)) else {
            frame::header(ui, cx, frame::close_action(self.closable), None, None);
            return;
        };
        if lights.center().x < room.center().x {
            room.min.x = lights.max.x.min(room.max.x);
        } else {
            room.max.x = lights.min.x.max(room.min.x);
        }
        let closable = self.closable;
        ui.scope_builder(UiBuilder::new().max_rect(room).layout(*ui.layout()), |ui| frame::header(ui, cx, frame::close_action(closable), None, None));
    }

    fn end(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        if let Some(sides) = self.sides.as_mut() {
            sides(ui, cx);
        }
    }

    fn footer(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        let enter = self.focus_mut().enter == EnterRule::Submit;
        let (content, buttons) = (&mut self.footer, &self.buttons);
        let (picked, guard, on_pick, settle) = (&mut self.picked, &mut self.guard, &mut self.on_pick, &mut self.settle);
        if self.sides.is_some() {
            self.side_footer.show(ui, |ui| {
                let host = frame::FooterHost { picked: &mut *picked, guard: &mut *guard, on_pick: &mut *on_pick, settle: &mut *settle };
                frame::footer(ui, cx, content, buttons, enter, host);
            });
        } else {
            frame::footer(ui, cx, content, buttons, enter, frame::FooterHost { picked, guard, on_pick, settle });
        }
    }

    fn closed(&mut self, why: CloseReason) {
        self.closed.get_or_insert(why);
    }
}

impl<'a> FooterHooks<'a> for FullWindowDialog<'a> {
    fn buttons(self, buttons: impl IntoIterator<Item = FooterButton>) -> Self {
        FullWindowDialog::buttons(self, buttons)
    }

    fn settle_buttons<T: FnMut(&mut BodyCx<'_>, &mut [FooterButton]) + 'a>(self, settle: T) -> Self {
        FullWindowDialog::settle_buttons(self, settle)
    }

    fn on_pick<T: FnMut(&FooterButton, &mut BodyCx<'_>) + 'a>(self, act: T) -> Self {
        FullWindowDialog::on_pick(self, act)
    }

    fn close_guard<T: FnMut(&FooterButton, &mut BodyCx<'_>) -> bool + 'a>(self, allows: T) -> Self {
        FullWindowDialog::close_guard(self, allows)
    }
}
