//! Stories of the revisions panel.

use super::RevisionsPanel;
use crate::revisions::{Revision, RevisionKind};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn changes() -> Vec<Revision> {
    vec![
        Revision { kind: RevisionKind::Inserted, heading: "Slide 3 inserted".into(), details: vec!["Title: Results".into()] },
        Revision { kind: RevisionKind::Changed, heading: "Slide 5 changed".into(), details: vec!["Text changed".into(), "Picture moved".into()] },
        Revision { kind: RevisionKind::Deleted, heading: "Slide 7 deleted".into(), details: Vec::new() },
    ]
}

#[story(RevisionsPanel<'static>, covers = [Default, Many], drive = Drive::Never)]
fn revisions_changes(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    RevisionsPanel::new("Revisions", "Budget v2.pptx", &changes()).show(ui).into()
}

#[story(RevisionsPanel<'static>, covers = [Empty], drive = Drive::Never)]
fn revisions_none(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    RevisionsPanel::new("Revisions", "Budget v2.pptx", &[]).by_position(true).show(ui).into()
}
