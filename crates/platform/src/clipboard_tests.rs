//! `craft_platform::clipboard`.
#![allow(clippy::unwrap_used, reason = "tests fail by panicking")]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::clipboard::{rgba8_image, *};

/// A backend that holds whatever the last write or `held` assignment left, and reads without
/// enforcing any limit (the [`Clipboard`] must).
#[derive(Clone, Default)]
struct Fake(Arc<Mutex<Vec<Flavor>>>);

impl Fake {
    fn copy_elsewhere(&self, flavors: Vec<Flavor>) {
        *self.0.lock().unwrap() = flavors;
    }
}

impl ClipboardBackend for Fake {
    fn write(&mut self, flavors: &[Flavor]) -> Result<usize, ClipboardError> {
        *self.0.lock().unwrap() = flavors.to_vec();
        Ok(0)
    }
    fn read(&mut self, mimes: &[&str], _limit: u64) -> Result<Option<Flavor>, ClipboardError> {
        let held = self.0.lock().unwrap();
        Ok(mimes.iter().find_map(|mime| held.iter().find(|f| f.mime == *mime).cloned()))
    }
    fn read_image(&mut self, _limit: u64) -> Result<Option<ClipboardImage>, ClipboardError> {
        Ok(self.0.lock().unwrap().iter().find(|f| f.mime == RGBA8).and_then(|f| rgba8_image(&f.data)))
    }
    fn read_files(&mut self, _limit: usize) -> Result<Option<Vec<PathBuf>>, ClipboardError> {
        Ok(None)
    }
    fn has(&mut self, mimes: &[&str]) -> bool {
        self.0.lock().unwrap().iter().any(|f| mimes.contains(&&*f.mime))
    }
}

fn image(seed: u8) -> ClipboardImage {
    ClipboardImage { width: 4, height: 2, rgba: (0..32).map(|i| i as u8 ^ seed).collect() }
}

// While synthetic, write_text and write_image answer Fenced and the backend keeps the user's
// copy; reads still work; after set_synthetic(false) writes go through.
#[test]
fn synthetic_input_never_replaces_what_the_user_copied() {
    let fake = Fake::default();
    fake.copy_elsewhere(vec![Flavor::text("the user's")]);
    let mut clipboard = Clipboard::new(fake.clone());
    clipboard.set_synthetic(true);
    assert_eq!(clipboard.write_text("injected").unwrap(), Written::Fenced);
    assert_eq!(clipboard.write_image(&image(1)).unwrap(), Written::Fenced);
    assert_eq!(clipboard.read_text().unwrap().as_deref(), Some("the user's"));
    clipboard.set_synthetic(false);
    assert_eq!(clipboard.write_text("mine").unwrap(), Written::Written);
    assert_eq!(clipboard.read_text().unwrap().as_deref(), Some("mine"));
}

// Copied files reach the system clipboard as a file list holding the same paths, spaces and
// non-ASCII names included; a URI that names no local file is never written as one, and a list
// with no file in it leaves the write to the next flavour.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_file_list_is_written_as_files() {
    let paths = vec![std::env::temp_dir().join("a b#1.pdf"), std::env::temp_dir().join("café 100%.png")];
    let files = |plan: Result<(usize, Put), ClipboardError>| match plan {
        Ok((index, Put::Files(files))) => Some((index, files)),
        _ => None,
    };
    assert_eq!(files(plan_write(&[Flavor::files(&paths), Flavor::text("names")], None)), Some((0, paths.clone())));

    let mut mixed = b"# copied\r\nhttps://example.com/a.pdf\r\n".to_vec();
    mixed.extend(Flavor::files(&paths[..1]).data);
    assert_eq!(files(plan_write(&[Flavor::new(URI_LIST, mixed)], None)), Some((0, paths[..1].to_vec())));

    let web = Flavor::new(URI_LIST, b"https://example.com/a.pdf\r\n".to_vec());
    assert!(matches!(plan_write(&[web, Flavor::text("names")], None), Ok((1, Put::Text(text))) if text == "names"));
}
