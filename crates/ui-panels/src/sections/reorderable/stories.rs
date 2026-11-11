//! Stories of `ReorderableSections`.

use super::{ReorderableSections, SectionSpec};
use crate::contract::Bind;
use crate::sections::sample::body;
use crate::sections::{SectionKey, SectionLayout};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

const IDS: [(&str, &str); 4] = [("albums", "Albums"), ("folders", "Folders"), ("keywords", "Keywords"), ("map", "Map")];

#[story(ReorderableSections<'static>, covers = [Default], drive = Drive::Never)]
fn reorderable_sections(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let specs: Vec<SectionSpec> = IDS.iter().map(|(id, title)| SectionSpec::new(*id, *title)).collect();
    let initial = SectionLayout::new(specs.iter().map(|spec| spec.key.clone()));
    let layout = Bind::bound(data.value("layout", initial));
    ReorderableSections::new("Sidebar", layout, &specs).show(ui, |ui, _: &SectionKey, _| body(ui)).into()
}
