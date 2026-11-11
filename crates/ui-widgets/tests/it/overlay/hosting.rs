//! Hosting menus on the overlay contract: one popup per part, toggling on press.

use super::kit::PlainStyle;
use craft_behavior::{HitLevel, HitPriority, PartKey, Sense};
use craft_ui::contract::overlay::{Dismiss, Outside};
use craft_ui::contract::{Common, Component, Cx, MeasureCx, Overlay, OverlayUi, Overlays as _, PartsCx, Props as _, Role, Show as _};
use craft_ui::menu::MenuItem;
use craft_ui::style::StyleCx;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use egui::{Pos2, Rect, Response, Ui, Vec2};
use std::cell::Cell;

const TITLES: Rect = Rect { min: Pos2::new(100.0, 100.0), max: Pos2::new(230.0, 120.0) };
const FIRST: PartKey = PartKey::new("first");
const SECOND: PartKey = PartKey::new("second");
const FIRST_AT: Pos2 = Pos2::new(130.0, 110.0);

/// What the rows of a popup saw.
#[derive(Default)]
struct Rows {
    shown: [Cell<bool>; 2],
    activations: Cell<u32>,
}

/// One component with a toggling part per popup.
#[derive(Component)]
#[component(overlay)]
struct Titles<'a> {
    #[common]
    common: Common,
    on_press: bool,
    rows: &'a Rows,
}

impl Component for Titles<'_> {
    const ROLE: Role = Role::BUTTON;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: TitlesStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        TITLES.size()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        for (key, at, index) in [(FIRST, TITLES.min, 0), (SECOND, TITLES.min + Vec2::new(70.0, 0.0), 1)] {
            let rect = Rect::from_min_size(at, Vec2::new(60.0, 20.0));
            let part = cx.part(key, rect).sense(Sense::CLICK | Sense::HOVER).priority(HitPriority::new(HitLevel::Body, 0)).toggles_at("menu", index);
            if self.on_press {
                part.on_press();
            }
        }
    }

    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        for (key, index) in [(FIRST, 0), (SECOND, 1)] {
            let popup = cx.popup("menu").at(index);
            let overlay = Overlay::menu().dismiss(Dismiss::new().outside(Outside::Absorb).on_activate(true)).anchor(ui.anchor_part(key));
            let rows = self.rows;
            let shown = ui.show(popup, overlay, |ui, _| {
                if MenuItem::new("Row").show(ui).ran {
                    rows.activations.set(rows.activations.get() + 1);
                }
            });
            rows.shown[index].set(shown.is_some());
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

fn show_titles<'a>(rows: &'a Rows, on_press: bool) -> impl FnMut(&mut Ui) + 'a {
    move |ui| {
        rows.shown[0].set(false);
        rows.shown[1].set(false);
        Titles { common: Common::new(), on_press, rows }.id_salt("titles").show_at(ui, TITLES);
    }
}

#[trial]
fn a_part_that_toggles_on_press_opens_on_the_press_and_a_release_over_a_row_picks_it() {
    let (mut frames, rows) = (Frames::new(), Rows::default());
    let mut show = show_titles(&rows, true);
    frames.idle(3, &mut show);
    frames.frame(vec![moved(FIRST_AT)], &mut show);
    frames.frame(vec![press(FIRST_AT)], &mut show);
    frames.idle(2, &mut show);
    assert!(rows.shown[0].get(), "the popup opened while the button is still down");
    // The row hangs below the part, from its start edge.
    let row = FIRST_AT + Vec2::new(0.0, 30.0);
    frames.frame(vec![moved(row)], &mut show);
    frames.frame(vec![release(row)], &mut show);
    frames.idle(2, &mut show);
    assert_eq!(rows.activations.get(), 1, "the release over the row picked it");
}
