//! Stories of the Links panel and the Link Media dialog.
//!
//! `FakeLinkFiles` is an in-memory file set with a same file, a different file and a folder tree.
//! The dialog's search and checks run on a `ManualJobs`: the search story keeps the jobs waiting, the
//! candidates story runs them at the start of each pass.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use craft_behavior::Selection;
use craft_docpath::Refused;
use craft_docpath::links::{FakeLinkFiles, Link, LinkIdentity, LinkStatus, identity_of};
use craft_host::{Jobs, ManualJobs};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::dialog::{DialogStack, StackDialog as _};
use craft_ui::display::Text;
use craft_ui::list::ThumbSize;
use craft_ui::primitives::textures::{FakeThumbSource, ThumbnailOptions, Thumbnails};
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::{LinkList, LinkListSettings, LinkListState, RelinkDialog, RelinkOptions, RelinkRow};

craft_ui::words! {
    /// The sample text of the link stories, English by default.
    #[non_exhaustive]
    pub struct LinkStoryWords {
        media_missing: "linkStory.mediaMissing" = "Media missing";
        clip_n: template "linkStory.clipN" = "Clip {n}";
    }
}

/// What the files hold: three bytes that the stored links were recorded with.
const SAME: &[u8] = b"abc";
const STAMP: u64 = 1;

fn recorded() -> Option<LinkIdentity> {
    identity_of(&mut Cursor::new(SAME), SAME.len() as u64, Some(SystemTime::UNIX_EPOCH + Duration::from_secs(STAMP))).ok()
}

fn link(key: u32, name: &str, page: Option<u32>, embedded: bool) -> Link<u32> {
    Link { key, name: name.into(), stored: format!("/shoot/{name}"), identity: recorded(), page, embedded }
}

/// Every status once, with a page on one and an embedded link.
fn links() -> (Vec<Link<u32>>, HashMap<u32, LinkStatus>) {
    let rows = [
        ("beach.jpg", None, LinkStatus::Ok),
        ("brochure.pdf", Some(2), LinkStatus::Modified),
        ("cover.psd", None, LinkStatus::Missing),
        ("map.png", None, LinkStatus::Unreadable),
        ("remote.jpg", None, LinkStatus::Refused(Refused::Network)),
        ("sketch.png", None, LinkStatus::Embedded),
    ];
    let mut links = Vec::new();
    let mut statuses = HashMap::new();
    for (key, (name, page, status)) in (0u32..).zip(rows) {
        links.push(link(key, name, page, status == LinkStatus::Embedded));
        statuses.insert(key, status);
    }
    (links, statuses)
}

fn list_story(ui: &mut Ui, data: &mut StoryData, links: &[Link<u32>], statuses: &HashMap<u32, LinkStatus>) -> StoryOutput {
    let state = data.value(
        "state",
        LinkListState {
            selection: Selection::single(2),
            settings: LinkListSettings { thumbnails: ThumbSize::Small, info_open: true, ..LinkListSettings::default() },
        },
    );
    let thumbs = Thumbnails::<u32>::in_context(ui.ctx(), "craft-browse-link-list-story", ThumbnailOptions::new());
    let pictures = FakeThumbSource::<u32>::new().aspect(4, 3);
    LinkList::new("Links", links, statuses, Bind::bound(state))
        .thumbnails(&thumbs, &pictures)
        .info(|ui| {
            Text::new("/shoot/cover.psd").show(ui);
        })
        .show(ui)
        .into()
}

#[story(LinkList<'static, u32>, covers = [], drive = Drive::Never)]
fn link_list(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (links, statuses) = links();
    list_story(ui, data, &links, &statuses)
}

#[story(LinkList<'static, u32>, covers = [], drive = Drive::Never)]
fn link_list_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    list_story(ui, data, &[], &HashMap::new())
}

/// A dialog kept across passes, with the jobs it runs on.
struct Demo {
    jobs: Arc<ManualJobs>,
    dialog: Mutex<RelinkDialog<u32>>,
    started: Mutex<bool>,
}

#[derive(Clone)]
struct Held(Arc<Demo>);

impl PartialEq for Held {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for Held {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Held(..)")
    }
}

fn demo(words: &LinkStoryWords) -> Held {
    let fake = |fake: FakeLinkFiles, path: &str| fake.with(path, SAME, STAMP);
    let files = ["/new/shoot/a.mov", "/new/shoot/b.mov", "/new/other/d.mov"].into_iter().fold(FakeLinkFiles::new(), fake);
    let files = files.with("/new/shoot/c.mov", b"nine byte", STAMP);
    let rows = ["a", "b", "c", "d"]
        .iter()
        .zip(1..)
        .map(|(name, n)| {
            let file = format!("{name}.mov");
            RelinkRow::new(
                Link { key: n, name: file.clone(), stored: format!("/old/shoot/{file}"), identity: recorded(), page: None, embedded: false },
                words.clip_n.fill(&[("n", &n.to_string())]),
                words.media_missing.as_str(),
            )
        })
        .collect();
    let jobs = Arc::new(ManualJobs::new());
    let options = RelinkOptions::default().folder("/new").jobs(Arc::clone(&jobs) as Arc<dyn Jobs>);
    Held(Arc::new(Demo { jobs, dialog: Mutex::new(RelinkDialog::new(rows, Arc::new(files), options)), started: Mutex::new(false) }))
}

/// Shows the demo's dialog; `search` starts the search once and `finish` runs the waiting jobs.
fn dialog_story(ui: &mut Ui, data: &mut StoryData, search: bool, finish: bool) -> StoryOutput {
    let words = Env::of(ui).words::<LinkStoryWords>();
    let held = data.value("dialog", demo(&words)).clone();
    let demo = &held.0;
    let mut dialog = demo.dialog.lock().unwrap_or_else(PoisonError::into_inner);
    let mut stack = DialogStack::default();
    stack.push(dialog.id(), None);
    let _ = dialog.show(ui, &stack);
    let mut started = demo.started.lock().unwrap_or_else(PoisonError::into_inner);
    if search && !*started {
        *started = true;
        dialog.begin_search();
        ui.ctx().request_repaint();
    }
    if finish && demo.jobs.run_all() > 0 {
        ui.ctx().request_repaint();
    }
    StoryOutput::default()
}

#[story(RelinkDialog<u32>, covers = [], drive = Drive::Never)]
fn relink_dialog_before_search(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    dialog_story(ui, data, false, false)
}

#[story(RelinkDialog<u32>, covers = [], drive = Drive::Never)]
fn relink_dialog_searching(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    dialog_story(ui, data, true, false)
}

#[story(RelinkDialog<u32>, covers = [], drive = Drive::Never)]
fn relink_dialog_with_candidates(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    dialog_story(ui, data, true, true)
}
