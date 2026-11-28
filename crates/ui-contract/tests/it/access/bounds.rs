//! `runtime::access::bounds`: the rect automation clicks hides what the access tree hides.

use craft_ui::contract::{Common, Component, Container, ContainerProps, Cx, Mount, Overlay, Props as _, Role, Show as _, ShowContainer as _};
use craft_ui::runtime::access::bounds;
use craft_ui::style::StyleCx;
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Rect, Response, Ui, Vec2};
use std::cell::Cell;

/// A button-like leaf with a fixed size.
#[derive(craft_ui::contract::Component)]
struct Apply {
    #[common]
    common: Common,
}

impl Component for Apply {
    const ROLE: Role = Role::BUTTON;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: ApplyStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::new(80.0, 24.0))
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// A modal layer with an empty body.
#[derive(craft_ui::contract::Component)]
#[component(container)]
struct Modal {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl Component for Modal {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: ModalStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::new(120.0, 60.0))
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Modal {}

/// A shown button answers its last pass's rect; a button not shown this pass answers `None`; the same button behind
/// an open modal answers `None`, as the access tree hides it.
#[trial]
fn bounds_of_a_part_behind_a_modal_is_none() {
    let mut frames = Frames::new();
    let shown: Cell<Option<(egui::Id, Rect)>> = Cell::new(None);
    let button = |ui: &mut Ui| {
        let response = Apply { common: Common::new() }.label("Apply").id_salt("apply").show(ui);
        shown.set(Some((response.id, response.rect)));
    };

    for _ in 0..2 {
        frames.frame(vec![], button);
    }
    let Some((id, rect)) = shown.get() else { panic!("the button was shown") };
    assert!(rect.width() > 0.0 && rect.height() > 0.0, "the button has area: {rect:?}");
    assert_eq!(bounds(&frames.ctx, id), Some(rect), "a shown button answers its rect");

    frames.frame(vec![], |_| {});
    assert_eq!(bounds(&frames.ctx, id), None, "a button not shown this pass has no rect");

    // The modal registers at the end of a pass and egui takes its layer at the next pass's start, so a few frames
    // settle it before the check.
    for _ in 0..4 {
        frames.frame(vec![], |ui| {
            button(ui);
            let props = ContainerProps::new().mount(Mount::Layer(Overlay::modal()));
            Modal { common: Common::new(), props }.id_salt("modal").show(ui, |_, _| {});
        });
    }
    let Some((behind, _)) = shown.get() else { panic!("the button was shown") };
    assert_eq!(behind, id, "the button keeps its id under the modal");
    assert!(frames.ctx.read_response(id).is_some(), "the button was shown this pass");
    assert_eq!(bounds(&frames.ctx, id), None, "a button behind the open modal has no rect");
}
