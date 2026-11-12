//! A footer row: a row of view modes, a thumbnail slider and a footer bar narrows its slider to leave the bar its
//! actions; a footer bar with a start group shows it at the start edge and the rest at the end edge.

use craft_behavior::IconId;
use craft_ui::choice::Segmented;
use craft_ui::contract::{Bind, Entry, RowAlign, Show as _, ShowContainer as _};
use craft_ui::layout_components::Row;
use craft_ui::sections::{FooterAction, FooterBar};
use craft_ui::testing::{Fixture, trial};
use craft_ui::tile_grid::{ThumbnailSlider, TileZoom};
use egui::{Rect, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// The view modes' rect, the slider's rect and the rects of the footer bar's actions, in a footer row `width` wide.
fn footer(width: f32) -> (Rect, Rect, Vec<Rect>) {
    let seen = Rc::new(RefCell::new((Rect::NOTHING, Rect::NOTHING, Vec::new())));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let modes = [Entry::new(0u8, "List").icon(IconId::new("list")), Entry::new(1, "Icons").icon(IconId::new("grid"))];
        let mut zoom = 1.0;
        Row::new().fill_width().align(RowAlign::End).show(ui, |ui, _| {
            let modes = Segmented::new("View", Bind::given(0u8), &modes).show(ui).response.rect;
            let slider = ThumbnailSlider::new("Size", Bind::bound(&mut zoom), TileZoom::Ladder).show(ui).response.rect;
            let actions = ["new_folder", "trash", "more"].map(|icon| FooterAction::new(IconId::new(icon), icon));
            let bar = FooterBar::actions(actions).show_footer(ui);
            *out.borrow_mut() = (modes, slider, bar.responses.iter().map(|response| response.rect).collect());
        });
    });
    fixture.room(Vec2::new(width, 60.0)).settle();
    seen.borrow().clone()
}

#[trial]
fn a_footer_row_narrows_its_slider_to_leave_the_footer_bar_its_actions() {
    let (modes, wide, actions) = footer(900.0);
    let span = |rects: &[Rect]| rects.last().map_or(0.0, |last| last.max.x) - rects.first().map_or(0.0, |first| first.min.x);
    // A row with room for the modes, the actions and half the slider's natural length, with a member gap at each
    // seam: the track gives way to about a control's height, as a media bin's footer in a narrow panel needs.
    let gap = wide.min.x - modes.max.x;
    let width = (modes.width() + wide.width() / 2.0 + span(&actions) + 3.0 * gap).ceil();
    let (_, slider, actions) = footer(width);
    let last = actions.last().copied().unwrap_or(Rect::NOTHING);
    assert!(last.max.x <= width + 0.5, "the last action ends at {} past a {width}-point row", last.max.x);
    assert!(slider.width() < wide.width() - 0.5, "the slider kept its width {} in a narrow row", slider.width());
    assert!(slider.max.x <= actions[0].min.x + 0.5, "the slider {slider:?} runs into the actions {actions:?}");
}

/// The rects of a footer bar's four actions whose first two sit at the start edge, in a bar `WIDE` points wide.
fn split(direction: craft_ui::runtime::env::Direction) -> Vec<Rect> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let actions = ["ban", "check", "copy", "trash"].map(|icon| FooterAction::new(IconId::new(icon), icon));
        let bar = FooterBar::actions(actions).at_start(2).show_footer(ui);
        *out.borrow_mut() = bar.responses.iter().map(|response| response.rect).collect();
    });
    fixture.direction(direction).room(Vec2::new(WIDE, 60.0)).settle();
    seen.borrow().clone()
}

const WIDE: f32 = 400.0;

/// The start group sits at the start edge and the rest at the end edge, the room left between them; right to left
/// the groups trade sides.
#[trial]
fn the_start_and_end_groups_of_a_footer_bar_sit_at_their_edges() {
    use craft_ui::runtime::env::Direction;
    let ltr = split(Direction::Ltr);
    assert_eq!(ltr.len(), 4);
    let (pair, between) = (ltr[1].min.x - ltr[0].max.x, ltr[2].min.x - ltr[1].max.x);
    assert!(between > pair + WIDE / 4.0, "the room lies between the groups: {ltr:?}");
    assert!(ltr[0].min.x < WIDE / 4.0 && ltr[3].max.x > WIDE * 3.0 / 4.0, "the groups reach the edges: {ltr:?}");
    let rtl = split(Direction::Rtl);
    assert!(rtl[0].min.x > rtl[2].max.x, "right to left the start group sits at the right: {rtl:?}");
}
