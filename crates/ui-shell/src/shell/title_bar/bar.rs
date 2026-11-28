//! `TitleBar`: the app-drawn bar of a window, in the title slot of an `AppShell`.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::measure::{End, Ends, Strip};
use super::style::TitleBarStyle;
use super::title::{Title, centre_line};
use crate::contract::{
    Access, BodyCx, BodyLayout, Common, Component, Container, ContainerProps, Cx, DragBand, EndRegion, Gap, HeaderRegion, NoValue, Outcome, Pad,
    PartsCx, Role, ShowContainer as _, Sizing, WordSetCx as _,
};
use crate::layout::{Placed, arrange};
use crate::layout_components::Row;
use crate::runtime::access::Name;
use crate::runtime::egui::pass;
use crate::runtime::env::Env;
use crate::runtime::memory;
use crate::runtime::windows::{self, WindowAction, WindowHost};
use crate::shell::app_shell::RegionBody;
use crate::shell::{Caption, CaptionButton, CaptionIcons, Frame, ShellWords, SlotSize};
use crate::style::{StyleCx, SurfaceKind};
use craft_behavior::{PartKey, Sense};
use craft_host::target::{DoubleClick, WindowState};
use craft_window::desktop::Desktop;
use egui::{Rect, Response, Ui};
use std::sync::Arc;

/// What a title bar reports: the caption clicked and whether the window was asked to close.
#[derive(Debug)]
#[non_exhaustive]
pub struct TitleBarOutput {
    /// The bar's response.
    pub response: Response,
    /// The caption button clicked in this pass. Minimize and Maximize have already acted on the window, unless the bar
    /// is in an embedded shell.
    pub caption_clicked: Option<Caption>,
    /// The Close button was clicked. The bar never closes the window: the app runs its unsaved-documents path.
    pub close_requested: bool,
    /// The title was activated in this pass; only a title with an action can be.
    pub title_activated: bool,
}

impl Outcome for TitleBarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The bar of a window whose frame the app draws.
///
/// The bar paints the chrome surface; `surface` names another role (`SurfaceKind::Background` for an app whose title
/// bar is darker than its other bars).
///
/// The bar fills the width of its room; its height is its content's, the caption strip's `SlotSize::TitleBar`. What it shows follows the window's frame
/// (`WindowHost::frame`):
///
/// - **Native.** The OS draws the title bar; the app's strip shows its content and, with
///   [`title_when_native`](TitleBar::title_when_native), the title.
/// - **Inset (macOS).** The content starts after the traffic lights. A double click follows the system setting.
/// - **Integrated.** The caption buttons of the window's layout sit at the two ends, flush with the window's edges.
///   A double click follows the desktop's setting (GNOME's `action-double-click-titlebar`), and elsewhere
///   maximizes or restores.
///
/// An end with no caption buttons or lights keeps the content `space.pad.regular` off the window's edge.
///
/// In full screen the bar shows no captions and reserves no room. The free area is the container's move band:
/// pressing and dragging it moves the window, and a double click acts as above.
///
/// In an embedded shell (an app preview, a story) the bar is a picture of a window: it acts on no window and reads
/// none. The captions show hover and report their clicks, but send no window command, and the bar has no move band. The body is the leading content; the
/// trailing content ([`trailing`](TitleBar::trailing)) sits before the end's captions. [`show_titled`](TitleBar::show_titled)
/// adds the title in the gap between them; [`show_fitted`](TitleBar::show_fitted) fits the trailing content after the
/// leading content: its ranked members leave before the leading content narrows. Close never closes the window; see
/// [`TitleBarOutput`].
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub struct TitleBar<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    /// The frame to draw, in place of the running window's: a story or a preview.
    #[prop]
    frame: Option<Frame>,
    /// Whether the window shows as maximized, in place of the running window's state.
    #[prop]
    maximized: Option<bool>,
    /// Whether a bar in a native frame shows the title.
    #[prop(flag)]
    title_when_native: bool,
    /// The content at the end of the bar, before the caption buttons.
    #[prop(func = "FnMut(&mut Ui, &mut crate::contract::BodyCx<'_>)")]
    trailing: Option<RegionBody<'a>>,
    title: Option<Title>,
    clicked: Option<Caption>,
    close: bool,
}

/// The facts of the window a pass of a bar or a page band reads.
pub(super) struct Window {
    pub(super) frame: Frame,
    pub(super) maximized: bool,
    pub(super) double_click: DoubleClick,
    pub(super) ends: Ends,
    /// Whether the bar acts on the running window: false in an embedded shell.
    pub(super) acts: bool,
    /// The frame was given in place of the running window's (a story, a preview).
    pub(super) given: bool,
    /// The window the bar acts on is in OS full screen, where it has no ends.
    pub(super) full_screen: bool,
}

