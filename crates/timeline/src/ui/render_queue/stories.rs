//! Stories of the render queue list.

use super::{RenderItemId, RenderQueueItem, RenderQueueList, RenderStatus};
use craft_behavior::Selection;
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;
use std::time::Duration;

fn item(id: u64, status: RenderStatus) -> RenderQueueItem {
    RenderQueueItem::new(RenderItemId::new(id), "Sequence 01", format!("out{id}.mp4"), "Match Source", status)
}

fn queue(ui: &mut Ui, data: &mut StoryData, items: &[RenderQueueItem], running: bool) -> StoryOutput {
    let selection = data.value("selection", Selection::<RenderItemId>::default());
    RenderQueueList::new("Destinations", items, Bind::bound(selection)).running(running).show(ui).into()
}

#[story(RenderQueueList<'static>, covers = [Default, Many], drive = Drive::Never)]
fn render_queue(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let items = [
        item(1, RenderStatus::Ready),
        item(2, RenderStatus::encoding(0.42, Some(Duration::from_secs(75)))),
        item(3, RenderStatus::Done),
        item(4, RenderStatus::Failed("Disk full".to_owned())),
        item(5, RenderStatus::Cancelled),
    ];
    queue(ui, data, &items, true)
}

#[story(RenderQueueList<'static>, covers = [Empty], drive = Drive::Never)]
fn render_queue_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    queue(ui, data, &[], false)
}
