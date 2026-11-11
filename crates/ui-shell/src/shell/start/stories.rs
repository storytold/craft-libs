//! Stories of `StartScreen`.

use super::StartScreen;
use crate::contract::Bind;
use crate::primitives::{LoadFailure, Revision, TextureState, TextureStore, TextureStoreOptions};
use crate::shell::{RecentFile, RecentSort, RecentView, Template, ThumbKey};
use crate::testing::pictures::sample_picture;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;
use std::path::PathBuf;

/// The time the age lines count from, in seconds since the Unix epoch.
const NOW: i64 = 1_800_000_000;
const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;
/// The size of the stored pictures.
const PICTURE: [u32; 2] = [64, 36];

/// The view and sort the stories edit.
#[derive(Clone, Debug, Default, PartialEq)]
struct Choice {
    view: RecentView,
    sort: RecentSort,
}

/// Five files, four with a picture revision (the fourth fails to load) and one without.
fn picture_files() -> Vec<RecentFile> {
    let file = |name: &str, folder: &str, days: i64, thumbnail: Option<Revision>| {
        let mut file = RecentFile::new(PathBuf::from(folder).join(name), name);
        file.opened = Some(NOW - days * DAY);
        file.created = Some(NOW - (days + 30) * DAY);
        file.thumbnail = thumbnail;
        file
    };
    vec![
        file("Poster 2.fig", "/home/ana/Work", 0, Some(Revision::new(1))),
        file("Poster 10.fig", "/home/ana/Work", 2, Some(Revision::new(2))),
        file("Brochure.fig", "/home/ana/Print", 5, Some(Revision::new(3))),
        file("Broken.fig", "/home/ana/Print", 9, Some(Revision::new(4))),
        file("Notes.fig", "/home/ana/Notes", 30, None),
    ]
}

fn picture_templates() -> Vec<Template> {
    let mut a4 = Template::new("a4", "A4", "210 × 297 mm");
    a4.thumbnail = Some(Revision::new(1));
    vec![a4, Template::new("hd", "Full HD", "1920 × 1080 px")]
}

/// Fills the store with the pictures the stories name: ready for revisions 1 to 3 and the A4 template, failed for 4.
fn fill(ui: &Ui) -> TextureStore<ThumbKey> {
    let ctx = ui.ctx();
    let store = TextureStore::<ThumbKey>::in_context(ctx, "start-stories", TextureStoreOptions::default());
    for (file, key) in picture_files().into_iter().take(3).zip(0..) {
        if let Some(revision) = file.thumbnail {
            let _ = store.get_or_put(ctx, ThumbKey::Recent(file.path), revision, || sample_picture(key, PICTURE));
        }
    }
    let _ = store.get_or_put(ctx, ThumbKey::Template("a4".to_owned()), Revision::new(1), || sample_picture(3, PICTURE));
    let broken = ThumbKey::Recent(PathBuf::from("/home/ana/Print").join("Broken.fig"));
    if matches!(store.state(ctx, &broken, Revision::new(4)), TextureState::Missing) {
        store.put_failure(ctx, broken, Revision::new(4), LoadFailure::Unreadable);
    }
    store
}

/// Six files and three templates, none with a picture: every tile shows the placeholder.
fn plain_files() -> Vec<RecentFile> {
    let file = |name: &str, age: i64| {
        let mut file = RecentFile::new(PathBuf::from("/home/user/Pictures").join(name), name);
        file.opened = Some(NOW - age);
        file.created = Some(NOW - age - DAY);
        file
    };
    vec![
        file("Harbour.psd", 30),
        file("Poster draft.psd", 2 * HOUR),
        file("Logo.psd", 2 * DAY),
        file("a10.psd", 9 * DAY),
        file("a2.psd", 40 * DAY),
        file("Scan 0007.psd", 400 * DAY),
    ]
}

fn plain_templates() -> Vec<Template> {
    vec![
        Template::new("hd", "Full HD", "1920 × 1080 px"),
        Template::new("a4", "A4", "210 × 297 mm"),
        Template::new("square", "Square", "1080 × 1080 px"),
    ]
}

fn screen(ui: &mut Ui, data: &mut StoryData, choice: Choice, files: &[RecentFile], pinned: &[PathBuf], templates: &[Template]) -> StoryOutput {
    let store = fill(ui);
    let mut choice = data.value("choice", choice).clone();
    let shown = StartScreen::new(Bind::bound(&mut choice.view), Bind::bound(&mut choice.sort), files)
        .pinned(pinned)
        .templates(templates)
        .now(NOW)
        .thumbnails(store)
        .show(ui)
        .into();
    *data.value("choice", Choice::default()) = choice;
    shown
}

#[story(StartScreen<'static>, covers = [], drive = Drive::Never)]
fn start_screen(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let pinned = [PathBuf::from("/home/user/Pictures"), PathBuf::from("/home/user/Projects")];
    screen(ui, data, Choice::default(), &plain_files(), &pinned, &plain_templates())
}

#[story(StartScreen<'static>, covers = [], drive = Drive::Never)]
fn start_screen_with_pictures(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let pinned = [PathBuf::from("/home/ana/Work"), PathBuf::from("/home/ana/Print")];
    screen(ui, data, Choice::default(), &picture_files(), &pinned, &picture_templates())
}

#[story(StartScreen<'static>, covers = [], drive = Drive::Never)]
fn start_screen_list_sorted_by_name(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let choice = Choice { view: RecentView::List, sort: RecentSort::Name };
    screen(ui, data, choice, &picture_files(), &[], &picture_templates())
}

#[story(StartScreen<'static>, covers = [], drive = Drive::Never)]
fn start_screen_without_files(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    screen(ui, data, Choice::default(), &[], &[], &picture_templates())
}
