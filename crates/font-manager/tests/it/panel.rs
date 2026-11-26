//! The panel's flows through `FontsPanelView`, against a small index, a font folder made per test and a fake font store.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use craft_font_manager::craft_font_catalog::Os;
use craft_font_manager::index::Index;
use craft_font_manager::store::{Call, Conflict, FakeStore, Installer as _};
use craft_font_manager::{CATALOGS, Catalog, FontEnv, FontsPanel, FontsPanelView, Options};
use craft_font_store::preview::{Fetch, Previews};
use craft_font_store::{Cancel, FetchError};
use craft_host::{AppInfo, Host, InlineJobs, Jobs, MemorySections};
use craft_i18n::I18n;
use craft_ui::contract::Role;
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, Room, Target, trial};
use egui::{Event, Key, Modifiers, Pos2, Vec2};

const HACK: &str = "ofl/hack/Hack-Regular.ttf";
const UBUNTU: &str = "ufl/ubuntu/Ubuntu-Light.ttf";
const APP: AppInfo = AppInfo { id: "fonts-test", name: "Fonts test", version: "1.0" };

fn index() -> Index {
    let file = |path: &str, size: usize| {
        format!(r#"{{"path":"{path}","size":{size},"sha256":"{}","style":"normal","weight":400,"axes":[],"postscript":[]}}"#, "0a".repeat(32))
    };
    let family = |name: &str, category: &str, languages: &str, scripts: &str, files: String| {
        format!(
            r#"{{"family":"{name}","designer":"","categories":["{category}"],"languages":[{languages}],"scripts":[{scripts}],"license":"OFL","license_path":null,"files":[{files}]}}"#
        )
    };
    let json = format!(
        r#"{{"schema":1,"version":"1.0","source":"google/fonts","commit":"45e3d5269e4a17d823f6d84470bc084dc5ae46f6","base_url":"https://cdn.jsdelivr.net/gh/google/fonts@45e3d5269e4a17d823f6d84470bc084dc5ae46f6/","fallback_base_url":"https://raw.githubusercontent.com/google/fonts/45e3d5269e4a17d823f6d84470bc084dc5ae46f6/",
        "languages":[
          {{"id":"ar_Arab","name":"Arabic","autonym":"العربية","script":"Arab","population":1,"historical":false}},
          {{"id":"en_Latn","name":"English","autonym":"English","script":"Latn","population":1,"historical":false}},
          {{"id":"ja_Jpan","name":"Japanese","autonym":"日本語","script":"Jpan","population":1,"historical":false}}],
        "scripts":[{{"id":"Arab","name":"Arabic"}},{{"id":"Jpan","name":"Japanese"}},{{"id":"Latn","name":"Latin"}}],
        "families":[{},{},{},{}]}}"#,
        family("Amiri", "SERIF", "0,1", r#""Arab","Latn""#, file("ofl/amiri/Amiri-Regular.ttf", 1000)),
        family("Hack", "MONOSPACE", "1", r#""Latn""#, file(HACK, epaint_default_fonts::HACK_REGULAR.len())),
        family("Noto Sans JP", "SANS_SERIF", "1,2", r#""Jpan","Latn""#, file("ofl/notosansjp/NotoSansJP[wght].ttf", 1000)),
        family("Ubuntu", "SANS_SERIF", "1", r#""Latn""#, file(UBUNTU, epaint_default_fonts::UBUNTU_LIGHT.len())),
    );
    Index::from_json(json.as_bytes()).unwrap()
}

fn host_with(sections: Arc<MemorySections>, jobs: Arc<dyn Jobs>) -> Arc<Host> {
    Arc::new(Host::builder(APP).settings(sections).jobs(jobs).build())
}

type Task = Box<dyn FnOnce() + Send>;

// The job name of the framework's document-face loads (`craft_ui::fonts`), which the panel's specimens start.
const FACE_JOBS: &str = "craft-document-faces";

/// Keeps the spawned jobs until the test runs them.
#[derive(Default)]
struct Deferred(Mutex<Vec<(String, Task)>>);

impl Jobs for Deferred {
    fn spawn(&self, name: &str, job: Task) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).push((name.to_owned(), job));
    }
}

struct Rig {
    fx: Fixture,
    panel: Rc<RefCell<FontsPanel>>,
    store: Arc<FakeStore>,
}

fn root(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("panel").join(name);
    let _ = std::fs::remove_dir_all(&root);
    root
}

/// A system with Ubuntu Light installed, an empty font store, and a fake store that can "download" Hack and Ubuntu
/// Light.
fn setup(name: &str) -> Rig {
    setup_with(name, Arc::new(MemorySections::default()))
}

fn setup_with(name: &str, sections: Arc<MemorySections>) -> Rig {
    setup_jobs(name, sections, Arc::new(InlineJobs))
}

fn setup_jobs(name: &str, sections: Arc<MemorySections>, jobs: Arc<dyn Jobs>) -> Rig {
    setup_seeded(name, sections, jobs, |_| {})
}

/// [`setup_jobs`] after `seed` filled the store folder, as families installed by an earlier run.
fn setup_seeded(name: &str, sections: Arc<MemorySections>, jobs: Arc<dyn Jobs>, seed: impl Fn(&Path)) -> Rig {
    let root = root(name);
    let system = root.join("usr/share/fonts");
    std::fs::create_dir_all(&system).unwrap();
    std::fs::write(system.join("Ubuntu-Light.ttf"), epaint_default_fonts::UBUNTU_LIGHT).unwrap();
    let store_dir = root.join("store");
    std::fs::create_dir_all(&store_dir).unwrap();
    seed(&store_dir);
    let files = HashMap::from([
        (HACK.to_owned(), epaint_default_fonts::HACK_REGULAR.to_vec()),
        (UBUNTU.to_owned(), epaint_default_fonts::UBUNTU_LIGHT.to_vec()),
    ]);
    start(&root, Arc::new(FakeStore::new(store_dir, files)), sections, jobs)
}

/// The panel over the system under `root` and `store`, as at a start of the app.
fn start(root: &Path, store: Arc<FakeStore>, sections: Arc<MemorySections>, jobs: Arc<dyn Jobs>) -> Rig {
    let panel = FontsPanel::new(Options::new(catalog(root), store.clone(), Ok(index())));
    show(panel, store, host_with(sections, jobs))
}

/// The fonts installed under `root`, its store folder included.
fn catalog(root: &Path) -> Arc<Catalog> {
    let env = FontEnv { os: Os::Unix, root: root.to_owned(), font_store: Some(root.join("store")), ..FontEnv::default() };
    let catalog = Arc::new(Catalog::new(env).with_font_service(false));
    catalog.rescan();
    catalog
}

/// `panel` over `store`, shown to a window of `host`.
fn show(panel: FontsPanel, store: Arc<FakeStore>, host: Arc<Host>) -> Rig {
    let panel = Rc::new(RefCell::new(panel));
    let shown = Rc::clone(&panel);
    let mut fx = Fixture::new(move |ui| {
        FontsPanelView::new(&mut shown.borrow_mut()).show(ui);
    });
    fx.in_room(Room::Window).host(host);
    fx.settle();
    Rig { fx, panel, store }
}

impl Rig {
    fn named(&self, role: Role, name: &str) -> Target {
        self.fx
            .by_role(role)
            .into_iter()
            .find(|target| {
                craft_ui::testing::access::name(self.fx.tree(), target.node)
                    .is_some_and(|found| found == name || found.starts_with(&format!("{name},")))
            })
            .unwrap_or_else(|| panic!("no {role:?} named `{name}` on screen"))
    }

    /// Presses the notice action named `name`: a link beside the message, a button when the actions stack.
    fn click_notice_action(&mut self, name: &str) {
        let target = [Role::LINK, Role::BUTTON]
            .into_iter()
            .find_map(|role| {
                (self.fx.by_role(role).into_iter()).find(|target| craft_ui::testing::access::name(self.fx.tree(), target.node) == Some(name))
            })
            .unwrap_or_else(|| panic!("no notice action `{name}` on screen"));
        self.fx.click(&target);
        self.fx.settle();
    }

    /// Presses the control named `name`.
    fn click(&mut self, name: &str) {
        let target = self.fx.by_label(name).unwrap_or_else(|| panic!("no control `{name}` on screen"));
        self.fx.click(&target);
        self.fx.settle();
    }

    fn type_into(&mut self, name: &str, text: &str) {
        let target = self.fx.by_label(name).unwrap_or_else(|| panic!("no field `{name}` on screen"));
        self.fx.click(&target);
        self.fx.type_text(text);
        self.fx.settle();
    }

    fn key(&mut self, key: Key) {
        self.fx.key(key, Modifiers::NONE);
        self.fx.settle();
    }

    fn present(&self, text: &str) -> bool {
        self.fx.by_label(text).is_some() || self.fx.texts().iter().any(|painted| painted.text == text)
    }

    fn allow(&mut self) {
        self.panel.borrow_mut().set_downloads_allowed(true).unwrap();
        self.fx.settle();
    }

    fn has_installed(&self, family: &str) -> bool {
        self.panel.borrow().installed().iter().any(|f| f.name == family)
    }

    /// Opens the details of the card named `family` with a click.
    fn open(&mut self, family: &str) {
        let card = self.named(Role::LIST_ITEM, family);
        self.fx.click_at(Pos2::new(card.rect.left() + 4.0, card.rect.top() + 4.0));
        self.fx.settle();
        assert_eq!(self.panel.borrow().details_family().as_deref(), Some(family));
    }
}

#[trial]
fn nothing_downloads_without_consent() {
    let mut rig = setup("consent");
    assert!(rig.present("Downloads are off"));
    rig.open("Hack");

    rig.click("Install all");
    assert!(rig.present("Download from Google Fonts?"));
    rig.click("Cancel");
    assert!(rig.store.calls().is_empty());
    assert!(!rig.panel.borrow().downloads_allowed());
    assert!(!rig.present("Download from Google Fonts?"));
    rig.key(Key::Escape);

    // Allowed from the note, a preview downloads without asking and installs nothing.
    rig.type_into("Search", "Hack");
    rig.click_notice_action("Allow downloads");
    assert!(rig.panel.borrow().downloads_allowed());
    assert!(!rig.present("Downloads are off"));
    rig.click("Preview");
    assert_eq!(rig.store.calls(), [Call::Download("Hack".into())]);
    assert!(!rig.has_installed("Hack"));
}

#[trial]
fn queued_installs_run_one_runner_in_request_order() {
    let deferred = Arc::new(Deferred::default());
    let mut rig = setup_jobs("queue", Arc::new(MemorySections::default()), deferred.clone());
    rig.allow();
    // Each card offers its own Install; a card whose job is queued shows progress instead, so the second press lands on
    // the next family.
    rig.click("Install");
    rig.click("Install");

    // The cards' specimens load their faces through the same jobs; only the install runners count here.
    let mut spawned: Vec<Task> = std::mem::take(&mut *deferred.0.lock().unwrap_or_else(PoisonError::into_inner))
        .into_iter()
        .filter(|(name, _)| name != FACE_JOBS)
        .map(|(_, job)| job)
        .collect();
    assert_eq!(spawned.len(), 1, "one runner drains the queue");
    assert!(rig.store.calls().is_empty(), "nothing runs before the runner does");
    spawned.remove(0)();
    assert_eq!(rig.store.calls(), [Call::Install("Amiri".into()), Call::Install("Hack".into()), Call::MakeAvailable("Hack".into())]);
}

impl Rig {
    /// The names of the family cards on screen.
    fn cards(&self) -> Vec<String> {
        (self.fx.by_role(Role::LIST_ITEM).into_iter())
            .filter_map(|target| craft_ui::testing::access::name(self.fx.tree(), target.node).map(str::to_owned))
            .collect()
    }
}

/// With the keyboard on a card, Cmd/Ctrl+F puts it in the search field: typed text searches.
#[trial]
fn find_puts_the_keyboard_in_the_search_field() {
    let mut rig = setup("find");
    let card = rig.named(Role::LIST_ITEM, "Hack");
    rig.fx.click_at(Pos2::new(card.rect.left() + 4.0, card.rect.top() + 4.0));
    rig.fx.settle();
    rig.fx.key(Key::F, Modifiers::COMMAND);
    rig.fx.settle();
    rig.fx.type_text("Ubuntu");
    rig.fx.settle();
    let cards = rig.cards();
    assert!(!cards.is_empty() && cards.iter().all(|name| !name.starts_with("Hack")), "the search keeps only Ubuntu: {cards:?}");
}

/// A press outside the details sheet closes it and reaches nothing below: the card under it does not open.
#[trial]
fn a_press_outside_the_details_sheet_closes_it() {
    let mut rig = setup("outside");
    let other = rig.fx.by_role(Role::LIST_ITEM).into_iter().last().expect("a second card");
    rig.open("Hack");
    rig.fx.click_at(Pos2::new(other.rect.left() + 4.0, other.rect.bottom() - 4.0));
    rig.fx.settle();
    assert_eq!(rig.panel.borrow().details_family(), None, "the sheet closed and no other family opened");
}

#[trial]
fn installs_are_copied_to_the_system_fonts_folder_unless_kept_in_craft_apps() {
    let mut rig = setup("os-copy");
    rig.allow();
    let conflict = Conflict { postscript: Some("Hack-Regular".into()), path: PathBuf::from("/fonts/Hack-Regular.ttf") };
    rig.store.refuse_next(vec![conflict]);
    rig.open("Hack");
    rig.click("Install all");
    assert_eq!(rig.store.calls(), [Call::Install("Hack".into()), Call::MakeAvailable("Hack".into())]);
    assert!(rig.has_installed("Hack"));
    assert!(rig.present("Not copied to your system's fonts folder: /fonts/Hack-Regular.ttf already has the font name Hack-Regular"));

    // The setting decides what Install does from now on; Hack stays as it is.
    rig.key(Key::Escape);
    rig.click("Keep fonts in Craft apps only");
    assert!(rig.store.craft_apps_only());
    assert_eq!(rig.store.calls().len(), 2);
    rig.open("Hack");
    rig.click("Remove all");
    rig.click("Install all");
    assert_eq!(rig.store.calls()[2..], [Call::Remove("Hack".into()), Call::Install("Hack".into())]);
    assert!(rig.present("Kept in Craft apps only"));

    // The details switch copies it after all.
    rig.click("Make available to all apps");
    assert_eq!(rig.store.calls().last(), Some(&Call::MakeAvailable("Hack".into())));
    assert!(rig.present("Available to all apps"));
}

#[trial]
fn each_family_keeps_its_system_fonts_choice_across_starts() {
    // Ubuntu, installed by an earlier run with no choice recorded, follows the default.
    let seed = |dir: &Path| {
        std::fs::create_dir_all(dir.join("Ubuntu")).unwrap();
        std::fs::write(dir.join("Ubuntu/Ubuntu-Light.ttf"), epaint_default_fonts::UBUNTU_LIGHT).unwrap();
    };
    let mut rig = setup_seeded("os-choice", Arc::new(MemorySections::default()), Arc::new(InlineJobs), seed);
    assert_eq!(rig.store.calls(), [Call::MakeAvailable("Ubuntu".into())]);

    rig.allow();
    rig.open("Hack");
    rig.click("More install all options");
    rig.click("Install for Craft apps only");
    assert_eq!(rig.store.calls()[1..], [Call::Install("Hack".into())]);
    assert!(rig.present("Kept in Craft apps only"));

    // The next start copies neither: Hack is kept in Craft apps, Ubuntu is there already.
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("panel").join("os-choice");
    let again = start(&root, rig.store.clone(), Arc::new(MemorySections::default()), Arc::new(InlineJobs));
    assert_eq!(again.store.calls().len(), 2);
}

/// A single click on a card's name area opens its details; a click on its Install control does only its own thing.
#[trial]
fn a_single_click_on_a_card_opens_its_details() {
    let mut rig = setup("card-click");
    rig.allow();
    rig.type_into("Search", "Hack");
    rig.click("Install");
    assert_eq!(rig.panel.borrow().details_family(), None, "a press on a card's control opens no details");
    let card = rig.named(Role::LIST_ITEM, "Hack");
    rig.fx.click_at(Pos2::new(card.rect.left() + 4.0, card.rect.top() + 4.0));
    rig.fx.settle();
    assert_eq!(rig.panel.borrow().details_family().as_deref(), Some("Hack"));
}

/// A card's Install menu installs for the choice picked, and the press on it opens no details.
#[trial]
fn a_cards_install_menu_chooses_where_the_fonts_go() {
    let mut rig = setup("card-menu");
    rig.allow();
    rig.type_into("Search", "Hack");
    rig.click("More install options");
    rig.click("Install for Craft apps only");
    assert_eq!(rig.store.calls(), [Call::Install("Hack".into())]);
    assert_eq!(rig.panel.borrow().details_family(), None);
}

/// A preview service that answers every subset with the same small font, and logs what it answered. While held it
/// answers nothing, and a request cancelled meanwhile is logged as cancelled instead.
#[derive(Default)]
struct FakeService {
    answered: Mutex<Vec<String>>,
    cancelled: Mutex<Vec<String>>,
    /// Requests that reached the service.
    started: AtomicUsize,
    held: AtomicBool,
    /// Answers sample subsets with HTTP 404, as for a sample text the service doesn't have.
    no_samples: AtomicBool,
    /// Answers every subset with HTTP 500.
    failing: AtomicBool,
}

impl Fetch for FakeService {
    fn get(&self, url: &str, _: u64, cancel: &Cancel) -> Result<Vec<u8>, FetchError> {
        self.started.fetch_add(1, Ordering::SeqCst);
        while self.held.load(Ordering::SeqCst) && !cancel.is_cancelled() {
            std::thread::sleep(Duration::from_millis(2));
        }
        if cancel.is_cancelled() {
            self.cancelled.lock().unwrap_or_else(PoisonError::into_inner).push(url.to_owned());
            return Err(FetchError::Cancelled);
        }
        self.answered.lock().unwrap_or_else(PoisonError::into_inner).push(url.to_owned());
        if self.no_samples.load(Ordering::SeqCst) && url.contains("/sample-") {
            return Err(FetchError::Failed("HTTP 404".into()));
        }
        if self.failing.load(Ordering::SeqCst) {
            return Err(FetchError::Failed("HTTP 500".into()));
        }
        Ok(include_bytes!("../data/Tiny-glyf.woff2").to_vec())
    }
}

impl FakeService {
    /// The URLs answered since the last call.
    fn urls(&self) -> Vec<String> {
        std::mem::take(&mut *self.answered.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// The numbers of the families whose `subset` the URLs of `answered` name: `sample`, `ascii`, or `u/` for typed
    /// characters.
    fn numbers(answered: &[String], subset: &str) -> Vec<usize> {
        let mut numbers: Vec<usize> = answered
            .iter()
            .filter_map(|url| url.split_once("/v1/faces/"))
            .filter(|(_, path)| path[64..].starts_with(&format!("/list/{subset}")))
            .map(|(_, path)| usize::from_str_radix(&path[..2], 16).unwrap())
            .collect();
        numbers.sort_unstable();
        numbers
    }
}

const SERVICE: &str = "http://127.0.0.1:9";
const SAMPLE: &str = "Sphinx of black quartz, judge my vow";

/// An index of `families` with `SAMPLE` as the sample text of Latin.
fn service_index(families: &str) -> Index {
    Index::from_json(service_json(families).as_bytes()).unwrap()
}

fn service_json(families: &str) -> String {
    format!(
        r#"{{"schema":1,"version":"1.2","source":"google/fonts","commit":"45e3d5269e4a17d823f6d84470bc084dc5ae46f6","base_url":"https://cdn.jsdelivr.net/gh/google/fonts@45e3d5269e4a17d823f6d84470bc084dc5ae46f6/",
        "languages":[{{"id":"en_Latn","name":"English","script":"Latn","population":1}}],
        "scripts":[{{"id":"Latn","name":"Latin"}}],
        "samples":{{"Latn":{{"text":"{SAMPLE}","method":"udhr","language":"en_Latn"}}}},
        "families":[{families}]}}"#
    )
}

/// Sixty Latin families, `Family 01` to `Family 60`, whose one file's sha256 starts with the family's number in hex.
fn latin_index() -> Index {
    let families: Vec<String> = (1..=60)
        .map(|n| {
            format!(
                r#"{{"family":"Family {n:02}","categories":["SANS_SERIF"],"languages":[0],"scripts":["Latn"],"license":"OFL","sample_script":"Latn",
                "files":[{{"path":"ofl/family{n}/Family-Regular.ttf","size":1000,"sha256":"{}","style":"normal","weight":400,"axes":[],"postscript":[]}}]}}"#,
                format!("{n:02x}").repeat(32)
            )
        })
        .collect();
    service_index(&families.join(","))
}

