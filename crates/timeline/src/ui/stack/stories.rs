//! Stories of `TrackStack`: no tracks, one track and many.

use super::{TrackRow, TrackStack, Tracks};
use crate::model::{TrackId, TrackInfo, TrackKind};
use craft_ui::contract::CollectionEvent;
use craft_ui::contract::ShowCollection as _;
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn track(id: u64, name: &str) -> TrackInfo<'_> {
    TrackInfo {
        id: TrackId(id),
        kind: TrackKind::Video,
        name,
        height: None,
        depth: 0,
        label: None,
        locked: Some(false),
        targeted: Some(true),
        sync_lock: Some(false),
        enabled: Some(true),
        muted: None,
        solo: None,
    }
}

fn stack(ui: &mut Ui, names: &[&str]) -> StoryOutput {
    let tracks: Vec<TrackInfo<'_>> = names.iter().enumerate().map(|(at, name)| track(at as u64 + 1, name)).collect();
    let words = Env::of(ui).words::<crate::ui::view::TimelineWords>();
    let source = Tracks::new(&tracks);
    let none: [crate::model::ClipInfo<'_>; 0] = [];
    let out = TrackStack::new("Tracks", None, &words).tracks(&tracks).show_items(ui, &source, &mut |key, icx| {
        let at = tracks.iter().position(|t| t.id == *key).unwrap_or(0);
        let lane = crate::ui::clips::ClipLane::new(&none);
        TrackRow::new(&tracks[at], icx.cells().to_vec(), lane, false, false)
    });
    // The strip and the header act through the row's event: an activation of either counts as a click.
    let acted = out.events.iter().any(|event| matches!(event, CollectionEvent::Item(_, row) if row.label_menu || row.height.is_some()));
    let mut facts = StoryOutput::from(out);
    facts.clicked |= acted;
    facts
}

#[story(TrackStack, covers = [Empty], drive = Drive::Never)]
fn track_stack_empty(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    stack(ui, &[])
}

#[story(TrackStack, covers = [One], drive = Drive::Never)]
fn track_stack_one(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    stack(ui, &["V1"])
}

#[story(TrackStack, covers = [Default, Many], drive = Drive::Never)]
fn track_stack_many(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    stack(ui, &["V3", "V2", "V1", "A1", "A2"])
}
