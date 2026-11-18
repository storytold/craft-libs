//! Each cap of [`Limits`]: past it, the part is left out and named, and nothing panics.

use std::io::{self, Cursor, Read, Seek, SeekFrom};

use super::writer::{AttachSpec, Data, FolderSpec, Layout, Node, build, raw};
use super::{note, person, sample};
use crate::{AttachmentContent, Cap, Encryption, Error, Format, Limits, MessageRef, Nid, Part, Pst};

fn open(bytes: &[u8], limits: Limits) -> Pst<Cursor<&[u8]>> {
    Pst::open_with(Cursor::new(bytes), limits).unwrap()
}

fn unicode() -> Layout {
    Layout::new(Format::Unicode, Encryption::Permute)
}

/// A reader of a file that says it is 200 GB and holds nothing but zeros.
struct Huge(u64);

impl Read for Huge {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        buf.fill(0);
        Ok(buf.len())
    }
}

impl Seek for Huge {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.0 = match pos {
            SeekFrom::End(_) => 200 * 1024 * 1024 * 1024,
            SeekFrom::Start(n) => n,
            SeekFrom::Current(n) => self.0.saturating_add_signed(n),
        };
        Ok(self.0)
    }
}

#[test]
fn a_file_over_100_gb_is_refused_before_it_is_read() {
    assert_eq!(Pst::open(Huge(0)).err(), Some(Error::Cap(Cap::FileSize)));
    let bytes = build(&sample(unicode(), None));
    let small = Limits { max_file_size: 1000, ..Default::default() };
    assert_eq!(Pst::open_with(Cursor::new(&bytes[..]), small).err(), Some(Error::Cap(Cap::FileSize)));
}

#[test]
fn items_past_the_cap_are_counted_and_left_out() {
    let bytes = build(&sample(unicode(), None));
    let mut pst = open(&bytes, Limits { max_items: 3, ..Default::default() });
    let tree = pst.walk_folders().unwrap();
    let id = |name: &str| tree.folders.iter().find(|f| f.folder.name == name).unwrap().folder.id;
    let inbox = pst.items(id("Inbox")).unwrap();
    assert_eq!((inbox.items.len(), inbox.left_out.len()), (2, 0));
    let projects = pst.items(id("Projects")).unwrap();
    assert_eq!((projects.items.len(), projects.left_out.len()), (1, 0));
    let calendar = pst.items(id("Calendar")).unwrap();
    assert_eq!((calendar.items.len(), calendar.past_cap), (0, 1));
    assert_eq!(calendar.left_out, vec![crate::LeftOut { id: id("Calendar"), parent: None, property: None, reason: Error::Cap(Cap::Items) }]);
    // A table of more rows than the cap is refused as it is read.
    let mut small = open(&bytes, Limits { max_items: 1, ..Default::default() });
    assert_eq!(small.items(id("Inbox")).err(), Some(Error::Cap(Cap::Items)));
    // Listing a folder again doesn't count it twice.
    let again = pst.items(id("Inbox")).unwrap();
    assert_eq!((again.items.len(), again.left_out.len()), (2, 0));
}

#[test]
fn folders_past_the_cap_are_left_out() {
    let mut spec = sample(unicode(), None);
    spec.folders[2].children = (0..20).map(|i| FolderSpec { name: format!("Old {i}"), ..Default::default() }).collect();
    let bytes = build(&spec);
    let mut pst = open(&bytes, Limits { max_folders: 6, ..Default::default() });
    let tree = pst.walk_folders().unwrap();
    assert!(tree.folders.len() <= 6, "{}", tree.folders.len());
    assert!(tree.left_out.iter().any(|l| l.reason == Error::Cap(Cap::Folders)));
    // A hierarchy table with more rows than the cap is refused as a whole.
    assert_eq!(pst.subfolders(tree.folders.iter().find(|f| f.folder.name == "Contacts").unwrap().folder.id).err(), Some(Error::Cap(Cap::Folders)));
}

#[test]
fn folders_nested_past_the_cap_are_left_out() {
    let mut spec = sample(unicode(), None);
    let mut deep = FolderSpec { name: "Level 6".into(), ..Default::default() };
    for level in (2..6).rev() {
        deep = FolderSpec { name: format!("Level {level}"), children: vec![deep], ..Default::default() };
    }
    spec.folders[2].children = vec![deep];
    let bytes = build(&spec);
    let mut pst = open(&bytes, Limits { max_folder_depth: 4, ..Default::default() });
    let tree = pst.walk_folders().unwrap();
    assert!(tree.folders.iter().all(|f| f.depth <= 4));
    let left: Vec<&Error> = tree.left_out.iter().map(|l| &l.reason).collect();
    assert_eq!(left, vec![&Error::Cap(Cap::FolderDepth)]);
}

