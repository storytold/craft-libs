//! A footer row: a row of view modes, a thumbnail slider and a footer bar narrows its slider to leave the bar its
//! actions.

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

/// The slider's rect and the rects of the footer bar's actions, in a footer row `width` wide.
fn footer(width: f32) -> (Rect, Vec<Rect>) {
    let seen = Rc::new(RefCell::new((Rect::NOTHING, Vec::new())));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let modes = [Entry::new(0u8, "List").icon(IconId::new("list")), Entry::new(1, "Icons").icon(IconId::new("grid"))];
        let mut zoom = 1.0;
        Row::new().fill_width().align(RowAlign::End).show(ui, |ui, _| {
            Segmented::new("View", Bind::given(0u8), &modes).show(ui);
            let slider = ThumbnailSlider::new("Size", Bind::bound(&mut zoom), TileZoom::Ladder).show(ui).response.rect;
            let actions = ["new_folder", "trash", "more"].map(|icon| FooterAction::new(IconId::new(icon), icon));
            let bar = FooterBar::actions(actions).show_footer(ui);
            *out.borrow_mut() = (slider, bar.responses.iter().map(|response| response.rect).collect());
        });
    });
    fixture.room(Vec2::new(width, 60.0)).settle();
    seen.borrow().clone()
}

#[trial]
fn a_footer_row_narrows_its_slider_to_leave_the_footer_bar_its_actions() {
    let (wide, _) = footer(900.0);
    let (slider, actions) = footer(300.0);
    let last = actions.last().copied().unwrap_or(Rect::NOTHING);
    assert!(last.max.x <= 300.0 + 0.5, "the last action ends at {} past a 300-point row", last.max.x);
    assert!(slider.width() < wide.width() - 0.5, "the slider kept its width {} in a narrow row", slider.width());
    assert!(slider.max.x <= actions[0].min.x + 0.5, "the slider {slider:?} runs into the actions {actions:?}");
}
