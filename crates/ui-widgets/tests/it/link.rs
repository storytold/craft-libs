//! A link opens its URL in the same tab on a plain click and in a new tab on a click with a modifier held, as a
//! middle click does.

use craft_ui::buttons::Link;
use craft_ui::contract::Show as _;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, pointer, trial};
use egui::{Modifiers, Rect, pos2, vec2};
use std::cell::Cell;

#[trial]
fn a_modified_click_opens_the_link_in_a_new_tab() {
    let mut frames = Frames::new();
    let shown = Cell::new(Rect::NOTHING);
    let mut show = |ui: &mut egui::Ui| {
        let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 40.0));
        shown.set(in_region(ui, region, |ui| Link::new("Shortcuts").url("https://example.com").show(ui).rect));
    };
    frames.frame(Vec::new(), &mut show);
    let at = shown.get().center();
    pointer::click(&mut frames, at, &mut show);
    frames.hold(Modifiers::COMMAND);
    pointer::click(&mut frames, at, &mut show);
    let tabs: Vec<bool> = frames.opened().iter().map(|(_, new_tab)| *new_tab).collect();
    assert_eq!(tabs, [false, true], "a plain click opens in the same tab, a Cmd/Ctrl click in a new one");
}
