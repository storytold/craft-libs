//! A scroll body with an overlay bar (`Scroll::overlay`): the content keeps the whole view, the bar lies in the
//! container's end padding, and it shows only while the pointer is over the container.

use craft_ui::contract::{ContainerProps, Pad, Props as _, Scroll, Show as _, ShowContainer as _, Sizing};
use craft_ui::runtime::env::{Direction, with_env};
use craft_ui::testing::{Frames, pointer, trial};
use craft_ui_test_kit::body::Slab;
use craft_ui_test_kit::contract::{Probe, Seen};
use egui::{Pos2, Rect, Shape, Ui, Vec2};
use std::cell::RefCell;

/// Half a pixel at the scales the box runs.
const SLACK: f32 = 0.5;

/// The container's rect: shorter than its content, so it scrolls.
const AT: Rect = Rect { min: Pos2::new(20.0, 20.0), max: Pos2::new(180.0, 120.0) };

/// A point outside the container.
const AWAY: Pos2 = Pos2::new(300.0, 300.0);

/// The padded container that scrolls along y under an overlay bar, holding a probe that fills its width and is three
/// times its height.
fn overlay(ui: &mut Ui, seen: &RefCell<Seen>) {
    let props = ContainerProps::new().pad(Pad::Control).sizing(Sizing::Fill).scroll(Scroll::vertical().overlay());
    Slab::new(props).id_salt("overlay").show_at(ui, AT, |ui, _| {
        Probe::new(seen).size(Vec2::new(40.0, AT.height() * 3.0)).sizing(Sizing::FillWidth).id_salt("content").show(ui);
    });
}

/// Whether any of `shapes` lies inside `band` (and paints something).
fn paints_in(shapes: &[Shape], band: Rect) -> bool {
    shapes.iter().map(Shape::visual_bounding_rect).any(|bounds| bounds.is_positive() && band.expand(SLACK).contains_rect(bounds))
}

#[trial]
fn an_overlay_bar_takes_no_room_lies_in_the_end_padding_and_shows_only_under_the_pointer() {
    for direction in [Direction::Ltr, Direction::Rtl] {
        let seen = RefCell::new(Seen::default());
        let mut show = |ui: &mut Ui| with_env(ui, |o| o.direction = Some(direction), |ui| overlay(ui, &seen));
        let mut frames = Frames::new();
        frames.frame(vec![pointer::moved(AWAY)], &mut show);
        frames.idle(4, &mut show);
        let content = seen.borrow().rect;
        let (start, end) = (content.min.x - AT.min.x, AT.max.x - content.max.x);
        assert!(start > 0.0 && (start - end).abs() < SLACK, "{direction:?}: the content keeps the whole view between equal pads: {content:?}");
        let band = match direction {
            Direction::Rtl => Rect::from_x_y_ranges(AT.min.x..=content.min.x, content.min.y.max(AT.min.y)..=AT.max.y),
            _ => Rect::from_x_y_ranges(content.max.x..=AT.max.x, content.min.y.max(AT.min.y)..=AT.max.y),
        };
        let away = frames.frame_shapes(&mut show);
        assert!(!paints_in(&away, band), "{direction:?}: no bar shows while the pointer is away");
        frames.frame(vec![pointer::moved(Pos2::new(content.center().x, AT.center().y))], &mut show);
        frames.idle(2, &mut show);
        let over = frames.frame_shapes(&mut show);
        assert!(paints_in(&over, band), "{direction:?}: the bar shows in the end padding while the pointer is over the container");
    }
}
