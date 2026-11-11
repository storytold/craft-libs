//! `FullWindowSide`: a side column of a full-window dialog, with its title row and scrolling body; and `SideFooter`,
//! where the dialog's footer sits among the side columns.

use crate::contract::{
    BodyCx, Common, Component, Container, ContainerProps, Cx, FooterPlace, HeaderRegion, NoValue, Outcome, Pad, Props as _, Role, RowAlign, Scroll,
    Show as _, ShowContainer as _, Sizing, WordSetCx as _,
};
use crate::layout_components::Row;
use crate::runtime::focus::FocusPolicy;
use crate::sections::{HeaderStrength, TitleRow};
use crate::style::{ComponentStyle, StyleCx, SurfaceKind};
use craft_behavior::{Metrics, Sense};
use egui::{Response, Ui};

type LeadingFn<'a> = Box<dyn FnMut(&mut Ui) + 'a>;

/// Where the footer of a [`FullWindowDialog`](crate::dialog::FullWindowDialog) with side columns sits: at the bottom
/// of the side columns' region (Liquify, Select and Mask, Content-Aware Fill, Wide Angle, Vanishing Point) or at its
/// top (Filter Gallery), with an optional leading slot (a Reset icon button) at the reading start of the footer row.
/// The buttons are the dialog's own (`FullWindowDialog::buttons`).
#[must_use = "pass the footer to `FullWindowDialog::side_footer`"]
pub struct SideFooter<'a> {
    top: bool,
    leading: Option<LeadingFn<'a>>,
}

impl<'a> SideFooter<'a> {
    /// The footer at the bottom of the side columns, under their content.
    pub fn bottom() -> Self {
        SideFooter { top: false, leading: None }
    }

    /// The footer at the top of the side columns, above their content (Filter Gallery).
    pub fn top() -> Self {
        SideFooter { top: true, leading: None }
    }

    /// Shows `add` at the reading start of the footer row, the buttons at its end: Select and Mask's and
    /// Content-Aware Fill's Reset.
    pub fn leading(mut self, add: impl FnMut(&mut Ui) + 'a) -> Self {
        self.leading = Some(Box::new(add));
        self
    }

    /// Where the footer region sits.
    pub(crate) fn place(&self) -> FooterPlace {
        if self.top { FooterPlace::EndTop } else { FooterPlace::EndBottom }
    }

    /// Shows the footer row across the footer region: the leading slot at the reading start and `rest` (the dialog's
    /// footer content and buttons) at the end.
    pub(crate) fn show(&mut self, ui: &mut Ui, mut rest: impl FnMut(&mut Ui)) {
        let align = if self.leading.is_some() { RowAlign::SpaceBetween } else { RowAlign::End };
        let leading = &mut self.leading;
        Row::new().fill_width().align(align).show(ui, |ui, _| {
            if let Some(add) = leading.as_mut() {
                add(ui);
            }
            rest(ui);
        });
    }
}

impl Default for SideFooter<'_> {
    fn default() -> Self {
        SideFooter::bottom()
    }
}

/// The style of a side column: the surface and padding come from the container.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FullWindowSideStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What a side column reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct FullWindowSideOutput {
    /// The column's response.
    pub response: Response,
}

impl Outcome for FullWindowSideOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A side column of a full-window dialog: Liquify's and Select and Mask's Properties, Content-Aware Fill's Preview
/// and settings panels, Filter Gallery's settings. Shown in the dialog's side columns
/// ([`FullWindowDialog::sides`](crate::dialog::FullWindowDialog::sides)), side by side in reading order.
///
/// - **Look.** The panel surface; the title (the label) in a title row at the top, none with
///   [`headerless`](Self::headerless); the body padded by the panel tier.
/// - **Body.** Scrolls vertically when taller than the room; [`unscrolled`](Self::unscrolled) leaves the scroll out
///   for a pane that bounds itself.
/// - **Size.** As wide as its content; as high as its content, at most the room of the side columns.
/// - **Footer.** None of its own: the dialog's buttons sit in the dialog's footer, which [`SideFooter`] places among
///   the side columns.
/// - **Keyboard and AccessKit.** A group named by the label, an F6 pane; Tab goes top to bottom.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub struct FullWindowSide {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    /// Whether the title row shows.
    titled: bool,
}

impl FullWindowSide {
    /// A column named and titled `title`, with a scrolling body.
    pub fn new(title: impl Into<crate::runtime::env::Label>) -> Self {
        let props = ContainerProps::new()
            .surface(SurfaceKind::Panel)
            .pad(Pad::Regular)
            .sizing(Sizing::Content)
            .header(HeaderRegion::new().rule(true))
            .scroll(Scroll::vertical())
            .focus(FocusPolicy::PANE);
        FullWindowSide { common: Common::new(), props, titled: true }.label(title)
    }

    /// A column with no title row: the label names the column for assistive technology only.
    #[must_use]
    pub fn headerless(mut self) -> Self {
        self.titled = false;
        self.props.header = None;
        self
    }

    /// A column whose body does not scroll: a pane that bounds itself.
    #[must_use]
    pub fn unscrolled(mut self) -> Self {
        self.props.scroll = None;
        self
    }
}

impl Component for FullWindowSide {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = FullWindowSideStyle;
    type Output = FullWindowSideOutput;

    fn style(_: &StyleCx<'_>, _: FullWindowSideStyleKey) -> FullWindowSideStyle {
        FullWindowSideStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> FullWindowSideOutput {
        FullWindowSideOutput { response: cx.response.clone() }
    }
}

impl Container for FullWindowSide {
    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        if self.titled {
            TitleRow::new(cx.label()).strength(HeaderStrength::Strong).show(ui);
        }
    }
}
