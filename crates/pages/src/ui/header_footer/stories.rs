//! Stories of the header and footer zones.

use super::{HeaderFooterZones, Zone, ZoneAt, ZoneBands};
use crate::ui::fixtures::{INK, PAPER, demo_placed};
use craft_behavior::CanvasView;
use craft_behavior::kurbo::Point;
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The header and footer band heights and the sides' inset of the demo page, points.
const BANDS: ZoneBands = ZoneBands::new(72.0, 72.0, [50.0, 50.0]);

/// The document point the view is centred on.
const CENTRE: Point = Point::new(306.0, 396.0);

#[story(HeaderFooterZones<'static>, covers = [Default], drive = Drive::Never)]
fn zone_being_edited(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = craft_ui::testing::stage(ui);
    let view = CanvasView::new(room, CENTRE, 1.0);
    let placed = demo_placed(1, view.to_screen(Point::ZERO));
    let bands = |_: usize| BANDS;
    let text = |at: ZoneAt| if at.zone == Zone::new(false, 1) { "Annual report".to_owned() } else { String::new() };
    Canvas::new("Pages")
        .view(view)
        .show_at(ui, room, |ui, body| {
            HeaderFooterZones::new("Zones", &placed, &bands, PAPER, INK).text(text).editing(ZoneAt::new(0, Zone::new(true, 2))).show_at(ui, body.rect)
        })
        .inner
        .map(StoryOutput::from)
        .unwrap_or_default()
}
