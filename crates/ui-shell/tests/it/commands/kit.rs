//! Components and command sets for the command trials.

use craft_behavior::{Metrics, Sense};
use craft_ui::contract::{CommandSet, Common, Component, Container, ContainerProps, Cx, Role};
use craft_ui::runtime::commands::CommandScope;
use craft_ui::style::{ComponentStyle, StyleCx};
use egui::{Response, Vec2};

/// A style with a fixed size.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FixedStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// App-scope commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "trial")]
#[non_exhaustive]
pub enum AppCommand {
    #[command(label = "Go")]
    Go,
    #[command(label = "Save", default = "Cmd+K")]
    Save,
    #[command(label = "Play", default = "F8")]
    Play,
    #[command(label = "Jump", default = "Cmd+Alt+J")]
    Jump,
}

/// Commands of the `trial.pane` scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "pane", scope = "trial.pane")]
#[non_exhaustive]
pub enum PaneCommand {
    #[command(label = "Zoom In", default = "Cmd+Alt+K")]
    ZoomIn,
}

/// Commands of the `trial.dock` scope, whose instances contain panes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "dock", scope = "trial.dock")]
#[non_exhaustive]
pub enum DockCommand {
    #[command(label = "Close", default = "Cmd+Alt+J")]
    Close,
}

pub const PANE: CommandScope = <PaneCommand as CommandSet>::SCOPE;
pub const DOCK: CommandScope = <DockCommand as CommandSet>::SCOPE;

/// A button-like leaf that can report the label and enabled state the pipeline resolved for it.
#[derive(Component)]
pub struct Tap<'a> {
    #[common]
    common: Common,
    on: Option<&'a std::cell::Cell<bool>>,
}

impl<'a> Tap<'a> {
    pub fn new() -> Self {
        Tap { common: Common::new(), on: None }
    }
}

impl Component for Tap<'_> {
    const ROLE: Role = Role::BUTTON;
    type Style = FixedStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: TapStyleKey) -> FixedStyle {
        FixedStyle::default()
    }

    fn measure(&mut self, _: &craft_ui::contract::MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(40.0, 20.0)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        if let Some(on) = self.on {
            on.set(craft_ui::contract::Styled::state(&cx).flags.intersects(craft_ui::style::Flags::ON));
        }
        cx.response.clone()
    }
}

/// A container that is an instance of the `trial.pane` scope.
#[derive(Component)]
#[component(container)]
pub struct Pane {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl Pane {
    pub fn new() -> Self {
        Pane { common: Common::new(), props: ContainerProps::new().command_scope(PANE) }
    }
}

impl Component for Pane {
    const ROLE: Role = Role::GROUP;
    type Style = FixedStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: PaneStyleKey) -> FixedStyle {
        FixedStyle::default()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Pane {}

/// A container that is an instance of the `trial.dock` scope.
#[derive(Component)]
#[component(container)]
pub struct Dock {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl Dock {
    pub fn new() -> Self {
        Dock { common: Common::new(), props: ContainerProps::new().command_scope(DOCK) }
    }
}

impl Component for Dock {
    const ROLE: Role = Role::GROUP;
    type Style = FixedStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: DockStyleKey) -> FixedStyle {
        FixedStyle::default()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Dock {}
