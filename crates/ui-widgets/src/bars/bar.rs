//! `Bar`: the container of a toolbar, an options bar or a status bar.

use super::words::BarWords;
use crate::buttons::IconButton;
use crate::contract::{
    Access, BodyCx, BodyLayout, Common, Component, Container, ContainerProps, Cx, EndRegion, Gap, InBar, NoValue, Outcome, Pad, Politeness,
    Props as _, Role, Scroll, Show as _, ShowContainer as _, Sizing, WordSetCx as _,
};
use crate::layout_components::Row;
use crate::runtime::access::Name;
use crate::runtime::commands::{CommandSet as _, Commands as _};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx, SurfaceKind};
use craft_behavior::{IconId, Metrics, NavShape, Roving, Sense};
use craft_theme::ControlSize;
use egui::{Response, Ui};
use std::num::NonZeroU8;

/// The trailing group's content.
type TrailingFn<'a> = Box<dyn FnMut(&mut Ui) + 'a>;

/// What a bar is for; it picks the bar's role, spacing and name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BarKind {
    /// A strip of tools or commands.
    Tool,
    /// The options of the active tool.
    Options,
    /// Status messages and readouts.
    Status,
}

/// The commands of a bar's end region. They belong to the bar that shows them: a chord reaches the bar that holds
/// key focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui_macros::CommandSet)]
#[command(prefix = "bar", scope = "bar")]
#[non_exhaustive]
pub enum BarCommands {
    /// Accept the edit the options bar belongs to.
    #[command(label = "Commit", icon = "check", default = "Enter")]
    Commit,
    /// Abandon the edit the options bar belongs to.
    #[command(label = "Cancel", icon = "ban", default = "Escape")]
    Cancel,
}

/// The style of a bar: it paints its surface and takes its spacing from the body gap its props name.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct BarStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What a bar reports: its response and the commit and cancel buttons clicked this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct BarOutput {
    /// The bar's response.
    pub response: Response,
    /// The commit button was clicked.
    pub committed: bool,
    /// The cancel button was clicked.
    pub cancelled: bool,
}

impl Outcome for BarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A bar over a chrome surface whose body holds the members in reading order: a tool bar's at the standard control
/// size, an options or status bar's at the small one. They are one roving group unless the bar is made of
/// [`tab_stops`](Bar::tab_stops). The bar is as tall as its tallest member plus the control padding, and takes the
/// width it is offered. A bar of one row lays its body out as a row; a bar of several rows as a stack whose rows the
/// caller makes with `Row`. A row too long for the bar first narrows its selects and elidable text, then scrolls, so a
/// member past the edge is reached by Tab and revealed. A rule between groups of members is a `Divider::vertical()`. A
/// bar with `commit_cancel` ends with a cancel and a commit button in its end region, outside the scroll; they run
/// `BarCommands`.
#[derive(crate::contract::Component)]
#[component(container)]
pub struct Bar<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    kind: BarKind,
    commit_cancel: Option<(Label, Label)>,
    committed: bool,
    cancelled: bool,
    trailing: Option<TrailingFn<'a>>,
}

impl<'a> Bar<'a> {
    /// A bar named `label` of `kind`.
    pub fn new(label: impl Into<Label>, kind: BarKind) -> Self {
        let gap = match kind {
            BarKind::Tool => Gap::Tight,
            BarKind::Options => Gap::Item,
            BarKind::Status => Gap::Section,
        };
        let pad = if kind == BarKind::Options { Pad::Regular } else { Pad::Control };
        let size = if kind == BarKind::Tool { ControlSize::Md } else { ControlSize::Sm };
        let mut props = ContainerProps::new()
            .surface(SurfaceKind::Chrome)
            .pad(pad)
            .sizing(Sizing::FillWidth)
            .body_gap(gap)
            .child_size(size)
            .roving(Roving::new(NavShape::Flat))
            .layout(BodyLayout::row())
            .scroll(Scroll::horizontal());
        if kind == BarKind::Status {
            props = props.live(Politeness::Polite);
        }
        Bar { common: Common::new(), props, kind, commit_cancel: None, committed: false, cancelled: false, trailing: None }.label(label)
    }