#[test]
fn recipients_and_attachments_past_their_caps_are_left_out() {
    let bytes = build(&sample(unicode(), None));
    let mut pst = open(&bytes, Limits { max_recipients: 2, max_attachments: 1, ..Default::default() });
    let inbox = pst.walk_folders().unwrap().folders[2].folder.id;
    let item = pst.items(inbox).unwrap().items[0].id;
    let m = pst.message(&MessageRef::item(item)).unwrap();
    assert_eq!(m.recipients.len(), 2);
    assert_eq!(m.attachments.len(), 1);
    let parts: Vec<&Part> = m.left_out.iter().map(|p| &p.part).collect();
    assert_eq!(parts, vec![&Part::Recipients { count: 1 }, &Part::Attachments { count: 1 }]);
    assert_eq!(m.left_out[0].reason, Error::Cap(Cap::Recipients));
    assert_eq!(m.left_out[1].reason, Error::Cap(Cap::Attachments));
}

#[test]
fn an_attachment_past_its_cap_is_refused_before_it_is_read() {
    let bytes = build(&sample(unicode(), None));
    let mut pst = open(&bytes, Limits { max_attachment_bytes: 10_000, ..Default::default() });
    let inbox = pst.walk_folders().unwrap().folders[2].folder.id;
    let item = pst.items(inbox).unwrap().items[0].id;
    let m = pst.message(&MessageRef::item(item)).unwrap();
    assert_eq!(m.attachments[0].content, AttachmentContent::Data { size: 20_000 });
    assert_eq!(pst.attachment_reader(&m.attachments[0].id).err(), Some(Error::Cap(Cap::AttachmentSize)));
    assert_eq!(pst.attachment_bytes(&m.attachments[0].id).err(), Some(Error::Cap(Cap::AttachmentSize)));
}

#[test]
fn an_item_past_its_cap_is_left_out() {
    let mut spec = sample(unicode(), None);
    spec.folders[0].messages[1].body = "Words and more words. ".repeat(500);
    let bytes = build(&spec);
    let mut pst = open(&bytes, Limits { max_item_bytes: 4096, ..Default::default() });
    let inbox = pst.walk_folders().unwrap().folders[2].folder.id;
    let items = pst.items(inbox).unwrap();
    assert_eq!(pst.message(&MessageRef::item(items.items[1].id)).err(), Some(Error::Cap(Cap::ItemSize)));
}

#[test]
fn messages_attached_past_the_depth_cap_are_left_out() {
    let mut spec = sample(unicode(), None);
    let mut nested = note("Innermost");
    for level in 0..4 {
        let mut outer = note(&format!("Wrapper {level}"));
        outer.attachments = vec![AttachSpec::Message(Box::new(nested))];
        nested = outer;
    }
    spec.folders[0].messages = vec![nested];
    let bytes = build(&spec);
    let mut pst = open(&bytes, Limits { max_embedded_depth: 2, ..Default::default() });
    let inbox = pst.walk_folders().unwrap().folders[2].folder.id;
    let first = pst.items(inbox).unwrap().items[0].id;
    let mut m = pst.message(&MessageRef::item(first)).unwrap();
    let mut depth = 0;
    loop {
        match m.attachments[0].content.clone() {
            AttachmentContent::Message(inner) => {
                depth += 1;
                m = pst.message(&inner).unwrap();
            }
            AttachmentContent::MessageTooDeep => break,
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(depth, 2);
    let too_deep = MessageRef { node: m.id.node, path: vec![Nid(1); 6] };
    assert_eq!(pst.message(&too_deep).err(), Some(Error::Cap(Cap::EmbeddedDepth)));
}

#[test]
fn an_item_with_too_many_parts_is_left_out() {
    let mut spec = sample(unicode(), None);
    let mut m = note("Many files");
    m.recipients = vec![person("Ada Fernsby", "ada", 1)];
    m.attachments = (0..40).map(|i| AttachSpec::File { name: format!("f{i}.txt"), mime: "text/plain".into(), data: vec![b'x'; 10] }).collect();
    spec.folders[0].messages = vec![m];
    let bytes = build(&spec);
    let mut pst = open(&bytes, Limits { max_subnodes: 20, ..Default::default() });
    let inbox = pst.walk_folders().unwrap().folders[2].folder.id;
    let item = pst.items(inbox).unwrap().items[0].id;
    assert_eq!(pst.message(&MessageRef::item(item)).err(), Some(Error::Cap(Cap::Subnodes)));
}

#[test]
fn data_spread_over_too_many_blocks_is_refused() {
    // The message store's data in 600 blocks of one byte each.
    let layout = Layout { block_data: 1, xblock_ids: 1000, ..Layout::new(Format::Unicode, Encryption::None) };
    let nodes = vec![Node { nid: 0x21, parent: 0, data: Data::Stream(vec![0xEC; 600]), subnodes: vec![] }];
    let bytes = raw(&layout, &nodes);
    assert_eq!(Pst::open(Cursor::new(&bytes[..])).err(), Some(Error::Cap(Cap::DataBlocks)));
}

#[test]
fn a_store_past_the_item_cap_is_refused_before_it_is_read() {
    let layout = Layout { block_data: 8000, ..Layout::new(Format::Unicode, Encryption::None) };
    let nodes = vec![Node { nid: 0x21, parent: 0, data: Data::Stream(vec![0; 40_000]), subnodes: vec![] }];
    let bytes = raw(&layout, &nodes);
    let limits = Limits { max_item_bytes: 30_000, ..Default::default() };
    assert_eq!(Pst::open_with(Cursor::new(&bytes[..]), limits).err(), Some(Error::Cap(Cap::ItemSize)));
}
