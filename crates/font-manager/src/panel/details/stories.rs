//! Stories of the details sheet and its style rows.

use craft_ui::contract::ShowCollection as _;
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Drive, StoryData, StoryOutput, show_as_only_item, story};
use egui::Ui;

use super::rows::{Action, Lines, StyleLine, StyleList, StyleRow};
use super::{DetailsSheet, DetailsWords};
use crate::panel::app::Detail;
use crate::panel::faces::Owner;
use crate::panel::fixtures::{SharedPanel, story_catalog, story_panel};
use crate::panel::samples::PANGRAM;
use crate::panel::specimen::SampleFace;

/// The sample size of the stories, in points.
const POINTS: f32 = 24.0;

/// A panel that shows the sheet of the first catalogue family, or of the first installed one.
fn panel_with(installed: bool) -> SharedPanel {
    let mut panel = story_panel();
    panel.detail =
        if installed { panel.installed.first().map(|family| Detail::Installed(family.name.clone(), family.source)) } else { Some(Detail::Family(1)) };
    SharedPanel::new(panel)
}

fn sheet(ui: &mut Ui, data: &mut StoryData, installed: bool) -> StoryOutput {
    let shared = data.value::<Option<SharedPanel>>("panel", None).get_or_insert_with(|| panel_with(installed)).clone();
    let mut panel = shared.lock();
    DetailsSheet::new(&mut panel).show(ui).into()
}

#[story(DetailsSheet<'static>, covers = [], drive = Drive::Never)]
fn family_sheet(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    sheet(ui, data, false)
}

#[story(DetailsSheet<'static>, covers = [], drive = Drive::Never)]
fn installed_sheet(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    sheet(ui, data, true)
}

fn lines(count: usize) -> Vec<StyleLine> {
    let catalog = story_catalog();
    let face = catalog.styles("Hack").into_iter().next();
    (0..count)
        .map(|n| StyleLine {
            label: format!("Style {n}"),
            face: face.clone().map(|face| (Owner::Installed, face)),
            action: if n % 2 == 0 { Action::Install(format!("ofl/hack/Style{n}.ttf")) } else { Action::None },
        })
        .collect()
}

fn list(ui: &mut Ui, count: usize) -> StoryOutput {
    let lines = lines(count);
    let catalog = story_catalog();
    let words = Env::of(ui).words::<DetailsWords>();
    let source = Lines(&lines);
    StyleList::new("Styles", words.no_styles.clone())
        .show_items(ui, &source, &mut |key, _| {
            let line = &lines[*key];
            let face = match &line.face {
                Some((owner, face)) => SampleFace::Face { catalog: &catalog, owner: *owner, face },
                None => SampleFace::Interface,
            };
            StyleRow::new(line, "Hack", PANGRAM, POINTS, face, &words)
        })
        .into()
}

#[story(StyleList, covers = [Empty], drive = Drive::Never)]
fn style_list_empty(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    list(ui, 0)
}

#[story(StyleList, covers = [One], drive = Drive::Never)]
fn style_list_one(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    list(ui, 1)
}

#[story(StyleList, covers = [Default, Many], drive = Drive::Never)]
fn style_list_many(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    list(ui, 4)
}

#[story(StyleRow<'static>, covers = [Default], drive = Drive::Never)]
fn style_row_installing(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let line = StyleLine { label: "Bold".to_owned(), face: None, action: Action::Running(Some(0.4)) };
    let words = Env::of(ui).words::<DetailsWords>();
    show_as_only_item(ui, StyleRow::new(&line, "Hack", PANGRAM, POINTS, SampleFace::Interface, &words)).into()
}
