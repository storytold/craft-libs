//! Files from the test writer read back whole, in every format, encoding and layout.

use std::io::{Cursor, Read};

use super::writer::{FolderSpec, Layout, build};
use super::{DELIVERED, layouts, pattern, sample};
use crate::{AttachmentContent, Encryption, Error, FolderKind, Format, MessageRef, Nid, PropName, PropValue, Pst, RecipientKind};

fn open(bytes: &[u8]) -> Pst<Cursor<&[u8]>> {
    Pst::open(Cursor::new(bytes)).unwrap()
}

#[test]
#[allow(clippy::cognitive_complexity, reason = "one walk over every layout, with its checks in line")]
fn every_layout_reads_back() {
    for layout in layouts() {
        let what = format!("{:?} {:?} block {}", layout.format, layout.encryption, layout.block_data);
        let bytes = build(&sample(layout.clone(), None));
        let mut pst = open(&bytes);
        assert_eq!(pst.format(), layout.format, "{what}");
        assert_eq!(pst.encryption(), layout.encryption, "{what}");
        assert!(!pst.password_required());

        let store = pst.store().unwrap();
        assert_eq!(store.display_name.as_deref(), Some("Personal Folders"), "{what}");
        let ipm = store.ipm_subtree.unwrap();

        let tree = pst.walk_folders().unwrap();
        assert!(tree.left_out.is_empty(), "{what}: {:?}", tree.left_out);
        let names: Vec<(&str, usize, bool)> = tree.folders.iter().map(|f| (f.folder.name.as_str(), f.depth, f.in_ipm_subtree)).collect();
        assert_eq!(
            names,
            vec![
                ("", 0, false),
                ("Top of Personal Folders", 1, true),
                ("Inbox", 2, true),
                ("Projects", 3, true),
                ("Calendar", 2, true),
                ("Contacts", 2, true),
                ("Search Root", 1, false),
            ],
            "{what}"
        );
        assert_eq!(tree.folders[1].folder.id, ipm);
        let kinds: Vec<FolderKind> = tree.folders.iter().map(|f| f.folder.kind.clone()).collect();
        assert_eq!(kinds[2], FolderKind::Mail);
        assert_eq!(kinds[4], FolderKind::Calendar);
        assert_eq!(kinds[5], FolderKind::Contacts);
        let inbox = tree.folders[2].folder.clone();
        assert_eq!((inbox.content_count, inbox.unread_count, inbox.has_subfolders), (2, 1, true), "{what}");
        assert_eq!(inbox.parent, Some(ipm));

        let items = pst.items(inbox.id).unwrap();
        assert!(items.left_out.is_empty(), "{what}: {:?}", items.left_out);
        let classes: Vec<Option<&str>> = items.items.iter().map(|i| i.class.as_deref()).collect();
        assert_eq!(classes, vec![Some("IPM.Note"), Some("IPM.Note")], "{what}");
        assert_eq!(items.items[0].properties.text(0x0037).as_deref(), Some("Quarterly figures"));
        assert!(pst.associated_items(inbox.id).unwrap().items.is_empty());

        let m = pst.message(&MessageRef::item(items.items[0].id)).unwrap();
        assert!(m.left_out.is_empty(), "{what}: {:?}", m.left_out);
        assert_eq!(m.class.as_deref(), Some("IPM.Note"));
        assert_eq!(m.properties.text(0x0037).as_deref(), Some("Quarterly figures"));
        assert_eq!(m.properties.text(0x1000).as_deref(), Some("Quarterly figures\r\n\r\nWritten for the tests."));
        assert_eq!(m.properties.time(0x0E06), Some(DELIVERED));
        assert_eq!(m.properties.int(0x8000), Some(42));
        let people: Vec<(Option<String>, Option<String>, RecipientKind)> = m.recipients.iter().map(|r| (r.name(), r.address(), r.kind())).collect();
        assert_eq!(
            people,
            vec![
                (Some("Ada Fernsby".into()), Some("ada@example.org".into()), RecipientKind::To),
                (Some("Bram Okonjo".into()), Some("bram@example.org".into()), RecipientKind::Cc),
                (Some("Cleo Varga".into()), Some("cleo@example.org".into()), RecipientKind::Bcc),
            ],
            "{what}"
        );

        assert_eq!(m.attachments.len(), 2, "{what}");
        let csv = &m.attachments[0];
        assert_eq!(csv.file_name.as_deref(), Some("figures.csv"));
        assert_eq!(csv.mime_type.as_deref(), Some("text/csv"));
        assert_eq!(csv.method, Some(1));
        assert_eq!(csv.content, AttachmentContent::Data { size: 20_000 });
        assert!(matches!(csv.properties.get(0x3701), Some(PropValue::Large { size: 20_000 })));
        assert_eq!(pst.attachment_bytes(&csv.id).unwrap(), pattern(20_000, 7), "{what}");
        let mut r = pst.attachment_reader(&csv.id).unwrap();
        assert_eq!(r.size(), 20_000);
        let mut first = [0u8; 10];
        r.read_exact(&mut first).unwrap();
        assert_eq!(first.to_vec(), pattern(10, 7));

        let AttachmentContent::Message(inner) = &m.attachments[1].content else { panic!("{what}: {:?}", m.attachments[1].content) };
        assert_eq!(inner.depth(), 1);
        let fwd = pst.message(inner).unwrap();
        assert_eq!(fwd.properties.text(0x0037).as_deref(), Some("Minutes of the garden committee"), "{what}");
        assert_eq!(fwd.recipients.len(), 1);
        assert_eq!(fwd.recipients[0].address().as_deref(), Some("dee@example.org"));
        assert_eq!(fwd.attachments.len(), 1);
        assert_eq!(pst.attachment_bytes(&fwd.attachments[0].id).unwrap(), b"Present: everyone.");

        let named = pst.named_properties().unwrap();
        let first = named.get(0x8000).unwrap();
        assert_eq!(first.name, PropName::Id(0x8514));
        assert_eq!(named.get(0x8001).unwrap().name, PropName::Name("Keywords".into()));
        assert_eq!(named.find(&crate::PS_PUBLIC_STRINGS, &PropName::Name("Keywords".into())), Some(0x8001));

        let mut messages = 0;
        pst.for_each_node(|n| {
            messages += usize::from(n.id.kind() == crate::node_types::NORMAL_MESSAGE);
            true
        })
        .unwrap();
        assert_eq!(messages, 4, "{what}");

        let calendar = tree.folders[4].folder.id;
        let cal = pst.items(calendar).unwrap();
        assert_eq!(cal.items[0].class.as_deref(), Some("IPM.Appointment"));
        assert!(pst.items(tree.folders[5].folder.id).unwrap().items.is_empty());
    }
}

