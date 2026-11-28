//! Stories of `PathsPanel`: saved, work and shape paths; a work path being renamed.

use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::{PathKind, PathRow, PathsOutput, PathsPanel};

type Rows = Vec<PathRow<u32>>;

const DOCUMENT: kurbo::Size = kurbo::Size::new(400.0, 300.0);

fn triangle() -> kurbo::BezPath {
    let mut path = kurbo::BezPath::new();
    path.move_to((40.0, 250.0));
    path.line_to((200.0, 40.0));
    path.line_to((360.0, 250.0));
    path.close_path();
    path
}

fn diamond() -> kurbo::BezPath {
    let mut path = kurbo::BezPath::new();
    path.move_to((200.0, 30.0));
    path.line_to((360.0, 150.0));
    path.line_to((200.0, 270.0));
    path.line_to((40.0, 150.0));
    path.close_path();
    path
}

fn mixed_rows() -> Rows {
    vec![
        PathRow::new(1, PathKind::Saved, "Path 1", triangle()),
        PathRow { selected: true, ..PathRow::new(2, PathKind::Work, "Work Path", diamond()) },
        PathRow::new(3, PathKind::ShapePath, "Rectangle 1 Shape Path", triangle()),
    ]
}

fn renaming_rows() -> Rows {
    vec![PathRow { selected: true, ..PathRow::new(2, PathKind::Work, "Work Path", diamond()) }]
}

/// The panel's output with the readout of its latest event, kept in the story's data across passes.
fn with_readout(data: &mut StoryData, shown: PathsOutput<u32>) -> StoryOutput {
    let last = data.note("last", None::<String>);
    if let Some(event) = shown.events.last() {
        *last = Some(format!("{event:?}"));
    }
    let readout = last.as_ref().map_or_else(|| "No event yet".to_owned(), |event| format!("Last event: {event}"));
    StoryOutput::from(shown).readout(readout)
}

#[story(PathsPanel<'static, u32>, covers = [Default], drive = Drive::Never)]
fn paths_with_work_and_shape_path(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = data.value("rows", mixed_rows()).clone();
    let shown = PathsPanel::new(&rows, DOCUMENT).work_name("Path 2").show(ui);
    with_readout(data, shown)
}

#[story(PathsPanel<'static, u32>, covers = [], drive = Drive::Never)]
fn paths_renaming(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = data.value("rows", renaming_rows()).clone();
    let requested = data.note("rename-requested", false);
    let first = !*requested;
    *requested = true;
    let panel = PathsPanel::new(&rows, DOCUMENT).work_name("Path 2");
    let shown = if first { panel.start_rename(2) } else { panel }.show(ui);
    with_readout(data, shown)
}
