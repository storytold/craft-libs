//! Stories of `StatusReadout`.

use super::{StatusItem, StatusReadout, StatusValues};
use crate::bars::{Bar, BarKind};
use crate::contract::{Bind, ShowContainer as _};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Size};
use egui::{Pos2, Rect, Ui};
use std::time::Duration;

/// The document the readout zooms, in document units.
const DOCUMENT: Size = Size::new(2000.0, 1500.0);

fn values() -> StatusValues {
    StatusValues {
        document_sizes: Some((26_100_000, 31_400_000)),
        efficiency: Some(100),
        timing: Some(Duration::from_millis(400)),
        layer_count: Some(7),
        ..StatusValues::default()
    }
}

#[story(StatusReadout<'static>, covers = [], drive = Drive::Never)]
fn status_readout(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let values = values();
    let canvas = Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0));
    let view = data.value("view", CanvasView::new(canvas, Point::new(1000.0, 750.0), 0.6667));
    Bar::new("Status", BarKind::Status)
        .show(ui, |ui, _| {
            StatusReadout::new(Bind::bound(view), DOCUMENT, StatusItem::DocumentSizes, &values).show(ui);
        })
        .output
        .into()
}
