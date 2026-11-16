//! Tests of the .pst reader. Every file here is made by the test writer (`writer.rs`) from
//! invented people at `example.org`, or is a sample printed in [MS-PST]; none was made by
//! Outlook, and none is a real archive.

mod accounting;
mod caps;
mod cost;
mod hostile;
mod list_stream;
mod properties;
mod recur;
mod roundtrip;
mod salvage;
mod spec_samples;
pub use crate::test_writer as writer;

use std::io::{Cursor, Read};

use writer::{AttachSpec, FileSpec, FolderSpec, Layout, MessageSpec, int32, unicode};

use crate::{AttachmentContent, Encryption, Format, Limits, MessageRef, Pst};

/// 2026-11-04 09:30 UTC as a FILETIME.
pub const DELIVERED: u64 = 134_067_498_000_000_000;

pub fn person(name: &str, user: &str, kind: i32) -> (String, String, i32) {
    (name.to_string(), format!("{user}@example.org"), kind)
}

/// Bytes that are not all alike, so that a misplaced block shows.
pub fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed) ^ (i >> 8) as u8).collect()
}

pub fn note(subject: &str) -> MessageSpec {
    MessageSpec {
        class: "IPM.Note".into(),
        subject: subject.into(),
        body: format!("{subject}\r\n\r\nWritten for the tests."),
        read: false,
        delivered: DELIVERED,
        ..Default::default()
    }
}

/// A file with a few folders, messages with recipients and attachments, and attached messages.
pub fn sample(layout: Layout, password: Option<&str>) -> FileSpec {
    let mut report = note("Quarterly figures");
    report.recipients = vec![person("Ada Fernsby", "ada", 1), person("Bram Okonjo", "bram", 2), person("Cleo Varga", "cleo", 3)];
    report.read = true;
    let mut forwarded = note("Minutes of the garden committee");
    forwarded.recipients = vec![person("Dee Halloran", "dee", 1)];
    forwarded.attachments = vec![AttachSpec::File { name: "minutes.txt".into(), mime: "text/plain".into(), data: b"Present: everyone.".to_vec() }];
    report.attachments = vec![
        AttachSpec::File { name: "figures.csv".into(), mime: "text/csv".into(), data: pattern(20_000, 7) },
        AttachSpec::Message(Box::new(forwarded)),
    ];
    report.extra = vec![(0x8000, int32(42)), (0x0E1D, unicode("Quarterly figures"))];
    let plain = note("Lunch on Friday?");
    let mut appointment = note("Dentist");
    appointment.class = "IPM.Appointment".into();
    FileSpec {
        layout,
        password: password.map(str::to_string),
        password_ansi: false,
        name: "Personal Folders".into(),
        folders: vec![
            FolderSpec {
                name: "Inbox".into(),
                class: Some("IPF.Note".into()),
                messages: vec![report, plain],
                children: vec![FolderSpec {
                    name: "Projects".into(),
                    class: Some("IPF.Note".into()),
                    messages: vec![note("Shed roof")],
                    children: vec![],
                }],
            },
            FolderSpec { name: "Calendar".into(), class: Some("IPF.Appointment".into()), messages: vec![appointment], children: vec![] },
            FolderSpec { name: "Contacts".into(), class: Some("IPF.Contact".into()), messages: vec![], children: vec![] },
        ],
        named: vec![([0x08, 0x20, 0x06, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46], 0x8514)],
        named_strings: vec![(crate::PS_PUBLIC_STRINGS, "Keywords".into())],
    }
}

pub fn layouts() -> Vec<Layout> {
    let mut out = Vec::new();
    for format in [Format::Unicode, Format::Ansi] {
        for enc in [Encryption::None, Encryption::Permute, Encryption::Cyclic] {
            out.push(Layout::new(format, enc));
            out.push(Layout::tiny(format, enc));
        }
    }
    out
}

/// Reads everything a file has, through every call of the API, and ignores what fails: the
/// hostile tests only ask that it ends without a panic.
pub fn exercise(bytes: &[u8], limits: Limits) -> usize {
    let Ok(mut pst) = Pst::open_with(Cursor::new(bytes), limits) else { return 0 };
    let mut seen = 0usize;
    let _ = pst.unlock("");
    let _ = pst.store();
    let _ = pst.named_properties().map(|m| seen += m.len());
    let _ = pst.for_each_node(|_| {
        seen += 1;
        seen < 100_000
    });
    let Ok(tree) = pst.walk_folders() else { return seen };
    for entry in &tree.folders {
        let Ok(items) = pst.items(entry.folder.id) else { continue };
        let _ = pst.associated_items(entry.folder.id);
        for item in items.items {
            let mut queue = vec![MessageRef::item(item.id)];
            while let Some(m) = queue.pop() {
                let Ok(message) = pst.message(&m) else { continue };
                seen += 1 + message.recipients.len();
                let _ = pst.read_property(&m, 0x1000, 1 << 20);
                for a in message.attachments {
                    match &a.content {
                        AttachmentContent::Message(inner) if queue.len() < 64 => queue.push(inner.clone()),
                        AttachmentContent::Data { .. } | AttachmentContent::Object { .. } => {
                            if let Ok(mut r) = pst.attachment_reader(&a.id) {
                                let mut sink = Vec::new();
                                let _ = r.read_to_end(&mut sink);
                                seen += sink.len().min(1);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    seen
}

/// The first item of the folder named Inbox.
pub fn first_item(pst: &mut Pst<Cursor<&[u8]>>) -> crate::Nid {
    let walk = pst.walk_folders().unwrap();
    let inbox = walk.folders.iter().find(|f| f.folder.name == "Inbox").unwrap().folder.id;
    pst.items(inbox).unwrap().items[0].id
}
