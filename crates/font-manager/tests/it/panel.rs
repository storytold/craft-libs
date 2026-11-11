//! The panel's flows through `FontsPanelView`, against a small index, a font folder made per test and a fake font store.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};

use craft_font_manager::craft_font_catalog::Os;
use craft_font_manager::index::Index;
use craft_font_manager::store::{Call, FakeStore};
use craft_font_manager::{Catalog, FontEnv, FontsPanel, FontsPanelView, Options};
use craft_host::{AppInfo, Host, InlineJobs, Jobs, MemorySections};
use craft_ui::contract::Role;
use craft_ui::testing::{Fixture, Room, Target, trial};
use egui::{Key, Modifiers, Pos2};

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

/// Keeps the spawned jobs until the test runs them.
#[derive(Default)]
struct Deferred(Mutex<Vec<Task>>);

impl Jobs for Deferred {
    fn spawn(&self, _: &str, job: Task) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).push(job);
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
    let root = root(name);
    let system = root.join("usr/share/fonts");
    std::fs::create_dir_all(&system).unwrap();
    std::fs::write(system.join("Ubuntu-Light.ttf"), epaint_default_fonts::UBUNTU_LIGHT).unwrap();
    let store_dir = root.join("store");
    std::fs::create_dir_all(&store_dir).unwrap();
    let env = FontEnv { os: Os::Unix, root: root.clone(), font_store: Some(store_dir.clone()), ..FontEnv::default() };
    let catalog = Arc::new(Catalog::new(env).with_font_service(false));
    catalog.rescan();
    let files = HashMap::from([
        (HACK.to_owned(), epaint_default_fonts::HACK_REGULAR.to_vec()),
        (UBUNTU.to_owned(), epaint_default_fonts::UBUNTU_LIGHT.to_vec()),
    ]);
    let store = Arc::new(FakeStore::new(store_dir, files));
    let panel = Rc::new(RefCell::new(FontsPanel::new(Options::new(catalog, store.clone(), Ok(index())))));
    let shown = Rc::clone(&panel);
    let mut fx = Fixture::new(move |ui| {
        FontsPanelView::new(&mut shown.borrow_mut()).show(ui);
    });
    fx.in_room(Room::Window).host(host_with(sections, jobs));
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

    /// Opens the details of the card named `family`: the card takes the focus, Enter opens it.
    fn open(&mut self, family: &str) {
        let card = self.named(Role::LIST_ITEM, family);
        self.fx.click_at(Pos2::new(card.rect.left() + 4.0, card.rect.top() + 4.0));
        self.key(Key::Enter);
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

    let mut spawned = std::mem::take(&mut *deferred.0.lock().unwrap_or_else(PoisonError::into_inner));
    assert_eq!(spawned.len(), 1, "one runner drains the queue");
    assert!(rig.store.calls().is_empty(), "nothing runs before the runner does");
    spawned.remove(0)();
    assert_eq!(rig.store.calls(), [Call::Install("Amiri".into()), Call::Install("Hack".into())]);
}
