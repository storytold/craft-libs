//! The wheel over a canvas a modal covers.

use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Size};
use craft_ui::canvas::{Canvas, NavigationOptions};
use craft_ui::contract::{Common, Component, Container, ContainerProps, Cx, Mount, Overlay, Props as _, Role, ShowContainer as _};
use craft_ui::style::StyleCx;
use craft_ui::testing::{Fixture, pointer, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Response, Vec2};
use std::cell::Cell;
use std::rc::Rc;

/// A modal layer with an empty body.
#[derive(craft_ui::contract::Component)]
#[component(container, unregistered)]
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

/// A modal opened over a canvas holds the wheel: neither a wheel line nor a press the canvas held from before the
/// modal opened moves the view, plain or eased.
#[trial]
fn the_wheel_over_a_canvas_under_a_modal_moves_nothing() {
    for eased in [false, true] {
        for held in [false, true] {
            let mut options = NavigationOptions::default();
            options.eased_wheel = eased;
            let view = Rc::new(Cell::new(CanvasView::new(egui::Rect::ZERO, Point::new(2000.0, 2000.0), 1.0)));
            let open = Rc::new(Cell::new(false));
            let (shown, modal) = (Rc::clone(&view), Rc::clone(&open));
            let mut fixture = Fixture::new(move |ui| {
                let mut given = shown.get();
                given.rect = ui.available_rect_before_wrap();
                let canvas = Canvas::new("Document").document(Size::new(4000.0, 4000.0)).navigation(options).view(given).show(ui, |_, _| {});
                shown.set(canvas.output.view.unwrap_or(given));
                if modal.get() {
                    let props = ContainerProps::new().mount(Mount::Layer(Overlay::modal()));
                    Modal { common: Common::new(), props }.id_salt("modal").show(ui, |_, _| {});
                }
            });
            fixture.settle();
            let at = view.get().rect.center() + Vec2::new(0.0, 150.0);
            fixture.frame(vec![pointer::moved(at)]);
            let start = view.get();
            fixture.frame(vec![pointer::wheel(-30.0)]);
            fixture.idle(40);
            assert!(view.get().center.y > start.center.y + 1.0, "precondition: the wheel scrolls the bare canvas");
            if held {
                fixture.frame(vec![pointer::press(at)]);
            }
            open.set(true);
            fixture.idle(3);
            let before = view.get();
            fixture.frame(vec![pointer::wheel(-30.0)]);
            fixture.idle(40);
            let after = view.get();
            assert!(
                (after.center.y - before.center.y).abs() < 1e-3 && after.zoom == before.zoom,
                "eased {eased}, held {held}: the view moved under the modal: {before:?} -> {after:?}"
            );
        }
    }
}
