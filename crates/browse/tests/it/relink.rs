//! `RelinkDialog` through the fixture, with a fake `LinkFiles` and `ManualJobs`.

use std::cell::RefCell;
use std::io::Cursor;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use craft_browse::ui::relink::{RelinkDialog, RelinkOptions, RelinkOutcome, RelinkRow};
use craft_docpath::links::{FakeLinkFiles, Link, identity_of};
use craft_host::{Jobs, ManualJobs};
use craft_ui::dialog::{DialogResult, DialogStack, StackDialog as _};
use craft_ui::testing::{Fixture, trial};

const STAMP: u64 = 1;

/// The bytes of a file that is the same as every link was recorded with.
const SAME: &[u8] = b"abc";
/// The fixture room of `width` by `height` points.
fn room_of(width: f32, height: f32) -> egui::Vec2 {
    egui::vec2(width, height)
}

/// Bytes of another size, so the file is a different one.
const OTHER: &[u8] = b"abcdefghi";

struct App {
    dialog: RelinkDialog<u32>,
    stack: DialogStack,
    result: Option<DialogResult<RelinkOutcome<u32>>>,
}

/// A dialog under test: the fixture, the dialog's app and the jobs it runs on.
struct Rig {
    fx: Fixture,
    app: Rc<RefCell<App>>,
    jobs: Arc<ManualJobs>,
}

fn row(key: u32, owner: &str, stored: &str) -> RelinkRow<u32> {
    let identity = identity_of(&mut Cursor::new(SAME), SAME.len() as u64, Some(SystemTime::UNIX_EPOCH + Duration::from_secs(STAMP))).ok();
    RelinkRow::new(Link { key, name: owner.into(), stored: stored.into(), identity, page: None, embedded: false }, owner, "Media missing")
}

/// A file system with `files` (path, bytes), all modified at the stamp the links were recorded with.
fn files(files: &[(&str, &[u8])]) -> Arc<FakeLinkFiles> {
    Arc::new(files.iter().fold(FakeLinkFiles::new(), |fake, (path, bytes)| fake.with(*path, bytes, STAMP)))
}

fn rig(rows: Vec<RelinkRow<u32>>, files: &Arc<FakeLinkFiles>, folder: &str, relink_others: bool, with_jobs: bool) -> Rig {
    let jobs = Arc::new(ManualJobs::new());
    let mut options = RelinkOptions::default().relink_others(relink_others);
    if !folder.is_empty() {
        options = options.folder(folder);
    }
    if with_jobs {
        options = options.jobs(Arc::clone(&jobs) as Arc<dyn Jobs>);
    }
    let dialog = RelinkDialog::new(rows, files.clone(), options);
    let mut stack = DialogStack::default();
    stack.push(dialog.id(), None);
    let app = Rc::new(RefCell::new(App { dialog, stack, result: None }));
    let shown = Rc::clone(&app);
    let mut fx = Fixture::new(move |ui| {
        let mut app = shown.borrow_mut();
        let App { dialog, stack, result } = &mut *app;
        if let Some(answer) = dialog.show(ui, stack) {
            *result = Some(answer);
        }
    });
    fx.room(room_of(900.0, 900.0)).settle();
    Rig { fx, app, jobs }
}

impl Rig {
    fn click(&mut self, label: &str) {
        let target = self.fx.by_label(label).unwrap_or_else(|| panic!("no `{label}` on screen"));
        self.fx.click(&target);
        self.fx.settle();
    }

    fn present(&self, text: &str) -> bool {
        self.fx.by_label(text).is_some() || self.fx.texts().iter().any(|painted| painted.text == text)
    }

    /// Runs the search for the selected row and lets its hits arrive.
    fn search(&mut self) {
        self.click("Search");
        self.jobs.run_all();
        self.fx.settle();
    }

    fn done(&mut self) -> DialogResult<RelinkOutcome<u32>> {
        self.click("Done");
        self.app.borrow_mut().result.take().expect("Done answers")
    }
}

fn linked_keys(result: &DialogResult<RelinkOutcome<u32>>) -> (Vec<u32>, Vec<u32>) {
    let DialogResult::Ok(outcome) = result else { panic!("the dialog was cancelled") };
    (outcome.linked.iter().map(|(key, _)| *key).collect(), outcome.offline.clone())
}

#[trial]
fn relink_others_offers_the_rows_the_remap_fits() {
    let files = files(&[("/T7/shoot/a.mov", SAME), ("/T7/shoot/b.mov", SAME), ("/T7/shoot/c.mov", OTHER)]);
    let rows = vec![row(1, "Clip A", "C:\\shoot\\a.mov"), row(2, "Clip B", "C:\\shoot\\b.mov"), row(3, "Clip C", "C:\\shoot\\c.mov")];
    let mut rig = rig(rows, &files, "/T7/shoot", true, true);
    rig.search();
    rig.click("Link");
    rig.jobs.run_all();
    rig.fx.settle();
    assert!(!rig.present("Clip B"), "the row the remap fits is linked");
    assert!(rig.present("Clip C"), "the row that fails its check stays missing");
    assert_eq!(linked_keys(&rig.done()), (vec![1, 2], vec![]));
}
