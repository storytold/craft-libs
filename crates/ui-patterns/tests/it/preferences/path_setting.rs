//! `PathSetting` over the settings draft, and draft keys that name a list entry.

use craft_host::{AppInfo, Host, PickId, PickRequest, Picked, ScriptedFiles};
use craft_ui::contract::Role;
use craft_ui::patterns::preferences::{PathSetting, SettingsDraft};
use craft_ui::testing::{Fixture, trial};
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Disk {
    enabled: bool,
    path: String,
}

fn disks(paths: &[&str]) -> Vec<Disk> {
    paths.iter().map(|path| Disk { enabled: true, path: (*path).to_owned() }).collect()
}

/// A key part may name a list entry: reads and writes reach that entry only, a write to an entry that is not there
/// changes nothing, and removing an entry removes it from the list.
#[trial]
fn draft_keys_name_list_entries() {
    let mut draft = SettingsDraft::default();
    draft.set("performance.scratchDisks", &disks(&["/a", "/b"]));
    assert_eq!(draft.get::<String>("performance.scratchDisks[1].path").as_deref(), Some("/b"));
    draft.set("performance.scratchDisks[1].path", &"/c");
    assert_eq!(draft.get::<Vec<Disk>>("performance.scratchDisks"), Some(disks(&["/a", "/c"])));
    draft.set("performance.scratchDisks[5].path", &"/x");
    assert_eq!(draft.get::<Vec<Disk>>("performance.scratchDisks"), Some(disks(&["/a", "/c"])), "no entry 5");
    assert_eq!(draft.get::<String>("performance.scratchDisks[x].path"), None, "a malformed index names nothing");
    draft.remove("performance.scratchDisks[0]");
    assert_eq!(draft.get::<Vec<Disk>>("performance.scratchDisks"), Some(disks(&["/c"])));
}

/// Browse on one entry's row asks the host's file service; the answer lands in that entry of the draft and nowhere else,
/// once, and a typed path commits into the draft too.
#[trial]
fn a_path_setting_writes_the_picked_folder_into_its_draft_entry() {
    let files = Arc::new(ScriptedFiles::held());
    let draft = Rc::new(RefCell::new(SettingsDraft::default()));
    draft.borrow_mut().set("performance.scratchDisks", &disks(&["/a", "/b"]));
    let changes = Rc::new(Cell::new(0usize));
    let ticket = Rc::new(Cell::new(None::<PickId>));
    let (held, counted, issued) = (Rc::clone(&draft), Rc::clone(&changes), Rc::clone(&ticket));
    let mut fixture = Fixture::new(move |ui| {
        let mut draft = held.borrow_mut();
        for (i, label) in ["Disk 1", "Disk 2"].into_iter().enumerate() {
            let key = format!("performance.scratchDisks[{i}].path");
            let out = PathSetting::new(label, &key, &mut draft, PickRequest::folder("scratch")).show(ui);
            if out.browsing.is_some() {
                issued.set(out.browsing);
            }
            if out.changed {
                counted.set(counted.get() + 1);
            }
        }
    });
    fixture.host(Arc::new(Host::builder(AppInfo { id: "test", name: "Test", version: "1.0" }).files(files.clone()).build()));
    fixture.settle();
    let browse: Vec<_> = fixture.by_role(Role::BUTTON).into_iter().filter(|button| fixture.tree().name_of(button.node) == Some("Browse…")).collect();
    assert_eq!(browse.len(), 2, "one Browse per entry");
    fixture.click(&browse[1]);
    fixture.settle();
    let id = ticket.get().expect("Browse filed a pick");
    assert!(files.answer_to(id, Picked::Paths(vec![PathBuf::from("/scratch")])), "the pick is open");
    fixture.idle(3);
    assert_eq!(draft.borrow().get::<Vec<Disk>>("performance.scratchDisks"), Some(disks(&["/a", "/scratch"])));
    assert_eq!(changes.get(), 1, "the answer is written once");
}