#[test]
fn a_large_body_is_left_for_later_and_read_on_its_own() {
    let mut spec = sample(Layout::new(Format::Unicode, Encryption::Cyclic), None);
    let body: String = "All work and no play. ".repeat(30_000);
    spec.folders[0].messages[1].body = body.clone();
    let bytes = build(&spec);
    let mut pst = Pst::open_with(Cursor::new(&bytes[..]), crate::Limits { max_inline_value: 64 * 1024, ..Default::default() }).unwrap();
    let inbox = pst.walk_folders().unwrap().folders[2].folder.id;
    let item = pst.items(inbox).unwrap().items[1].id;
    let m = pst.message(&MessageRef::item(item)).unwrap();
    let size = (body.len() * 2) as u64;
    assert_eq!(m.properties.get(0x1000), Some(&PropValue::Large { size }));
    assert_eq!(pst.read_property(&m.id, 0x1000, 4 << 20).unwrap(), Some(PropValue::String(body)));
    assert_eq!(pst.read_property(&m.id, 0x1000, 1000).unwrap(), Some(PropValue::Large { size }));
}

#[test]
fn many_folders_and_items_spread_over_pages_and_blocks() {
    for format in [Format::Unicode, Format::Ansi] {
        let mut spec = sample(Layout::tiny(format, Encryption::Permute), None);
        spec.folders[2].children = (0..30).map(|i| FolderSpec { name: format!("Box {i}"), ..Default::default() }).collect();
        spec.folders[1].messages = (0..120).map(|i| super::note(&format!("Visit {i}"))).collect();
        let bytes = build(&spec);
        let mut pst = open(&bytes);
        let tree = pst.walk_folders().unwrap();
        assert!(tree.left_out.is_empty());
        let boxes: Vec<&str> =
            tree.folders.iter().filter(|f| f.depth == 3 && f.folder.name.starts_with("Box")).map(|f| f.folder.name.as_str()).collect();
        assert_eq!(boxes.len(), 30);
        assert_eq!(boxes[29], "Box 29");
        let calendar = tree.folders.iter().find(|f| f.folder.name == "Calendar").unwrap().folder.id;
        let items = pst.items(calendar).unwrap();
        assert_eq!(items.items.len(), 120);
        assert_eq!(items.items[119].properties.text(0x0037).as_deref(), Some("Visit 119"));
        let last = pst.message(&MessageRef::item(items.items[119].id)).unwrap();
        assert_eq!(last.properties.text(0x0037).as_deref(), Some("Visit 119"));
    }
}

#[test]
fn a_password_is_asked_for_before_anything_is_read() {
    for format in [Format::Unicode, Format::Ansi] {
        let bytes = build(&sample(Layout::new(format, Encryption::Permute), Some("tomato soup")));
        let mut pst = open(&bytes);
        assert!(pst.password_required());
        assert_eq!(pst.store().err(), Some(Error::PasswordRequired));
        assert_eq!(pst.walk_folders().err(), Some(Error::PasswordRequired));
        assert_eq!(pst.folder(Nid(0x122)).err(), Some(Error::PasswordRequired));
        assert_eq!(pst.items(Nid(0x8022)).err(), Some(Error::PasswordRequired));
        assert_eq!(pst.message(&MessageRef::item(Nid(0x200004))).err(), Some(Error::PasswordRequired));
        assert_eq!(pst.named_properties().err(), Some(Error::PasswordRequired));
        assert_eq!(pst.for_each_node(|_| true).err(), Some(Error::PasswordRequired));
        assert!(!pst.check_password(""));
        assert!(!pst.check_password("tomato"));
        assert_eq!(pst.unlock("Tomato soup"), Err(Error::WrongPassword));
        assert!(pst.password_required());
        assert!(pst.check_password("tomato soup"));
        pst.unlock("tomato soup").unwrap();
        assert_eq!(pst.walk_folders().unwrap().folders.len(), 7);
    }
}

#[test]
fn a_file_without_a_password_takes_any() {
    let bytes = build(&sample(Layout::new(Format::Unicode, Encryption::None), None));
    let mut pst = open(&bytes);
    assert!(pst.check_password("anything"));
    pst.unlock("anything").unwrap();
}

#[test]
fn a_search_folder_lists_no_items() {
    let bytes = build(&sample(Layout::new(Format::Unicode, Encryption::None), None));
    let mut pst = open(&bytes);
    assert_eq!(pst.items(Nid((0x500 << 5) | 0x03)).err(), Some(Error::Unsupported("a search folder's items")));
    assert_eq!(pst.items(Nid(0x21)).err(), Some(Error::Unsupported("a node that is not a folder")));
}
