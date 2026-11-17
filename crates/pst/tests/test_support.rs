//! The test writer used the way a dependent crate's tests use it: through the `test-support`
//! feature (or `test-writer`, its alias), from outside the crate.

use std::io::Cursor;

use craft_pst::test_writer::{FileSpec, FolderSpec, Layout, MessageSpec, build};
use craft_pst::{Encryption, Format, Pst};

#[test]
fn a_file_from_the_test_writer_opens() {
    let spec = FileSpec {
        layout: Layout::new(Format::Unicode, Encryption::Permute),
        password: None,
        password_ansi: false,
        name: "Personal Folders".into(),
        folders: vec![FolderSpec {
            name: "Inbox".into(),
            class: Some("IPF.Note".into()),
            messages: vec![MessageSpec { class: "IPM.Note".into(), subject: "Lunch on Friday?".into(), ..Default::default() }],
            children: Vec::new(),
        }],
        named: Vec::new(),
        named_strings: Vec::new(),
    };
    let bytes = build(&spec);
    let mut pst = Pst::open(Cursor::new(bytes.as_slice())).unwrap();
    assert_eq!(pst.format(), Format::Unicode);
    assert_eq!(pst.encryption(), Encryption::Permute);
    assert_eq!(pst.store().unwrap().display_name.as_deref(), Some("Personal Folders"));

    let tree = pst.walk_folders().unwrap();
    let inbox = tree.folders.iter().find(|f| f.folder.name == "Inbox").unwrap();
    let items = pst.items(inbox.folder.id).unwrap();
    let subjects: Vec<Option<String>> = items.items.iter().map(|i| i.properties.text(0x0037)).collect();
    assert_eq!(subjects, vec![Some("Lunch on Friday?".to_string())]);
}
