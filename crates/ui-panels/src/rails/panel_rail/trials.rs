//! Trials of `PanelRail`: a short window, the content mark and the icons' responses.
#![allow(clippy::panic, clippy::expect_used, reason = "trials assert by panicking")]

use super::PanelRail;
use crate::buttons::Button;
use crate::contract::Show as _;
use crate::rails::tool::stories::{StoryTools, tools};
use crate::runtime::commands::CommandState;
use crate::testing::pointer::{moved, wheel};
use crate::testing::relations::{above, grows_with};
use crate::testing::{Fixture, trial};
use egui::{Rect, Shape, Vec2};
use std::cell::Cell;
use std::rc::Rc;

/// The bounds of the node labelled `label`.
fn rect(fixture: &Fixture, label: &str) -> Rect {
    fixture.by_label(label).unwrap_or_else(|| panic!("{label} is shown")).rect
}

/// When the window is too short for every icon, the footer stays in view at the bottom and the icons scroll in the
/// room above it, down to a room smaller than one scroll region's usual least height.
#[trial]
fn in_a_short_window_the_icons_scroll_above_the_footer_that_stays_in_view() {
    for height in [160.0, 100.0] {
        let mut fixture = Fixture::new(|ui| {
            let panels = tools(&[0, 1, 2, 3, 4]);
            PanelRail::new("Panels", &panels).show_rail(ui, |ui| {
                Button::new("Foot").show(ui);
            });
        });
        fixture.room(Vec2::new(48.0, height)).settle();
        let foot = rect(&fixture, "Foot");
        assert!(foot.bottom() <= height, "{height}: the footer is in the room: {foot:?}");
        let first = rect(&fixture, "Move");
        assert!(first.top() < foot.top(), "{height}: the first icon is above the footer: {first:?}");
        assert!(rect(&fixture, "Clone Stamp").bottom() > foot.top(), "{height}: the last icon starts out of view");

        fixture.frame(vec![moved(first.center())]);
        for _ in 0..10 {
            fixture.frame(vec![wheel(-120.0)]);
        }
        fixture.settle();
        let last = rect(&fixture, "Clone Stamp");
        assert!(grows_with(first.top(), last.top()), "{height}: the last icon is in view after scrolling: {last:?}");
        assert!(above(last, foot), "{height}: the last icon is above the footer after scrolling: {last:?}");
        assert_eq!(rect(&fixture, "Foot"), foot, "{height}: the footer stays put");
    }
}

/// The dots painted inside the Move icon while its panel's command is marked and `open` or closed, and how many item
/// responses the rail reported.
fn marked_move(open: bool) -> (usize, usize) {
    let items = Rc::new(Cell::new(0));
    let reported = Rc::clone(&items);
    let mut fixture = Fixture::new(move |ui| {
        let panels = tools(&[0, 1, 2]);
        let out = PanelRail::new("Panels", &panels).show_rail(ui, |_| {});
        reported.set(out.items.len());
    });
    fixture.command_state(StoryTools::Move, CommandState::new().marked(true).checked(open)).room(Vec2::new(48.0, 200.0)).settle();
    let icon = rect(&fixture, "Move");
    let dots = fixture.frame_shapes().iter().filter(|shape| matches!(shape, Shape::Circle(dot) if icon.contains(dot.center))).count();
    (dots, items.get())
}

/// A marked panel's icon shows the mark while the panel is closed, not while it is open, and the rail reports one
/// response per icon for the host's menus and drags.
#[trial]
fn a_marked_panel_icon_shows_its_mark_only_while_the_panel_is_closed_and_each_icon_reports_its_response() {
    let (closed, items) = marked_move(false);
    let (open, _) = marked_move(true);
    assert!(closed > open, "the mark shows while the panel is closed ({closed}) and not while open ({open})");
    assert_eq!(items, 3, "one response per icon");
}

/// The rails set the glyph role of their buttons: the tool rail's icons are larger than a lone icon button's and rest
/// in a quieter ink than they show under the pointer; the panel rail's keep the lone button's side and rest quieter
/// than it.
#[trial]
fn the_rails_show_their_buttons_icons_in_their_glyph_roles() {
    use crate::buttons::IconButton;
    use crate::contract::Props as _;
    use crate::rails::{LastUsed, ToolRail, ToolSlot};
    use craft_behavior::IconId;
    let glyphs = |rail: Option<bool>| {
        let mut fixture = Fixture::new(move |ui| {
            let panels = tools(&[0, 1]);
            match rail {
                Some(true) => {
                    let slots = vec![ToolSlot::new(0, panels.clone())];
                    ToolRail::new("Tools", &slots, &0, &LastUsed::default()).show_rail(ui, |_, _| {});
                }
                Some(false) => {
                    PanelRail::new("Panels", &panels).show_rail(ui, |_| {});
                }
                None => {
                    IconButton::new(IconId::new("move")).label("Alone").show(ui);
                }
            }
        });
        fixture.settle();
        let seen = fixture.styles_of(std::any::type_name::<IconButton<'static>>());
        let glyph = seen.first().and_then(|seen| seen.looks.icons.first().cloned());
        glyph.unwrap_or_else(|| panic!("an icon button showed ({rail:?})"))
    };
    let (alone, tool, panel) = (glyphs(None), glyphs(Some(true)), glyphs(Some(false)));
    assert!(tool.side > alone.side, "a tool rail icon ({}) is larger than a lone one ({})", tool.side, alone.side);
    assert!(tool.color.off.rest != tool.color.off.hover, "a tool rail icon rests quieter than under the pointer");
    assert_eq!(panel.side, alone.side, "a panel rail icon keeps the control glyph");
    assert!(panel.color.off.rest != alone.color.off.rest, "a panel rail icon rests quieter than a lone one");
}
