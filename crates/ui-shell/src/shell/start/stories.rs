//! Stories of `StartScreen`.

use super::{FolderListing, PinnedFile, PinnedFolder, StartLayout, StartLink, StartPlace, StartScreen};
use crate::buttons::Button;
use crate::contract::{Bind, Props as _, Show as _};
use crate::notices::Notice;
use crate::primitives::{FakeThumbSource, ThumbnailOptions, Thumbnails};
use crate::runtime::env::Label;
use crate::shell::{RecentFile, RecentList, RecentSort, RecentView, ScreenModeCommands, Template, ThumbKey};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::IconId;
use craft_theme::variant::Tone;
use egui::Ui;
use std::path::{Path, PathBuf};

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
    RecentFile::new(PathBuf::from(folder).join(name), name).opened(NOW - days * DAY).created(NOW - (days + 30) * DAY)
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
    let file =
        |name: &str, age: i64| RecentFile::new(PathBuf::from("/home/user/Pictures").join(name), name).opened(NOW - age).created(NOW - age - DAY);
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

/// An app section a story adds: its title and its body.
type AppSection<'a> = (&'a str, &'a mut dyn FnMut(&mut Ui));

/// A pinned folder at `path`, named by its last component, not listed.
fn pinned_folder(path: &str) -> PinnedFolder {
    let name = Path::new(path).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    PinnedFolder::new(path, name)
}

/// A folder at `path` the app listed: the newest `kept` of its `total` files, each with its page count.
fn listed_folder(path: &str, kept: usize, total: usize) -> PinnedFolder {
    let files = (1..=kept)
        .map(|n| {
            let name = format!("Invoice {n:03}.pdf");
            let file = PathBuf::from(path).join(&name);
            PinnedFile::new(file, name).note(format!("{} pages", n % 9 + 1))
        })
        .collect();
    pinned_folder(path).listing(FolderListing::Listed { files, total })
}

/// What a story passes besides its lists.
#[derive(Default)]
struct Extras<'a> {
    source: Option<&'a FakeThumbSource<ThumbKey>>,
    links: &'a [StartLink],
    drop_hint: Option<&'a str>,
    section: Option<AppSection<'a>>,
    /// New is left out (`offers_new(false)`).
    without_new: bool,
    /// A host section before Recent and one after the built-in sections.
    host_sections: bool,
}

fn screen(
    ui: &mut Ui,
    data: &mut StoryData,
    choice: Choice,
    files: &[RecentFile],
    pinned: &[PinnedFolder],
    templates: &[Template],
    extras: Extras<'_>,
) -> StoryOutput {
    let (choice, choice_at) = data.value_at("choice", choice);
    let thumbnails = Thumbnails::<ThumbKey>::in_context(ui.ctx(), "start-stories", ThumbnailOptions::new());
    let mut start = StartScreen::new(choice_at.bind(&mut choice.view), choice_at.bind(&mut choice.sort), files)
        .pinned(pinned)
        .templates(templates)
        .now(NOW)
        .links(extras.links)
        .offers_new(!extras.without_new);
    if let Some(source) = extras.source {
        start = start.thumbnails(&thumbnails, source);
    }
    if let Some(hint) = extras.drop_hint {
        start = start.drop_hint(Label::from(hint.to_owned()));
    }
    if let Some((title, body)) = extras.section {
        start = start.app_section(Label::from(title.to_owned()), body);
    }
    if extras.host_sections {
        start = start
            .section(StartPlace::BeforeRecent, |ui, _| {
                let _ = Notice::new(Tone::Neutral, Label::from("Sign in to see the files shared with you.".to_owned())).show(ui);
            })
            .section(StartPlace::AfterSections, |ui, _| {
                let _ = Button::new(Label::from("Browse Samples…".to_owned())).id_salt("samples").show(ui);
            });
    }
    start.show(ui).into()
}

#[story(StartScreen<'static>, covers = [], drive = Drive::Never)]
fn start_screen(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let pinned = [pinned_folder("/home/user/Pictures"), pinned_folder("/home/user/Projects")];
    screen(ui, data, Choice::default(), &plain_files(), &pinned, &plain_templates(), Extras::default())
}

#[story(StartScreen<'static>, covers = [], drive = Drive::Never)]
fn start_screen_with_app_section(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let pinned = [pinned_folder("/home/user/Pictures")];
    let mut buttons = |ui: &mut Ui| {
        let _ = Button::new(Label::from("Browse Tutorials".to_owned())).id_salt("tutorials").show(ui);
        let _ = Button::new(Label::from("What's New".to_owned())).id_salt("whats-new").show(ui);
    };
    let body: &mut dyn FnMut(&mut Ui) = &mut buttons;
    let extras = Extras { section: Some(("Learn", body)), ..Extras::default() };
    screen(ui, data, Choice::default(), &plain_files(), &pinned, &plain_templates(), extras)
}