/// The facts of the window a bar in `cx` draws: `frame` and `maximized` stand in for the running window's when given.
/// In an embedded shell the bar acts on no window and reads it as a normal window.
pub(super) fn window(cx: &BodyCx<'_>, frame: Option<Frame>, maximized: Option<bool>) -> Window {
    let acts = !cx.shell().is_some_and(|shell| shell.region().is_some());
    window_acting(cx, frame, maximized, acts)
}

/// [`window`] for a bar that acts on the running window when `acts`, for a caller that knows it before the shell
/// around it is set (the shell itself).
pub(super) fn window_acting(cx: &BodyCx<'_>, frame: Option<Frame>, maximized: Option<bool>, acts: bool) -> Window {
    let ctx = cx.memory.ctx().clone();
    let shell = windows::shell_of(&ctx, pass::current_viewport(&ctx));
    let given = frame.is_some();
    let frame = frame.unwrap_or_else(|| WindowHost::of(&ctx).frame());
    let state = if acts { shell.window.state } else { WindowState::Normal };
    let full_screen = state == WindowState::Fullscreen;
    let rtl = cx.env.direction().is_rtl();
    let ends = Ends::of(&frame, !full_screen, rtl);
    Window {
        frame,
        maximized: maximized.unwrap_or(state == WindowState::Maximized),
        double_click: shell.double_click,
        ends,
        acts,
        given,
        full_screen,
    }
}

/// Acts on a double click on the move band of the container `cx` runs in, once a pass: maximizes, restores or
/// minimizes the window as the platform's setting says. A bar that acts on no window does nothing.
pub(super) fn act_on_double_click(cx: &BodyCx<'_>, window: &Window) {
    if window.acts
        && cx.band_double_pressed()
        && let Some(action) = windows::double_click_action(&window.frame, window.double_click, window.maximized)
    {
        let ctx = cx.memory.ctx().clone();
        WindowHost::of(&ctx).act(pass::current_viewport(&ctx), action);
    }
}

/// Shows one end's room and buttons, and acts on a click: Minimize and Maximize go to the window the bar acts on.
/// Returns the caption clicked in this pass; the caller decides what Close means.
pub(super) fn show_end(ui: &mut Ui, cx: &BodyCx<'_>, end: &End, window: &Window) -> Option<Caption> {
    let tokens = Env::of(ui).tokens.clone();
    let height = SlotSize::TitleBar.resolve(&tokens);
    let pad = tokens.tokens().space.pad.regular;
    // The running window's captions take the glyphs of the desktop's icon theme; a frame given in place of the
    // window's (a story, a preview) draws the family's own.
    let ctx = cx.memory.ctx().clone();
    let icons = if window.acts && !window.given { desktop_icons(&ctx) } else { None };
    let mut buttons: Vec<Placed<'_>> = end
        .captions
        .iter()
        .enumerate()
        .map(|(index, slot)| {
            let mut button = CaptionButton::new(slot.caption, window.ends.family).maximized(window.maximized);
            if let Some(icons) = &icons {
                button = button.icons(Arc::clone(icons));
            }
            if let Some(edge) = slot.end {
                button = button.outermost(edge);
            }
            Placed::new(("caption", index), button)
        })
        .collect();
    arrange(ui, &Strip::padded(end, height, pad), &mut buttons);
    let (host, viewport) = (WindowHost::of(&ctx), pass::current_viewport(&ctx));
    let mut clicked = None;
    for (slot, placed) in end.captions.iter().zip(&buttons) {
        if !placed.response().is_some_and(Response::clicked) {
            continue;
        }
        clicked = Some(slot.caption);
        match slot.caption {
            Caption::Minimize | Caption::Maximize if !window.acts => {}
            Caption::Minimize => {
                host.act(viewport, WindowAction::Minimize);
            }
            Caption::Maximize => {
                host.act(viewport, WindowAction::SetMaximized(!window.maximized));
            }
            _ => {}
        }
    }
    clicked
}

impl<'a> TitleBar<'a> {
    /// A title bar.
    pub fn new() -> Self {
        let props = ContainerProps::new()
            .surface(SurfaceKind::Chrome)
            .pad(Pad::None)
            .sizing(Sizing::FillWidth)
            .body_gap(Gap::Row)
            .layout(BodyLayout::row())
            .header(HeaderRegion::new().band(DragBand::Move).inline(true))
            .end(EndRegion::new());
        TitleBar {
            common: Common::new(),
            props,
            frame: None,
            maximized: None,
            title_when_native: false,
            trailing: None,
            title: None,
            clicked: None,
            close: false,
        }
    }

