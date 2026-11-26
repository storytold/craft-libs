//! A container for the label-column trials: its labelled rows share the widest label.

use super::kit::PlainStyle;
use craft_behavior::Sense;
use craft_ui::contract::{Common, Component, Container, ContainerProps, Cx, Props as _, Role};
use craft_ui::style::StyleCx;
use egui::Response;

/// A container with the props the trial gives it and a caller's body.
#[derive(craft_ui::contract::Component)]
#[component(container)]
pub struct Group {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl Group {
    pub fn new(props: ContainerProps) -> Self {
        Group { common: Common::new(), props }.label("Group")
    }
}

impl Component for Group {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: GroupStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Group {}
