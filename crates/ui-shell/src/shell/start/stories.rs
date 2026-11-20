//! Stories of `StartScreen`.

use super::{StartLayout, StartLink, StartScreen};
use crate::contract::Bind;
use crate::primitives::{FakeThumbSource, ThumbnailOptions, Thumbnails};
use crate::runtime::env::Label;
use crate::shell::{RecentFile, RecentList, RecentSort, RecentView, ScreenModeCommands, Template, ThumbKey};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::IconId;
use egui::Ui;
use std::path::PathBuf;

/// The time the age lines count from, in seconds since the Unix epoch.
const NOW: i64 = 1_800_000_000;
const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;

/// The view and sort the stories edit.
#[derive(Clone, Debug, Default, PartialEq)]
struct Choice {
    view: RecentView,
    sort: RecentSort,
}

fn recent_file(name: &str, folder: &str, days: i64) -> RecentFile {
    let mut file = RecentFile::new(PathBuf::from(folder).join(name), name);
    file.opened = Some(NOW - days * DAY);
    file.created = Some(NOW - (days + 30) * DAY);
    file
}

/// Five files; the source fails to load "Broken.fig" and has no picture for "Notes.fig".
fn picture_files() -> Vec<RecentFile> {
    vec![
        recent_file("Poster 2.fig", "/home/ana/Work", 0),
        recent_file("Poster 10.fig", "/home/ana/Work", 2),
        recent_file("Brochure.fig", "/home/ana/Print", 5),
        recent_file("Broken.fig", "/home/ana/Print", 9),
        recent_file("Notes.fig", "/home/ana/Notes", 30),
    ]
}

fn picture_templates() -> Vec<Template> {
    vec![Template::new("a4", "A4", "210 × 297 mm"), Template::new("hd", "Full HD", "1920 × 1080 px")]
}

/// The source of the picture stories: pictures for every key but the failing and the blank one.
fn picture_source() -> FakeThumbSource<ThumbKey> {
    let print = PathBuf::from("/home/ana/Print");
    FakeThumbSource::new()
        .failing([ThumbKey::Recent(print.join("Broken.fig"))])
        .blank([ThumbKey::Recent(PathBuf::from("/home/ana/Notes").join("Notes.fig")), ThumbKey::Template("hd".to_owned())])
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

/// What a story passes besides its lists.
#[derive(Default)]
struct Extras<'a> {
    source: Option<&'a FakeThumbSource<ThumbKey>>,
    links: &'a [StartLink],
    drop_hint: Option<&'a str>,
}

fn screen(
    ui: &mut Ui,
    data: &mut StoryData,
    choice: Choice,
    files: &[RecentFile],
    pinned: &[PathBuf],
    templates: &[Template],
    extras: Extras<'_>,
) -> StoryOutput {
    let (choice, choice_at) = data.value_at("choice", choice);
    let thumbnails = Thumbnails::<ThumbKey>::in_context(ui.ctx(), "start-stories", ThumbnailOptions::new());
    let mut start = StartScreen::new(choice_at.bind(&mut choice.view), choice_at.bind(&mut choice.sort), files)
        .pinned(pinned)
        .templates(templates)
        .now(NOW)
        .links(extras.links);
    if let Some(source) = extras.source {
        start = start.thumbnails(&thumbnails, source);
    }
    if let Some(hint) = extras.drop_hint {
        start = start.drop_hint(Label::from(hint.to_owned()));
    }
    start.show(ui).into()
}

#[story(StartScreen<'static>, covers = [], drive = Drive::Never)]
fn start_screen(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let pinned = [PathBuf::from("/home/user/Pictures"), PathBuf::from("/home/user/Projects")];
    screen(ui, data, Choice::default(), &plain_files(), &pinned, &plain_templates(), Extras::default())
}

#[story(StartScreen<'static>, covers = [], drive = Drive::Never)]
fn start_screen_with_pictures(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let pinned = [PathBuf::from("/home/ana/Work"), PathBuf::from("/home/ana/Print")];
    let source = picture_source();
    screen(ui, data, Choice::default(), &picture_files(), &pinned, &picture_templates(), Extras { source: Some(&source), ..Extras::default() })
}

#[story(StartScreen<'static>, covers = [], drive = Drive::Never)]
fn start_screen_list_sorted_by_name(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let choice = Choice { view: RecentView::List, sort: RecentSort::Name };
    let source = picture_source();
    screen(ui, data, choice, &picture_files(), &[], &picture_templates(), Extras { source: Some(&source), ..Extras::default() })
}

#[story(StartScreen<'static>, covers = [], drive = Drive::Never)]
fn start_screen_without_files(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let extras = Extras { drop_hint: Some("Drop a file here or click to open one"), ..Extras::default() };
    screen(ui, data, Choice::default(), &[], &[], &picture_templates(), extras)
}

#[story(StartScreen<'static>, covers = [], drive = Drive::Never)]
fn start_screen_greeting(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let (files, templates) = (plain_files(), plain_templates());
    let (mut view, mut sort) = (RecentView::default(), RecentSort::default());
    StartScreen::new(Bind::bound(&mut view), Bind::bound(&mut sort), &files)
        .templates(&templates)
        .now(NOW)
        .layout(StartLayout::Greeting)
        .title("Good afternoon")
        .more_templates("More templates")
        .show(ui)
        .into()
}

#[story(RecentList<'static>, covers = [], drive = Drive::Never)]
fn recent_list(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let files = plain_files();
    RecentList::new(&files).now(Some(NOW)).empty("No recent files").show(ui).into()
}

/// No recent files: the muted line under the heading, at the reading start.
#[story(RecentList<'static>, covers = [], drive = Drive::Never)]
fn recent_list_empty(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    RecentList::new(&[]).empty("Documents you open will show up here.").show(ui).into()
}

#[story(StartScreen<'static>, covers = [], drive = Drive::Never)]
fn start_screen_with_links(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let links = [
        StartLink::url("Documentation", Some(IconId::new("home")), "https://example.com/docs"),
        StartLink::url("Website", Some(IconId::new("globe")), "https://example.com"),
        StartLink::command("Cycle Screen Mode", None, ScreenModeCommands::Cycle),
    ];
    screen(ui, data, Choice::default(), &plain_files(), &[], &plain_templates(), Extras { links: &links, ..Extras::default() })
}
