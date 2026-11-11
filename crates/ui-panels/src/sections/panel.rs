//! `Panel` (16.5): an inline container on the panel surface that fills the room it is given. Its header is a
//! `TitleRow` with optional icon, subtitle and actions, its body scrolls vertically and an optional footer sits under
//! the body. The panel is one pane for F6.

use super::FooterBar;
use super::title_row::{HeaderAction, HeaderCommand, HeaderStrength, MenuRows, TitleRow};
use super::words::SectionWords;
use crate::contract::overlay::Popup;
use crate::contract::{
    Bind, BindMut, BodyCx, Common, Component, Container, ContainerProps, Cx, FooterRegion, HeaderRegion, NoValue, Outcome, Pad, PaintUi, Props as _,
    Role, Scroll, Show as _, ShowContainer as _, Sizing, WordSetCx as _,
};
use crate::primitives::{IconSpec, PaintShape, StrokeWidth};
use crate::runtime::env::Label;
use crate::runtime::focus::FocusPolicy;
use crate::runtime::overlay::CloseReason;
use crate::style::{ComponentStyle, StyleColor, StyleCx, SurfaceKind, UiTokens};
use craft_behavior::{Metrics, Sense};
use egui::{CornerRadius, Id, Response, Ui};

#[cfg(feature = "testing")]
mod stories;

/// The style of a panel: the surface and padding come from the container.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PanelStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The ring a selected panel shows inside its edge.
    ring: StyleColor,
    /// The line of a framed panel's outline.
    edge: StyleColor,
    corners: CornerRadius,
}

/// What a panel reports: its response, the header action taken this pass and how it closed.
#[derive(Debug)]
#[non_exhaustive]
pub struct PanelOutput {
    /// The panel's response.
    pub response: Response,
    /// The header action part taken this pass. The panel runs Close and Menu itself; Float is the host's.
    pub action: Option<HeaderAction>,
    /// How the panel closed in this pass (`Requested` for the close action); `None` while it stays open.
    pub closed: Option<CloseReason>,
}

impl Outcome for PanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The closure that fills the footer bar.
type FooterFn<'a> = Box<dyn FnMut(&mut Ui, &mut BodyCx<'_>) + 'a>;

/// A panel of a workspace: title row, scrolling body, optional footer.
///
/// - Header: the title, with the optional icon and subtitle, and the actions in `actions`. The Menu action opens the
///   rows given to `menu` under the title row; Close asks the panel to close, which hides its body until the host
///   stops showing it (bind the state with `opened` to keep it); Float is reported in `PanelOutput::action` for the
///   host. [`headerless`](Panel::headerless) leaves the header out.
/// - Body: scrolls vertically; the panel fills the width and height offered.
/// - Selected: `selected(true)` draws an accent ring inside the panel's edge, so a host can mark the picked panel.
/// - Framed: `framed()` outlines the panel's edge in the border line colour.
/// - Footer: the closure given to `footer` fills a footer bar under the body, separated by a rule.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub struct Panel<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    /// Text after the title in the header.
    #[prop(into)]
    subtitle: Option<Label>,
    /// An icon before the title.
    #[prop]
    icon: Option<IconSpec<'a>>,
    /// The rows of the menu the Menu action opens.
    #[prop(func = "FnMut(&mut Ui, &mut BodyCx<'_>)")]
    menu: Option<MenuRows<'a>>,
    actions: Vec<HeaderAction>,
    commands: Vec<HeaderCommand>,
    footer: Option<FooterFn<'a>>,
    taken: Option<HeaderAction>,
    menu_opener: Option<Id>,
    closed: Option<CloseReason>,
    pickable: bool,
    framed: bool,
    /// How much weight the header's title carries; a panel title by default.
    #[prop]
    strength: HeaderStrength,
}

impl<'a> Panel<'a> {
    /// A panel titled `title`, with a header, a vertical scroll and no footer.
    pub fn new(title: impl Into<Label>) -> Self {
        let props = ContainerProps::new()
            .surface(SurfaceKind::Panel)
            .pad(Pad::Regular)
            .sizing(Sizing::Fill)
            .header(HeaderRegion::new().rule(true))
            .scroll(Scroll::vertical())
            .focus(FocusPolicy::PANE);
        Panel {
            common: Common::new(),
            props,
            open: None,
            subtitle: None,
            icon: None,
            menu: None,
            actions: Vec::new(),
            commands: Vec::new(),
            footer: None,
            taken: None,
            menu_opener: None,
            closed: None,
            pickable: false,
            framed: false,
            strength: HeaderStrength::Strong,
        }
        .label(title)
    }