#[story(StartScreen<'static>, covers = [], drive = Drive::Never)]
fn start_screen_with_pictures(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let pinned = [pinned_folder("/home/ana/Work"), pinned_folder("/home/ana/Print")];
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

/// Templates that stand for page sizes: their cards show the pages.
fn page_templates() -> Vec<Template> {
    vec![
        Template::new("letter", "Letter", "612 × 792 pt").page(612.0, 792.0),
        Template::new("web", "Web 1920×1080", "1920 × 1080 px").page(1920.0, 1080.0),
        Template::new("phone", "Phone 390×844", "390 × 844 px").page(390.0, 844.0),
        Template::new("square", "Square 1080×1080", "1080 × 1080 px").page(1080.0, 1080.0),
    ]
}

/// The links of the Column stories: a call to action and two plain links.
fn column_links() -> [StartLink; 3] {
    [
        StartLink::url("Join the community", Some(IconId::new("note")), "https://example.com/chat").primary(),
        StartLink::url("Website", Some(IconId::new("globe")), "https://example.com"),
        StartLink::url("Source code", Some(IconId::new("open")), "https://example.com/source"),
    ]
}

/// The Column page of `files`: title and subtitle, New and Open, the recent rows, the page templates and the links.
fn column_screen(ui: &mut Ui, files: &[RecentFile]) -> StoryOutput {
    let (templates, links) = (page_templates(), column_links());
    let (mut view, mut sort) = (RecentView::default(), RecentSort::default());
    StartScreen::new(Bind::bound(&mut view), Bind::bound(&mut sort), files)
        .max_recent(6)
        .templates(&templates)
        .templates_title("Start a new file")
        .links(&links)
        .layout(StartLayout::Column)
        .title("Welcome")
        .subtitle("Make something today.")
        .links_title("Community")
        .show(ui)
        .into()
}

/// One centred column, with recent files.
#[story(StartScreen<'static>, covers = [], drive = Drive::Never)]
fn start_screen_column(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    column_screen(ui, &plain_files())
}

/// One centred column with no recent files: no Recent section.
#[story(StartScreen<'static>, covers = [], drive = Drive::Never)]
fn start_screen_column_without_files(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    column_screen(ui, &[])
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

/// A host section above Recent and one after Templates.
#[story(StartScreen<'static>, covers = [], drive = Drive::Never)]
fn start_screen_with_host_sections(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let extras = Extras { host_sections: true, ..Extras::default() };
    screen(ui, data, Choice::default(), &plain_files(), &[], &plain_templates(), extras)
}

/// An app that makes no documents: Open leads and New is left out.
#[story(StartScreen<'static>, covers = [], drive = Drive::Never)]
fn start_screen_without_new(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let extras = Extras { without_new: true, ..Extras::default() };
    screen(ui, data, Choice::default(), &plain_files(), &[], &[], extras)
}

/// Recent files whose app notes show in place of the age, in the list and on the cards; the last two keep their age.
#[story(StartScreen<'static>, covers = [], drive = Drive::Never)]
fn start_screen_with_recent_notes(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let notes = ["12 pages · 2.4 MB", "1 page · 180 KB", "48 pages · 9.1 MB", "3 pages · 640 KB"];
    let files: Vec<RecentFile> = plain_files()
        .into_iter()
        .enumerate()
        .map(|(at, file)| match notes.get(at) {
            Some(note) => file.note(*note),
            None => file,
        })
        .collect();
    let choice = Choice { view: RecentView::List, sort: RecentSort::Opened };
    screen(ui, data, choice, &files, &[], &[], Extras::default())
}

/// Pinned folders in every listing state: listed, folded, pending, unavailable, empty, capped (the app kept the newest
/// 12 of 40, eight show with a More row) and expanded.
#[story(StartScreen<'static>, covers = [], drive = Drive::Never)]
fn start_screen_with_pinned_listing(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let pinned = [
        listed_folder("/home/ana/Invoices", 5, 5),
        listed_folder("/home/ana/Archive", 6, 6).folded(true),
        pinned_folder("/home/ana/Inbox").listing(FolderListing::Pending),
        pinned_folder("/mnt/share/Scans").listing(FolderListing::Unavailable),
        pinned_folder("/home/ana/Empty").listing(FolderListing::Listed { files: Vec::new(), total: 0 }),
        listed_folder("/home/ana/Statements", 12, 40),
        listed_folder("/home/ana/Receipts", 10, 10).expanded(true),
    ];
    screen(ui, data, Choice::default(), &plain_files(), &pinned, &[], Extras::default())
}
