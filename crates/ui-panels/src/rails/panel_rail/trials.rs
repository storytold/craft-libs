//! Trials of `PanelRail` in a short window.

use super::PanelRail;
use crate::buttons::Button;
use crate::contract::Show as _;
use crate::rails::tool::stories::tools;
use crate::testing::pointer::{moved, wheel};
use crate::testing::{Fixture, trial};
use egui::{Rect, Vec2};

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
        assert!(last.top() >= first.top() - 0.5, "{height}: the last icon is in view after scrolling: {last:?}");
        assert!(last.bottom() <= foot.top() + 0.5, "{height}: the last icon is above the footer after scrolling: {last:?}");
        assert_eq!(rect(&fixture, "Foot"), foot, "{height}: the footer stays put");
    }
}