    /// Sets the number of rows the body holds.
    #[must_use]
    pub fn rows(mut self, rows: NonZeroU8) -> Self {
        if rows.get() > 1 {
            self.props = self.props.layout(BodyLayout::stack());
        }
        self
    }

    /// Makes each member its own Tab stop, in reading order, as media controls are; the arrows no longer move between
    /// them.
    #[must_use]
    pub fn tab_stops(mut self) -> Self {
        self.props.roving = None;
        self
    }

    /// Ends the bar with a cancel and a commit button named `cancel` and `commit`. They run `BarCommands::Cancel`
    /// and `BarCommands::Commit`, so their tooltips carry the chords the registry binds.
    #[must_use]
    pub fn commit_cancel(mut self, commit: impl Into<Label>, cancel: impl Into<Label>) -> Self {
        self.commit_cancel = Some((commit.into(), cancel.into()));
        self.props = self.props.end(EndRegion::new()).command_scope(BarCommands::SCOPE);
        self
    }

    /// Ends the bar with a group of controls that stays at the reading end, outside the scroll, while the members
    /// before it scroll. The group sits after the cancel and commit buttons.
    #[must_use]
    pub fn trailing(mut self, group: impl FnMut(&mut Ui) + 'a) -> Self {
        self.trailing = Some(Box::new(group));
        self.props = self.props.end(EndRegion::new());
        self
    }
}

impl Component for Bar<'_> {
    const ROLE: Role = Role::TOOLBAR;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const ALT_ROLES: &'static [Role] = &[Role::GROUP, Role::STATUS];
    const OVERLAYS: bool = false;
    type Style = BarStyle;
    type Output = BarOutput;

    fn style(_: &StyleCx<'_>, _: BarStyleKey) -> BarStyle {
        BarStyle { metrics: Metrics::new() }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    /// An options bar is a group and a status bar a status region; a bar the caller did not label is named by the
    /// words of the family.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        match self.kind {
            BarKind::Tool => {}
            BarKind::Options => {
                access.role(Role::GROUP);
            }
            BarKind::Status => {
                access.role(Role::STATUS);
            }
        }
        if cx.resolved.label.is_empty() {
            let words = cx.words::<BarWords>();
            let name = match self.kind {
                BarKind::Tool => &words.bar_tool,
                BarKind::Options => &words.bar_options,
                BarKind::Status => &words.bar_status,
            };
            access.name(Name::Text(name.as_str().to_owned()));
        }
    }

    fn content_env(&self, _: &Cx<'_, Self>, values: &mut crate::runtime::env::AppValues) {
        values.set(InBar(true));
    }

    /// The commit and cancel commands that ran this pass, from the buttons or from their chords.
    fn finish(mut self, mut cx: Cx<'_, Self>) -> BarOutput {
        while let Some((command, _)) = cx.take_command::<BarCommands>() {
            match command {
                BarCommands::Commit => self.committed = true,
                BarCommands::Cancel => self.cancelled = true,
            }
        }
        BarOutput { response: cx.response.clone(), committed: self.committed, cancelled: self.cancelled }
    }
}

impl Container for Bar<'_> {
    /// The cancel button and the commit button, then the trailing group, on one row as the body's members are, so a
    /// rule in the group is as tall as the row.
    fn end(&mut self, ui: &mut Ui, _: &mut BodyCx<'_>) {
        let (commit_cancel, trailing) = (&self.commit_cancel, &mut self.trailing);
        Row::new().show(ui, |ui, _| {
            if let Some((commit, cancel)) = commit_cancel {
                IconButton::command(IconId::new("ban"), BarCommands::Cancel).label(cancel.clone()).show(ui);
                IconButton::command(IconId::new("check"), BarCommands::Commit).label(commit.clone()).show(ui);
            }
            if let Some(group) = trailing {
                group(ui);
            }
        });
    }
}