    /// The header as a band of `kind` across the panel's width and top edge, with no rule under it. The header's
    /// content keeps the regular padding on its sides whatever padding the body has.
    #[must_use]
    pub fn band(mut self, kind: SurfaceKind) -> Self {
        self.props.header = Some(HeaderRegion::new().fill(kind).inset(Pad::Regular));
        self
    }

    /// The padding tier around the body, regular by default. A header band keeps its own inset.
    #[must_use]
    pub fn pad(mut self, pad: Pad) -> Self {
        self.props = self.props.pad(pad);
        self
    }

    /// The action parts at the end of the header, in reading order.
    #[must_use]
    pub fn actions(mut self, actions: impl IntoIterator<Item = HeaderAction>) -> Self {
        self.actions = actions.into_iter().collect();
        self
    }

    /// The icon commands at the end of the header, before the actions, in reading order. Each is a Tab stop named and
    /// enabled by its command, which an activation fires.
    #[must_use]
    pub fn commands(mut self, commands: impl IntoIterator<Item = HeaderCommand>) -> Self {
        self.commands = commands.into_iter().collect();
        self
    }

    /// The menu the Menu action opens, as ready-made rows.
    #[doc(hidden)]
    pub fn set_menu_rows(&mut self, rows: MenuRows<'a>) {
        self.menu = Some(rows);
    }

    /// A panel a press picks: a press on its own surface, where no control inside takes it, is reported by
    /// [`BodyCx::surface_pressed`] in the body, so a host can select the panel.
    #[must_use]
    pub fn pickable(mut self) -> Self {
        self.pickable = true;
        self
    }

    /// A panel with a 1 px outline in the border line colour on its edge, over its header band and body.
    #[must_use]
    pub fn framed(mut self) -> Self {
        self.framed = true;
        self
    }

    /// A panel with no header: the title names the panel for assistive technology only.
    #[must_use]
    pub fn headerless(mut self) -> Self {
        self.props.header = None;
        self
    }

    /// The footer's content: the closure fills a footer bar under the body.
    #[must_use]
    pub fn footer(mut self, content: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'a) -> Self {
        self.props = self.props.footer(FooterRegion::new().rule(true));
        self.footer = Some(Box::new(content));
        self
    }
}

impl Component for Panel<'_> {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = PanelStyle;
    type Output = PanelOutput;

    fn style(cx: &StyleCx<'_>, _: PanelStyleKey) -> PanelStyle {
        PanelStyle {
            metrics: Metrics::new(),
            ring: cx.color(cx.tokens.color.accent.solid),
            edge: cx.color(cx.tokens.color.line.border),
            corners: UiTokens::corner(cx.tokens.shape.radius.panel),
        }
    }

    fn sense(&self) -> Sense {
        if self.pickable { Sense::SURFACE } else { Sense::empty() }
    }

    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if self.framed {
            ui.draw().stroke(&PaintShape::Rect(cx.rect, cx.style.corners), StrokeWidth::Thin, cx.style.edge);
        }
        if cx.resolved.selected == Some(true) {
            ui.draw().stroke(&PaintShape::Rect(cx.rect, cx.style.corners), StrokeWidth::Medium, cx.style.ring);
        }
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> PanelOutput {
        if let Some(opener) = self.menu_opener {
            Popup::toggle(opener, &mut cx);
        }
        PanelOutput { response: cx.response.clone(), action: self.taken, closed: self.closed }
    }
}

impl Container for Panel<'_> {
    fn open(&mut self) -> Option<BindMut<'_, bool>> {
        self.open.as_mut().map(Bind::reborrow)
    }

    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        let mut row = TitleRow::new(cx.label()).strength(self.strength).actions(self.actions.iter().copied()).commands(self.commands.iter().cloned());
        if let Some(subtitle) = self.subtitle.clone() {
            row = row.subtitle(subtitle);
        }
        if let Some(icon) = self.icon.clone() {
            row = row.icon(icon);
        }
        let shown = row.show(ui);
        if shown.action == Some(HeaderAction::Close) {
            cx.request_close();
        }
        self.taken = shown.action.or(self.taken);
        if let Some(rows) = self.menu.as_mut() {
            self.menu_opener = rows.hang(ui, &shown, cx.words::<SectionWords>().menu.clone());
        }
    }

    fn footer(&mut self, ui: &mut Ui, _: &mut BodyCx<'_>) {
        if let Some(content) = self.footer.as_mut() {
            FooterBar::body().show(ui, |ui, bar| content(ui, bar));
        }
    }

    fn closed(&mut self, why: CloseReason) {
        self.closed.get_or_insert(why);
    }
}
