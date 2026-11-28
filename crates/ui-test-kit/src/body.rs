//! A container for the body trials: its props come from the trial, and its header and footer fill a block of
//! the size given.

use crate::contract::PlainStyle;
use craft_ui::contract::{BodyCx, Common, Component, Container, ContainerProps, Cx, Role};
use craft_ui::style::StyleCx;
use craft_ui::testing::layout::allocate;
use egui::{Response, Ui, Vec2};

/// A container whose header and footer are blocks of a fixed size around its body.
#[derive(craft_ui::contract::Component)]
#[component(container)]
pub struct Slab {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    header: Vec2,
    footer: Vec2,
}

impl Slab {
    /// A slab with the trial's `props`.
    pub fn new(props: ContainerProps) -> Self {
        Slab { common: Common::new(), props, header: Vec2::new(100.0, 24.0), footer: Vec2::new(100.0, 16.0) }
    }
}

impl Component for Slab {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: SlabStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Slab {
    fn header(&mut self, ui: &mut Ui, _: &mut BodyCx<'_>) {
        allocate(ui, self.header);
    }

    fn footer(&mut self, ui: &mut Ui, _: &mut BodyCx<'_>) {
        allocate(ui, self.footer);
    }
}