    /// Shows the bar with `leading` content at its start, `title` in the gap and `trailing` content before the end's
    /// captions.
    pub fn show_titled(
        mut self,
        ui: &mut Ui,
        title: Title,
        mut leading: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'a,
        trailing: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'a,
    ) -> TitleBarOutput {
        let native_hidden =
            !self.title_when_native && matches!(self.frame.clone().unwrap_or_else(|| WindowHost::of(ui.ctx()).frame()), Frame::Native);
        self.title = Some(title.clone());
        let mut activated = false;
        let mut out = self
            .trailing(trailing)
            .show(ui, |ui, cx| {
                leading(ui, cx);
                if !native_hidden {
                    let rtl = cx.env.direction().is_rtl();
                    let margin = cx.gap().unwrap_or_default();
                    activated = title.show_in_gap(ui, cx, cx.rect.width(), centre_line(cx, rtl), margin);
                }
            })
            .output;
        out.title_activated = activated;
        out
    }

    /// Shows the bar with `leading` content at its start and `trailing` content before the end's captions, where the
    /// end region joins the body's fit (`EndRegion::fit`): on a short bar the trailing members that carry a `fit_rank`
    /// leave whole, lowest rank first, before the leading content narrows: an app bar whose menus stay whole while its
    /// end controls go.
    pub fn show_fitted(
        mut self,
        ui: &mut Ui,
        leading: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'a,
        mut trailing: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'a,
    ) -> TitleBarOutput {
        self.props = self.props.end(EndRegion::new().fit(true));
        // The ranked members fit as units of the first row in the end region.
        self.trailing(move |ui: &mut Ui, cx: &mut BodyCx<'_>| {
            let _ = Row::new().show(ui, |ui, _| trailing(ui, cx));
        })
        .show(ui, leading)
        .output
    }

    /// Shows one end of the bar and records its clicked caption: Close is reported, never acted on.
    fn end_of(&mut self, ui: &mut Ui, cx: &BodyCx<'_>, end: &End, window: &Window) {
        if let Some(caption) = show_end(ui, cx, end, window) {
            self.clicked = Some(caption);
            self.close |= caption == Caption::Close;
        }
    }
}

impl Default for TitleBar<'_> {
    fn default() -> Self {
        TitleBar::new()
    }
}

impl Component for TitleBar<'_> {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = true;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = TitleBarStyle;
    type Output = TitleBarOutput;

    fn style(cx: &StyleCx<'_>, _: TitleBarStyleKey) -> TitleBarStyle {
        TitleBarStyle::of(cx)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    /// One inert part over the bar's rect. A press anywhere on the bar (its move band, the menu titles, the captions)
    /// keeps a focused draft (the zoom field) unapplied; sensing nothing, the part leaves presses and hover to the
    /// members under it.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.part(PartKey::new("title_bar"), rect).sense(Sense::empty()).holds_drafts();
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<ShellWords>();
        if cx.resolved.label.is_empty() {
            let name = self.title.as_ref().map_or_else(|| words.title_bar.as_str().to_owned(), |title| title.shown(&words));
            access.name(Name::Text(name));
        }
        if self.title.as_ref().is_some_and(Title::is_unsaved) {
            access.description(words.unsaved.as_str());
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> TitleBarOutput {
        TitleBarOutput { response: cx.response.clone(), caption_clicked: self.clicked, close_requested: self.close, title_activated: false }
    }
}

impl Container for TitleBar<'_> {
    /// The start end: the room for the traffic lights, or the captions at the start edge. A double click on the free
    /// band acts here, once a pass.
    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        let window = window(cx, self.frame.clone(), self.maximized);
        act_on_double_click(cx, &window);
        self.end_of(ui, cx, &window.ends.start, &window);
    }

    /// The end end: the trailing content, then the room for the lights or the captions at the end edge.
    fn end(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        if let Some(RegionBody(trailing)) = self.trailing.as_mut() {
            trailing(ui, cx);
        }
        let window = window(cx, self.frame.clone(), self.maximized);
        self.end_of(ui, cx, &window.ends.end, &window);
    }
}

/// The caption glyphs of the icon theme of the desktop the runtime reports (`WindowHost::desktop`), built with
/// `CaptionIcons::for_desktop` once per desktop value and kept for the app's bars while the desktop stays the same.
fn desktop_icons(ctx: &egui::Context) -> Option<Arc<CaptionIcons>> {
    let desktop = WindowHost::of(ctx).desktop()?;
    let key = memory::unique("craft_ui.title_bar.desktop_icons");
    memory::with_session(ctx, key, |kept: &mut Option<(Desktop, Option<Arc<CaptionIcons>>)>| {
        if let Some((seen, icons)) = kept.as_ref()
            && *seen == desktop
        {
            return icons.clone();
        }
        let previous = kept.take().and_then(|(_, icons)| icons).map(Arc::unwrap_or_clone);
        let icons = CaptionIcons::for_desktop(&desktop, previous).map(Arc::new);
        *kept = Some((desktop, icons.clone()));
        icons
    })
}
