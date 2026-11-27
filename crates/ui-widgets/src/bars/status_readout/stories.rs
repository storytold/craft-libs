//! Stories of `StatusReadout`.

use super::{StatusItem, StatusReadout, StatusValues};
use crate::bars::{Bar, BarKind};
use crate::contract::ShowContainer as _;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::kurbo::Size;
use egui::Ui;
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
    let (zoom, zoom_at) = data.value_at("zoom", 0.6667);
    Bar::new("Status", BarKind::Status)
        .show(ui, |ui, _| {
            StatusReadout::new(zoom_at.bind(zoom), DOCUMENT, StatusItem::DocumentSizes, &values).show(ui);
        })
        .output
        .into()
}
