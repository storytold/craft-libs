//! `TitleBar`: the app-drawn bar of a window, in the title slot of an `AppShell`.

#[cfg(feature = "testing")]
mod stories;

use super::measure::{End, Ends, Strip};
use super::style::TitleBarStyle;
use super::title::{Title, centre_line};
use crate::contract::{
    Access, BodyCx, BodyLayout, Common, Component, Container, ContainerProps, Cx, DragBand, EndRegion, Gap, HeaderRegion, NoValue, Outcome, Pad,
    PaintUi, Role, ShowContainer as _, Sizing, WordSetCx as _,
};
use crate::layout::{Placed, arrange};
use crate::primitives::RuleKind;
use crate::runtime::access::Name;
use crate::runtime::egui::pass;
use crate::runtime::env::Env;
use crate::runtime::windows::{self, WindowAction, WindowHost};
use crate::shell::app_shell::RegionBody;
use crate::shell::{Caption, CaptionButton, CaptionIcons, Frame, ShellWords, SlotSize};
use crate::style::{StyleCx, SurfaceKind};
use craft_behavior::Sense;
use craft_host::target::{DoubleClick, WindowState};
use egui::{Response, Ui, pos2};

/// What a title bar reports: the caption clicked and whether the window was asked to close.
#[derive(Debug)]
#[non_exhaustive]
pub struct TitleBarOutput {
    /// The bar's response.
    pub response: Response,
    /// The caption button clicked in this pass. Minimize and Maximize have already acted on the window.
    pub caption_clicked: Option<Caption>,
    /// The Close button was clicked. The bar never closes the window: the app runs its unsaved-documents path.
    pub close_requested: bool,
}

impl Outcome for TitleBarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The bar of a window whose frame the app draws.
///
/// The bar fills the width of its room; its height is its content's, the caption strip's `SlotSize::TitleBar`. What it shows follows the window's frame
/// (`WindowHost::frame`):
///
/// - **Native.** The OS draws the title bar; the app's strip shows its content and, with
///   [`title_when_native`](TitleBar::title_when_native), the title.
/// - **Inset (macOS).** The content starts after the traffic lights. A double click follows the system setting.
/// - **Integrated.** The caption buttons of the window's layout sit at the two ends, flush with the window's edges.
///   A double click maximizes or restores.
///
/// In full screen the bar shows no captions and reserves no room. The free area is the container's move band:
/// pressing and dragging it moves the window, and a double click acts as above. The body is the leading content; the
/// trailing content ([`trailing`](TitleBar::trailing)) sits before the end's captions. [`show_titled`](TitleBar::show_titled)
/// adds the title in the gap between them. Close never closes the window; see [`TitleBarOutput`].
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
    /// Glyphs from the desktop's icon theme for the round caption families.
    #[prop]
    icons: Option<&'a CaptionIcons>,
    /// The content at the end of the bar, before the caption buttons.
    #[prop(func = "FnMut(&mut Ui, &mut crate::contract::BodyCx<'_>)")]
    trailing: Option<RegionBody<'a>>,
    title: Option<Title>,
    clicked: Option<Caption>,
    close: bool,
}

/// The facts of the window a pass of the bar reads.
struct Window {
    frame: Frame,
    maximized: bool,
    double_click: DoubleClick,
    ends: Ends,
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
            icons: None,
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
        self.trailing(trailing)
            .show(ui, |ui, cx| {
                leading(ui, cx);
                if !native_hidden {
                    let rtl = cx.env.direction().is_rtl();
                    let margin = cx.gap().unwrap_or_default();
                    title.show_in_gap(ui, cx, cx.rect.width(), centre_line(cx, rtl), margin);
                }
            })
            .output
    }

    fn window(&self, cx: &BodyCx<'_>) -> Window {
        let ctx = cx.memory.ctx().clone();
        let shell = windows::shell_of(&ctx, pass::current_viewport(&ctx));
        let frame = self.frame.clone().unwrap_or_else(|| WindowHost::of(&ctx).frame());
        let state = shell.window.state;
        let rtl = cx.env.direction().is_rtl();
        let ends = Ends::of(&frame, state != WindowState::Fullscreen, rtl);
        Window { frame, maximized: self.maximized.unwrap_or(state == WindowState::Maximized), double_click: shell.double_click, ends }
    }

    /// Shows one end's room and buttons, and acts on a click: Minimize and Maximize go to the window, Close is
    /// reported.
    fn show_end(&mut self, ui: &mut Ui, cx: &BodyCx<'_>, end: &End, window: &Window) {
        let height = SlotSize::TitleBar.resolve(&Env::of(ui).tokens);
        let mut buttons: Vec<Placed<'_>> = end
            .captions
            .iter()
            .enumerate()
            .map(|(index, slot)| {
                let mut button = CaptionButton::new(slot.caption, window.ends.family).maximized(window.maximized);
                if let Some(icons) = self.icons {
                    button = button.icons(icons);
                }
                if let Some(edge) = slot.end {
                    button = button.outermost(edge);
                }
                Placed::new(("caption", index), button)
            })
            .collect();
        arrange(ui, &Strip::new(end, height), &mut buttons);
        let ctx = cx.memory.ctx().clone();
        let (host, viewport) = (WindowHost::of(&ctx), pass::current_viewport(&ctx));
        for (slot, placed) in end.captions.iter().zip(&buttons) {
            if !placed.response().is_some_and(Response::clicked) {
                continue;
            }
            self.clicked = Some(slot.caption);
            match slot.caption {
                Caption::Minimize => {
                    host.act(viewport, WindowAction::Minimize);
                }
                Caption::Maximize => {
                    host.act(viewport, WindowAction::SetMaximized(!window.maximized));
                }
                Caption::Close => self.close = true,
                _ => {}
            }
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
    const PARTS: bool = false;
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

    /// The hairline along the bottom edge, over the content.
    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let y = cx.rect.bottom() - cx.style.rule / 2.0;
        ui.draw().rule(RuleKind::Divider, pos2(cx.rect.left(), y), pos2(cx.rect.right(), y));
    }

    fn finish(self, cx: Cx<'_, Self>) -> TitleBarOutput {
        TitleBarOutput { response: cx.response.clone(), caption_clicked: self.clicked, close_requested: self.close }
    }
}

impl Container for TitleBar<'_> {
    /// The start end: the room for the traffic lights, or the captions at the start edge. A double click on the free
    /// band acts here, once a pass.
    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        let window = self.window(cx);
        if cx.band_double_pressed()
            && let Some(action) = windows::double_click_action(&window.frame, window.double_click, window.maximized)
        {
            let ctx = cx.memory.ctx().clone();
            WindowHost::of(&ctx).act(pass::current_viewport(&ctx), action);
        }
        self.show_end(ui, cx, &window.ends.start, &window);
    }

    /// The end end: the trailing content, then the room for the lights or the captions at the end edge.
    fn end(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        if let Some(RegionBody(trailing)) = self.trailing.as_mut() {
            trailing(ui, cx);
        }
        let window = self.window(cx);
        self.show_end(ui, cx, &window.ends.end, &window);
    }
}
