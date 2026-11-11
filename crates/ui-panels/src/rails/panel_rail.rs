//! `PanelRail`: the rail of icons that open panels.

use super::flyout::FlyoutButton;
use super::tool::RailTool;
use super::words::RailWords;
use crate::contract::{
    Access, BodyCx, BodyLayout, Common, Component, Container, ContainerProps, Cx, FooterRegion, Gap, NoValue, Outcome, Pad, Props as _, Role, Scroll,
    Show as _, ShowContainer as _, Sizing, WordSetCx as _,
};
use crate::runtime::access::Name;
use crate::runtime::commands;
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx, SurfaceKind};
use craft_behavior::{Metrics, NavShape, Roving, Sense, Value};
use craft_theme::ControlSize;
use egui::{Response, Ui};

#[cfg(feature = "testing")]
pub(super) mod stories;
#[cfg(feature = "testing")]
mod trials;

/// The style of a panel rail: it paints its chrome surface and takes its spacing from the body gap its props name.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PanelRailStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What a click on a panel's icon asks the host to do.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PanelRailRequest<P> {
    /// Open the panel: its icon was clicked while the panel was closed.
    Open(P),
    /// Close the panel: its icon was clicked while the panel was open.
    Close(P),
}

/// What a panel rail reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PanelRailOutput<P> {
    /// The rail's response.
    pub response: Response,
    /// The panel the user asked to open or close this pass. Its command has run.
    pub request: Option<PanelRailRequest<P>>,
}

impl<P: Value> Outcome for PanelRailOutput<P> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

type FooterFn<'a> = Box<dyn FnMut(&mut Ui) + 'a>;

/// The rail of icons that open panels, as a side panel at one edge of the window. A panel is a toggle command: its
/// label, icon, chord and enabled state come from the command, the icon shows the on look while the command is
/// checked (the panel is open), and the command's `marked` state and `hint` tell that the closed panel has content
/// to show. A click runs the command and reports the request that matches the panel's state. The icons are one
/// roving group: Up and Down move between them, Enter and Space toggle. The host's footer stays in view under the
/// icons, which scroll when the window is short.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container, probe(PanelRail<'static, u8>))]
pub struct PanelRail<'a, P: Value + 'static> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    panels: &'a [RailTool<P>],
    footer: Option<FooterFn<'a>>,
}

impl<'a, P: Value + 'static> PanelRail<'a, P> {
    /// A rail named `label` of `panels`, each a panel key with the toggle command that opens and closes it.
    pub fn new(label: impl Into<Label>, panels: &'a [RailTool<P>]) -> Self {
        let props = ContainerProps::new()
            .surface(SurfaceKind::Chrome)
            .pad(Pad::Control)
            .sizing(Sizing::Content)
            .layout(BodyLayout::stack())
            .body_gap(Gap::Tight)
            .child_size(ControlSize::Md)
            .roving(Roving::new(NavShape::Flat))
            .scroll(Scroll::vertical())
            .footer(FooterRegion::new());
        PanelRail { common: Common::new(), props, panels, footer: None }.label(label)
    }

    /// Shows the rail. `footer` draws under the last icon; it stays in view when the window is short.
    pub fn show_rail(mut self, ui: &mut Ui, footer: impl FnMut(&mut Ui) + 'a) -> PanelRailOutput<P> {
        self.footer = Some(Box::new(footer));
        let panels = self.panels;
        let mut request = None;
        let shown = self.show(ui, |ui, _| {
            let ctx = ui.ctx().clone();
            for (at, panel) in panels.iter().enumerate() {
                let open = commands::resolve(&ctx, panel.command.clone()).checked.unwrap_or(false);
                let button = FlyoutButton::new(std::slice::from_ref(panel), &panel.id).selected(open).id_salt(("panel", at)).show(ui);
                if button.select.is_some() {
                    request = Some(if open { PanelRailRequest::Close(panel.id.clone()) } else { PanelRailRequest::Open(panel.id.clone()) });
                }
            }
        });
        PanelRailOutput { request, ..shown.output }
    }
}

impl<P: Value + 'static> Component for PanelRail<'_, P> {
    const ROLE: Role = Role::TOOLBAR;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = PanelRailStyle;
    type Output = PanelRailOutput<P>;

    fn style(_: &StyleCx<'_>, _: PanelRailStyleKey<P>) -> PanelRailStyle {
        PanelRailStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    /// A rail the caller did not label is named by the words of the family.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if cx.resolved.label.is_empty() {
            access.name(Name::Text(cx.words::<RailWords>().panels.as_str().to_owned()));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> PanelRailOutput<P> {
        PanelRailOutput { response: cx.response.clone(), request: None }
    }
}

impl<P: Value + 'static> Container for PanelRail<'_, P> {
    /// The host's footer.
    fn footer(&mut self, ui: &mut Ui, _: &mut BodyCx<'_>) {
        if let Some(footer) = &mut self.footer {
            footer(ui);
        }
    }
}
