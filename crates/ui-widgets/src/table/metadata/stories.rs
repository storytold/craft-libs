//! Stories of `MetadataTable`: grouped fields, the check column, a search with no match and no fields at all.

use super::{MetadataGroup, MetadataRow, MetadataTable};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn groups() -> Vec<MetadataGroup> {
    vec![
        MetadataGroup::new(
            "Camera",
            vec![MetadataRow::new("Make", "FUJIFILM"), MetadataRow::new("Model", "X100V"), MetadataRow::new("Lens", "23 mm f/2")],
        ),
        MetadataGroup::new(
            "Exposure",
            vec![
                MetadataRow::new("Exposure Time", "1/250").key("exif:ExposureTime"),
                MetadataRow::new("Aperture", "f/4"),
                MetadataRow::new("ISO speed", "200"),
                MetadataRow::new("Focal Length", "23 mm"),
            ],
        ),
        MetadataGroup::new(
            "File",
            vec![
                MetadataRow::new("Creator", "A. Rivera").key("dc:creator"),
                MetadataRow::new("Title", "IMG_2041").locked(),
                MetadataRow::new("Software", "Ver 5.20"),
            ],
        ),
    ]
}

#[story(MetadataTable<'static>, covers = [Default], drive = Drive::Never)]
fn metadata_table(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (filter, filter_at) = data.value_at("filter", String::new());
    MetadataTable::new("Metadata", &groups()).filter(filter_at.bind(filter)).show(ui).into()
}

#[story(MetadataTable<'static>, covers = [], drive = Drive::Never)]
fn metadata_table_chooser(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let ((filter, chosen), at) = data.value_at("chooser", (String::new(), vec!["Make".to_owned(), "ISO speed".to_owned()]));
    MetadataTable::new("Metadata", &groups()).filter(at.bind(filter)).choose(at.bind(chosen)).show(ui).into()
}

#[story(MetadataTable<'static>, covers = [Empty], drive = Drive::Never)]
fn metadata_table_no_match(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (filter, filter_at) = data.value_at("filter", "zzz".to_owned());
    MetadataTable::new("Metadata", &groups()).filter(filter_at.bind(filter)).show(ui).into()
}
