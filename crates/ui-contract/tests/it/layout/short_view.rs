//! A scroll body whose rect is shorter than its padding: its view has no room, so its scroll bar paints nothing.

use craft_ui::contract::{ContainerProps, Pad, Props as _, Scroll, Show as _, ShowContainer as _, Sizing};
use craft_ui::testing::{Frames, pointer, trial};
use craft_ui_test_kit::body::Slab;
use craft_ui_test_kit::contract::{Probe, Seen};
use egui::{Pos2, Rect, Shape, Ui, Vec2};
use std::cell::RefCell;

/// The container's rect: shorter than the padding above and below its view.
const AT: Rect = Rect { min: Pos2::new(20.0, 20.0), max: Pos2::new(180.0, 26.0) };

/// The padded container that scrolls along y under `scroll`'s bar, holding a probe that paints nothing and is far
/// taller than the container.
fn short(ui: &mut Ui, seen: &RefCell<Seen>, scroll: Scroll) {
    let props = ContainerProps::new().pad(Pad::Control).sizing(Sizing::Fill).scroll(scroll);
    Slab::new(props).id_salt("short").show_at(ui, AT, |ui, _| {
        Probe::new(seen).size(Vec2::new(40.0, 400.0)).sizing(Sizing::FillWidth).id_salt("content").show(ui);
    });
}

#[trial]
fn a_scroll_body_shorter_than_its_padding_paints_no_bar() {
    for (name, scroll) in [("bar", Scroll::vertical()), ("overlay bar", Scroll::vertical().overlay())] {
        let seen = RefCell::new(Seen::default());
        let mut show = |ui: &mut Ui| short(ui, &seen, scroll);
        let mut frames = Frames::new();
        frames.frame(vec![pointer::moved(AT.center())], &mut show);
        frames.idle(4, &mut show);
        let painted: Vec<Rect> = frames.frame_shapes(&mut show).iter().map(Shape::visual_bounding_rect).filter(|bounds| bounds.is_finite()).collect();
        assert!(painted.is_empty(), "{name}: the bar paints nothing: {painted:?}");
    }
}