/// The panel over `index` with no installed fonts and previews from `service`, whose address is not set yet.
fn service_rig(name: &str, service: &Arc<FakeService>, index: Index) -> Rig {
    let root = root(name);
    std::fs::create_dir_all(root.join("store")).unwrap();
    let store = Arc::new(FakeStore::new(root.join("store"), HashMap::new()));
    let previews = Previews::new(store.clone()).with_fetcher(service.clone()).with_cache(None);
    let panel = FontsPanel::new(Options::new(catalog(&root), store.clone(), Ok(index))).with_previews(previews);
    show(panel, store, host_with(Arc::new(MemorySections::default()), Arc::new(InlineJobs)))
}

impl Rig {
    /// Sets the preview service and the previews setting, as a host's settings page does.
    fn set_previews(&mut self, on: bool) {
        self.store.set_preview_service(Some(SERVICE)).unwrap();
        self.store.set_previews_enabled(on).unwrap();
        self.panel.borrow_mut().reload_preview_settings();
        self.fx.settle();
    }

    /// Runs passes until the panel has fetched what it asks for and asks for nothing more.
    fn load(&mut self) {
        let mut quiet = 0;
        for _ in 0..500 {
            self.fx.frame(Vec::new());
            quiet = if self.panel.borrow().loading_previews() { 0 } else { quiet + 1 };
            if quiet == 3 {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("previews kept loading");
    }

    /// Types `text` as the sample, waits for the pause and loads what it asks for.
    fn sample(&mut self, text: &str) {
        self.type_into("Sample text", text);
        self.fx.advance(1.0);
        self.load();
    }

    /// The numbers of the family cards on screen.
    fn numbers(&self) -> Vec<usize> {
        let numbers = self.cards().iter().filter_map(|name| name.strip_prefix("Family ")?.get(..2)?.parse().ok()).collect();
        sorted(numbers)
    }

    /// Scrolls the cards by `points`.
    fn scroll(&mut self, points: f32) {
        let cards = self.named(Role::LIST, "Families").center();
        self.fx.frame(vec![Event::PointerMoved(cards)]).frame(vec![craft_ui::testing::pointer::wheel(-points)]);
        for _ in 0..4 {
            self.fx.advance(0.5);
            self.fx.frame(Vec::new());
        }
        self.fx.settle();
    }
}

fn sorted(mut numbers: Vec<usize>) -> Vec<usize> {
    numbers.sort_unstable();
    numbers
}

/// The cards on screen and round them draw from the preview service without a click, and the typed text too; a card that
/// scrolls away is not asked for any more.
#[trial]
fn previews_follow_the_screen_and_the_typed_text() {
    let service = Arc::new(FakeService::default());
    let mut rig = service_rig("previews", &service, latin_index());
    assert!(service.urls().is_empty(), "no service, no request");
    assert!(rig.present("Preview"), "the whole-family preview is offered");
    rig.set_previews(false);
    assert!(!rig.panel.borrow().loading_previews() && service.urls().is_empty(), "previews are off");

    rig.set_previews(true);
    rig.load();
    assert!(!rig.panel.borrow().downloads_allowed(), "installs are still not allowed");
    assert!(rig.present("Nothing is installed until you allow downloads."));
    assert!(!rig.present("Preview"), "no session download while the service previews");
    let shown = rig.numbers();
    let answered = service.urls();
    let samples = FakeService::numbers(&answered, "sample-");
    assert!(shown.len() > 1 && shown.len() < 20, "{shown:?}");
    assert!(shown.iter().all(|n| samples.contains(n)) && samples.len() > shown.len(), "the screen, then the rows below it: {samples:?}");
    assert!(samples.len() < 60, "not the whole catalogue: {samples:?}");
    assert_eq!(FakeService::numbers(&answered, "ascii"), shown, "ASCII for the screen");

    rig.sample("Hello, world");
    assert!(service.urls().is_empty(), "the ASCII subsets have typed ASCII");
    rig.sample("สวัสดี");
    assert_eq!(FakeService::numbers(&service.urls(), "u/"), shown, "one subset of the typed characters per row on screen");

    service.held.store(true, Ordering::SeqCst);
    rig.scroll(6.0 * 400.0);
    assert!(rig.panel.borrow().loading_previews());
    let passed = rig.numbers();
    // The first wheel goes to the end of the list as its unshown cards are estimated, the second to its end.
    rig.scroll(100_000.0);
    rig.scroll(100_000.0);
    service.held.store(false, Ordering::SeqCst);
    rig.load();
    let last = rig.numbers();
    assert!(passed[0] > shown[shown.len() - 1] && last[0] > passed[passed.len() - 1] && last.contains(&60), "{shown:?} {passed:?} {last:?}");
    let answered = FakeService::numbers(&service.urls(), "u/");
    assert!(!answered.is_empty() && answered.iter().all(|n| last.contains(n)), "rows that left were asked for: {answered:?} {last:?}");
}

/// A resize and a search that keep the cards on screen cancel nothing that is being fetched.
#[trial]
fn a_resize_and_a_search_cancel_no_preview() {
    let service = Arc::new(FakeService::default());
    service.held.store(true, Ordering::SeqCst);
    let mut rig = service_rig("previews-kept", &service, latin_index());
    rig.set_previews(true);
    for _ in 0..500 {
        if service.started.load(Ordering::SeqCst) > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(service.started.load(Ordering::SeqCst) > 0, "the service was asked");
    let shown = rig.numbers();
    rig.fx.room(Room::Window.size() + Vec2::new(40.0, 120.0)).settle();
    rig.type_into("Search", "Family");
    service.held.store(false, Ordering::SeqCst);
    rig.load();
    assert!(shown.iter().all(|n| rig.numbers().contains(n)), "the cards stayed on screen: {shown:?} {:?}", rig.numbers());
    let cancelled = std::mem::take(&mut *service.cancelled.lock().unwrap_or_else(PoisonError::into_inner));
    assert!(cancelled.is_empty(), "{cancelled:?}");
    let samples = FakeService::numbers(&service.urls(), "sample-");
    assert!(shown.iter().all(|n| samples.contains(n)), "{shown:?} {samples:?}");
}

/// The service is not asked for the files of a source the user added, which offer the whole-family preview.
#[trial]
fn a_family_of_an_added_source_is_not_asked_of_the_service() {
    let service = Arc::new(FakeService::default());
    let file = format!(
        r#"{{"path":"ofl/added/Added-Regular.ttf","size":1000,"sha256":"{}","style":"normal","weight":400,"axes":[],"postscript":[]}}"#,
        "ef".repeat(32)
    );
    let family = format!(
        r#"{{"family":"Added","categories":["SANS_SERIF"],"languages":[0],"scripts":["Latn"],"license":"OFL","sample_script":"Latn","files":[{file}]}}"#
    );
    let index = Index::from_user_bytes(service_json(&family).as_bytes(), "https://fonts.example.org/index.json").unwrap();
    let mut rig = service_rig("previews-added", &service, index);
    rig.set_previews(true);
    rig.load();
    rig.sample("Hello, world");
    assert!(rig.present("Preview"), "the card offers the whole-family preview");
    rig.open("Added");
    rig.load();
    rig.named(Role::BUTTON, "Preview");
    assert_eq!(service.started.load(Ordering::SeqCst), 0, "{:?}", service.urls());
}

/// A sample the service has not (another text) is asked for as the sample's characters, once.
#[trial]
fn a_sample_the_service_lacks_falls_back_to_its_characters_once() {
    let service = Arc::new(FakeService::default());
    service.no_samples.store(true, Ordering::SeqCst);
    let mut rig = service_rig("previews-404", &service, latin_index());
    rig.set_previews(true);
    rig.load();
    let shown = rig.numbers();
    assert_eq!(FakeService::numbers(&service.urls(), "u/"), shown, "one subset of the sample's characters per row on screen");
    rig.fx.advance(60.0);
    rig.load();
    assert!(FakeService::numbers(&service.urls(), "sample-").is_empty(), "the sample is not asked for again");
}

/// A family the service can't draw offers the whole-family preview, and the panel names the server that failed.
#[trial]
fn a_failing_preview_service_is_named_and_previews_are_offered() {
    let service = Arc::new(FakeService::default());
    service.failing.store(true, Ordering::SeqCst);
    let mut rig = service_rig("previews-failing", &service, latin_index());
    rig.set_previews(true);
    rig.load();
    assert!(!service.urls().is_empty());
    assert!(rig.present("Font previews are unavailable"));
    assert!(rig.fx.texts().iter().any(|painted| painted.text.contains(SERVICE)), "the notice names the server");
    assert!(rig.present("Preview"), "the card offers the whole-family preview");
    rig.open("Family 01");
    // The sheet's is a button, a card's a link.
    rig.named(Role::BUTTON, "Preview");
}

/// One family: a variable upright file with three named instances, whose sha256 is `ab…`, and a static italic, `cd…`.
fn variable_index() -> Index {
    let file = |path: &str, sha: &str, style: &str, more: &str| {
        format!(r#"{{"path":"ofl/varia/{path}","size":1000,"sha256":"{}","style":"{style}","weight":400,"postscript":[],{more}}}"#, sha.repeat(32))
    };
    let instance = |name: &str, weight: u32| format!(r#"{{"name":"{name}","coordinates":{{"wght":{weight}}}}}"#);
    let instances = [instance("Thin", 100), instance("Regular", 400), instance("Bold", 700)].join(",");
    let axes = format!(r#""axes":[{{"tag":"wght","min":100,"max":900}}],"instances":[{instances}],"list_instance":1"#);
    let files = [file("Varia[wght].ttf", "ab", "normal", &axes), file("Varia-Italic.ttf", "cd", "italic", r#""axes":[]"#)].join(",");
    service_index(&format!(
        r#"{{"family":"Varia","categories":["SANS_SERIF"],"languages":[0],"scripts":["Latn"],"license":"OFL","sample_script":"Latn","files":[{files}]}}"#
    ))
}

/// Every style of a variable file draws from one subset, whatever its axis sliders say.
#[trial]
fn a_variable_family_draws_one_subset_at_every_axis_value() {
    let service = Arc::new(FakeService::default());
    let mut rig = service_rig("previews-variable", &service, variable_index());
    rig.set_previews(true);
    rig.load();
    service.urls();
    rig.open("Varia");
    rig.load();
    let urls = service.urls();
    let var: Vec<&String> = urls.iter().filter(|url| url.contains("/var/")).collect();
    assert!(var.len() == 1 && var[0].contains(&"ab".repeat(32)) && var[0].contains("/var/sample-"), "one subset for the variable file: {urls:?}");
    assert!(urls.iter().any(|url| url.contains(&"cd".repeat(32)) && url.contains("/list/sample-")), "the static italic's own: {urls:?}");
    for style in ["Thin", "Regular", "Bold", "Italic"] {
        assert!(rig.present(style), "no {style} row");
    }

    rig.fx.type_text("9");
    assert!(rig.present("400"), "no field of the sheet has the keyboard when it opens");
    let weight = rig.named(Role::SLIDER, "Weight");
    assert!(rig.present("400") && !rig.present("401"));
    rig.fx.access(AccessActionKind::Increment, &weight);
    rig.load();
    assert!(rig.present("401"), "the slider moved");
    assert!(service.urls().is_empty(), "the slider asked for another subset");
}

/// The rows the crate exports read without a problem, and a Host with them shows the panel in its language.
#[trial]
fn the_exported_rows_show_the_panel_in_japanese() {
    let (i18n, problems) = I18n::new(CATALOGS, &["ja", "ar"]);
    assert!(problems.is_empty() && i18n.check_all().is_empty(), "{problems:?} {:?}", i18n.check_all());
    let lang = i18n.from_code("ja").unwrap();
    let root = root("japanese");
    let store = Arc::new(FakeStore::new(root.join("store"), HashMap::new()));
    let panel = FontsPanel::new(Options::new(catalog(&root), store.clone(), Ok(index())));
    let rig = show(panel, store, Arc::new(Host::builder(APP).i18n(Arc::new(i18n)).lang(lang).jobs(Arc::new(InlineJobs)).build()));
    assert!(rig.present("すべてのカテゴリ") && !rig.present("All categories"));
}

/// With downloads off, a family's NOTICE is offered in its sheet and not read until the user asks.
#[trial]
fn a_notice_is_read_only_when_asked_while_downloads_are_off() {
    let file = format!(
        r#"{{"path":"apache/a/A-Regular.ttf","size":1000,"sha256":"{}","style":"normal","weight":400,"axes":[],"postscript":[]}}"#,
        "aa".repeat(32)
    );
    let notice = format!(r#"{{"path":"apache/a/NOTICE","size":10,"sha256":"{}"}}"#, "bb".repeat(32));
    let family = format!(
        r#"{{"family":"Noticed","categories":["SANS_SERIF"],"languages":[0],"scripts":["Latn"],"license":"APACHE2","notice_file":{notice},"files":[{file}]}}"#
    );
    let root = root("notice");
    let files = HashMap::from([("apache/a/NOTICE".to_owned(), b"Copyright Noticed".to_vec())]);
    let store = Arc::new(FakeStore::new(root.join("store"), files));
    let panel = FontsPanel::new(Options::new(catalog(&root), store.clone(), Ok(service_index(&family))));
    let mut rig = show(panel, store, host_with(Arc::new(MemorySections::default()), Arc::new(InlineJobs)));
    rig.open("Noticed");
    assert!(!rig.panel.borrow().downloads_allowed());
    rig.named(Role::BUTTON, "Show notice");
    assert!(!rig.present("Copyright Noticed"), "the text is not read");
    assert!(!rig.present("Retry"), "nor was a read refused");

    rig.allow();
    assert!(rig.fx.texts().iter().any(|painted| painted.text.contains("Copyright Noticed")), "read once downloads are allowed");
}

#[trial]
fn default_filters_are_sans_serif_and_the_system_language() {
    let mut rig = setup("default-filters");
    rig.panel.borrow_mut().use_default_filters("ja-JP");
    rig.fx.settle();
    assert_eq!(rig.panel.borrow().results(), ["Noto Sans JP"]);
    rig.panel.borrow_mut().use_default_filters("fr");
    rig.fx.settle();
    assert_eq!(rig.panel.borrow().results(), ["Noto Sans JP", "Ubuntu"], "no French in the index: English");
}
