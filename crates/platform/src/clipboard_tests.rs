//! `craft_platform::clipboard`.
#![allow(clippy::unwrap_used, reason = "tests fail by panicking")]

use crate::clipboard::*;

fn image(seed: u8) -> ClipboardImage {
    ClipboardImage { width: 4, height: 2, rgba: (0..32).map(|i| i as u8 ^ seed).collect() }
}

// While synthetic, write_text and write_image answer Fenced and the backend keeps the user's
// copy; reads still work; after set_synthetic(false) writes go through.
#[test]
fn synthetic_input_never_replaces_what_the_user_copied() {
    let mut clipboard = Clipboard::new(MemoryClipboard::default());
    // Copied elsewhere: straight to the backend, past the fence.
    clipboard.backend_mut().write(&[Flavor::text("the user's")]).unwrap();
    clipboard.set_synthetic(true);
    assert_eq!(clipboard.write_text("injected").unwrap(), Written::Fenced);
    assert_eq!(clipboard.write_image(&image(1)).unwrap(), Written::Fenced);
    assert_eq!(clipboard.read_text().unwrap().as_deref(), Some("the user's"));
    clipboard.set_synthetic(false);
    assert_eq!(clipboard.write_text("mine").unwrap(), Written::Written);
    assert_eq!(clipboard.read_text().unwrap().as_deref(), Some("mine"));
}

// Copied files arrive as a text/uri-list. A backend that holds the list but answers no files
// turns every file paste in tests, and in hosts without a system clipboard, into a no-op.
#[test]
fn a_held_uri_list_reads_as_files() {
    let mut clipboard = Clipboard::new(MemoryClipboard::default());
    let list = "# copied\r\nfile:///tmp/a%20b.txt\r\nfile:///tmp/c.csv\r\n";
    clipboard.backend_mut().write(&[Flavor::new(URI_LIST, list.as_bytes().to_vec())]).unwrap();
    let files = clipboard.read_files().unwrap().unwrap();
    assert_eq!(files.len(), 2);
    assert!(files[0].ends_with("a b.txt") && files[1].ends_with("c.csv"));
}
