//! Stories of `HistoryPanel`: states with redo rows dimmed, snapshots, the source column, empty and disabled.

use super::{HistoryFooter, HistoryKey, HistoryPanel, HistoryRow};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The states of a short edit, the last two undone, and two snapshots.
fn rows() -> Vec<HistoryRow> {
    vec![
        HistoryRow::snapshot(1, "Snapshot 1"),
        HistoryRow::state(1, "Open"),
        HistoryRow::state(2, "Brush Tool"),
        HistoryRow::state(3, "Gaussian Blur").redo(true),
        HistoryRow::state(4, "Fill").redo(true),
        HistoryRow::snapshot(2, "Before Crop"),
    ]
}

/// Shows the panel over `rows`; the oldest state is never deleted.
fn panel(ui: &mut Ui, rows: &[HistoryRow], current: Option<HistoryKey>, source: Option<Option<HistoryKey>>, enabled: bool) -> StoryOutput {
    let mut panel = HistoryPanel::new(rows, current)
        .can_delete(|key| *key != HistoryKey::State(1))
        .footer(HistoryFooter { delete: true, new_snapshot: true, new_document: true })
        .enabled(enabled);
    if let Some(source) = source {
        panel = panel.source(source);
    }
    let out = panel.show(ui);
    let readout = format!("{:?}", out.events);
    StoryOutput::from(out).readout(readout)
}

#[story(HistoryPanel<'static>, covers = [], drive = Drive::Never, title = "Snapshots and states")]
fn history_panel(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    panel(ui, &rows(), Some(HistoryKey::State(2)), None, true)
}

#[story(HistoryPanel<'static>, covers = [], drive = Drive::Never, title = "With the source column")]
fn history_panel_source(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    panel(ui, &rows(), Some(HistoryKey::State(2)), Some(Some(HistoryKey::Snapshot(1))), true)
}

#[story(HistoryPanel<'static>, covers = [], drive = Drive::Never, title = "Empty")]
fn history_panel_empty(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    panel(ui, &[], None, None, true)
}

#[story(HistoryPanel<'static>, covers = [], drive = Drive::Never, title = "Disabled")]
fn history_panel_disabled(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    panel(ui, &rows(), Some(HistoryKey::State(2)), Some(None), false)
}
