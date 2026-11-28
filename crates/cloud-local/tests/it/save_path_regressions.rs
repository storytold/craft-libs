//! Two save-path defects that need a real backend to show: a reload after a conflict carrying a
//! stale etag into the next save, and two creates of the same name racing past an exists check.
//! Driven through craft_ui::cloud's public functions the way an application does.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use craft_cloud::{Entry, EntryKind};
use craft_cloud_local::LocalBackend;
use craft_ui::cloud::{self, CloudApp, CloudFiles, Destination, DocumentKind};

const NOTE: DocumentKind = DocumentKind { extensions: &["txt"], mimes: &["text/plain"], noun: "note", default_extension: "txt" };

struct Window {
    cloud: CloudFiles,
    share: craft_ui::share::ShareDialog,
    presence: craft_ui::presence::PresenceBar,
    body: Vec<u8>,
    said: Arc<Mutex<Vec<String>>>,
}

impl Window {
    fn new(root: std::path::PathBuf, body: &[u8]) -> Window {
        let mut w = Window {
            cloud: CloudFiles::new(),
            share: craft_ui::share::ShareDialog::new(),
            presence: craft_ui::presence::PresenceBar::new(),
            body: body.to_vec(),
            said: Arc::default(),
        };
        w.cloud.connect(LocalBackend::new(root));
        w
    }
}

impl CloudApp for Window {
    fn cloud(&self) -> &CloudFiles {
        &self.cloud
    }
    fn cloud_mut(&mut self) -> &mut CloudFiles {
        &mut self.cloud
    }
    fn kind(&self) -> DocumentKind {
        NOTE
    }
    fn active(&self) -> Option<u64> {
        Some(1)
    }
    fn title(&self, _uid: u64) -> String {
        String::new()
    }
    fn open_bytes(&mut self, _name: &str, bytes: Vec<u8>) -> Result<Option<u64>, String> {
        self.body = bytes;
        Ok(Some(1))
    }
    fn encode(&mut self, _uid: u64, _into: Destination<'_>) -> Result<Vec<u8>, String> {
        Ok(self.body.clone())
    }
    fn close(&mut self, _uid: u64) {}
    fn sharing(&mut self) -> &mut craft_ui::share::ShareDialog {
        &mut self.share
    }
    fn presence(&mut self) -> &mut craft_ui::presence::PresenceBar {
        &mut self.presence
    }
    fn paint_icon(&self, _p: &egui::Painter, _r: egui::Rect, _k: EntryKind, _c: egui::Color32) {}
    fn open_url(&self, _url: &str) {}
    fn say(&mut self, text: String) {
        self.said.lock().unwrap().push(text);
    }
    fn app_name(&self) -> &str {
        "Review"
    }
}

fn settle(app: &mut Window, ctx: &egui::Context, mut done: impl FnMut(&Window) -> bool) {
    let start = Instant::now();
    loop {
        ctx.begin_pass(Default::default());
        cloud::pump(app, ctx);
        ctx.end_pass().textures_delta.clear();
        if done(app) {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(5), "timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn listed(app: &mut Window, ctx: &egui::Context, name: &str) -> Entry {
    cloud::browse(app, "");
    settle(app, ctx, |a| a.cloud.mine.loaded);
    app.cloud.mine.entries.iter().find(|e| e.name == name).cloned().expect("listed")
}

/// Open v1, another writer stores v2, Save conflicts, the user picks "Reload from the cloud"
/// (`reload_active`, what the modal calls). The window now shows v2's bytes, so the next Save
/// must go through rather than conflict again on the etag the reloaded copy was read at.
#[test]
fn reload_after_a_conflict_carries_the_freshly_read_etag_so_the_next_save_succeeds() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("note.txt"), b"v1").unwrap();
    let ctx = egui::Context::default();
    let mut app = Window::new(root.path().to_owned(), b"");
    let v1 = listed(&mut app, &ctx, "note.txt");
    cloud::open_entry(&mut app, v1.clone(), None);
    settle(&mut app, &ctx, |a| a.cloud.busy.is_none() && a.body == b"v1");

    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(root.path().join("note.txt"), b"v2 by someone else").unwrap();

    app.body = b"my edit".to_vec();
    cloud::save(&mut app).unwrap();
    settle(&mut app, &ctx, |a| a.cloud.busy.is_none());
    assert!(app.cloud.conflict.is_some(), "the first save is refused (correct)");

    cloud::reload_active(&mut app).unwrap();
    settle(&mut app, &ctx, |a| a.cloud.busy.is_none());
    assert_eq!(app.body, b"v2 by someone else", "reload shows the stored bytes");
    let origin_etag = cloud::report(&app)["origin"]["etag"].clone();

    cloud::save(&mut app).unwrap();
    settle(&mut app, &ctx, |a| a.cloud.busy.is_none());
    let on_disk = std::fs::read(root.path().join("note.txt")).unwrap();
    assert!(
        app.cloud.conflict.is_none(),
        "after reloading v2 the save conflicted again: origin etag after reload = {origin_etag} \
         (listing etag was {:?}); file on disk = {:?}",
        v1.etag,
        String::from_utf8_lossy(&on_disk)
    );
}

/// Two windows Save As the same new name at once. At most one create may succeed; the other
/// must be refused ("already exists") rather than both reporting success while one silently
/// replaces the other on disk.
#[test]
fn concurrent_create_of_one_name_refuses_the_loser() {
    let rounds = 300;
    let (mut both_ok, mut example) = (0, String::new());
    for round in 0..rounds {
        let root = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut a = Window::new(root.path().to_owned(), format!("window A round {round}").as_bytes());
        let mut b = Window::new(root.path().to_owned(), format!("window B round {round}").as_bytes());
        cloud::save_as(&mut a, "", "Report.txt").unwrap();
        cloud::save_as(&mut b, "", "Report.txt").unwrap();
        settle(&mut a, &ctx, |w| w.cloud.busy.is_none());
        settle(&mut b, &ctx, |w| w.cloud.busy.is_none());
        let (a_ok, b_ok) = (cloud::origin(&a).is_some(), cloud::origin(&b).is_some());
        if a_ok && b_ok {
            both_ok += 1;
            if example.is_empty() {
                let disk = std::fs::read_to_string(root.path().join("Report.txt")).unwrap();
                example = format!("round {round}: A status {:?}, B status {:?}, file holds {disk:?}", a.cloud.status, b.cloud.status);
            }
        }
    }
    assert_eq!(both_ok, 0, "{both_ok}/{rounds} rounds: both windows were told the new file was created; e.g. {example}");
}
