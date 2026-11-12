//! Stories of `ReorderableSections`.

use super::{ReorderableSections, SectionButton, SectionSpec};
use crate::contract::Bind;
use crate::sections::sample::body;
use crate::sections::{SectionKey, SectionLayout};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::IconId;
use egui::Ui;

/// Each section's key, title and row count.
const IDS: [(&str, &str, usize); 4] = [("albums", "Albums", 2), ("folders", "Folders", 4), ("keywords", "Keywords", 0), ("map", "Map", 3)];

#[story(ReorderableSections<'static>, covers = [Default], drive = Drive::Never)]
fn reorderable_sections(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let specs: Vec<SectionSpec> = IDS
        .iter()
        .map(|(id, title, count)| {
            let spec = SectionSpec::new(*id, *title).count(*count).empty_hint(format!("No {} yet", title.to_lowercase()));
            if *id == "albums" { spec.button(SectionButton::new("albumNew", IconId::new("plus"), "Create Album")) } else { spec }
        })
        .collect();
    let mut initial = SectionLayout::new(specs.iter().map(|spec| spec.key.clone()));
    initial.set_folded(&SectionKey::new("folders"), true);
    let layout = Bind::bound(data.value("layout", initial));
    ReorderableSections::new("Sidebar", layout, &specs)
        .show(ui, |ui, key: &SectionKey, _| {
            if IDS.iter().any(|(id, _, count)| *id == key.as_str() && *count > 0) {
                body(ui);
            }
        })
        .into()
}
