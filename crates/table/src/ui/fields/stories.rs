//! Stories of `FieldListPane`: fields in every area and the Σ Values row.

use super::{FieldArea, FieldListPane, PivotField, ValuesRow};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn fields() -> Vec<PivotField> {
    let names = [("Region", false), ("Product", false), ("Year", false), ("Sales", true), ("Units", true)];
    names.into_iter().map(|(name, numeric)| PivotField::new(name, numeric).samples(vec!["North".to_owned(), "South".to_owned()])).collect()
}

fn areas() -> Vec<(FieldArea, Vec<String>)> {
    let names = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect::<Vec<_>>();
    vec![
        (FieldArea::Filters, names(&["Year"])),
        (FieldArea::Columns, names(&["Product"])),
        (FieldArea::Rows, names(&["Region"])),
        (FieldArea::Values, names(&["Sales", "Units"])),
    ]
}

#[story(FieldListPane<'static>, covers = [Default], drive = Drive::Never)]
fn field_list_pane(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let (fields, areas) = (fields(), areas());
    FieldListPane::new(&fields, &areas).values_row(Some(ValuesRow::new(FieldArea::Columns, 1))).show(ui).into()
}
