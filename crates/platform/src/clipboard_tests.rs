//! `craft_platform::clipboard`.
#![allow(clippy::unwrap_used, reason = "tests fail by panicking")]

use std::path::PathBuf;
use std::sync::Arc;

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

// A paste read off the UI thread keeps a read's limits: the memory backend enforces none, so a
// flavour over the text limit gets through only if `read_later` drops its gate.
#[test]
fn a_later_read_keeps_the_limits_of_a_read_now() {
    let mut clipboard = Clipboard::new(MemoryClipboard::default()).limits(ClipboardLimits { text_bytes: 8, ..ClipboardLimits::DEFAULT });
    assert!(clipboard.read_later(&[TEXT]).unwrap().is_none(), "nothing copied");
    clipboard.backend_mut().write(&[Flavor::text("short")]).unwrap();
    let make = clipboard.read_later(&[TEXT]).unwrap().unwrap();
    assert_eq!(std::thread::spawn(make).join().unwrap().map(|f| f.data), Some(b"short".to_vec()));
    clipboard.backend_mut().write(&[Flavor::text("longer than eight bytes")]).unwrap();
    let make = clipboard.read_later(&[TEXT]).unwrap().unwrap();
    assert!(make().is_none(), "over the text limit");
}

// The web's clipboard is read only inside a paste event; an app that reads it at any other time
// pastes nothing.
#[test]
fn only_a_clipboard_outside_paste_events_is_readable_any_time() {
    assert!(Clipboard::new(MemoryClipboard::default()).readable_any_time());
    assert!(!Clipboard::new(EventClipboard::new(|| 0, |_| Ok(0))).readable_any_time());
}

// A host picks its backend at run time and hands it over boxed; the box answers what the backend
// overrides, not the trait's defaults.
#[test]
fn a_boxed_backend_answers_as_the_backend_it_holds() {
    let event: Box<dyn ClipboardBackend> = Box::new(EventClipboard::new(|| 0, |_| Ok(0)));
    assert!(!Clipboard::new(event).readable_any_time());
    let mut memory: Box<dyn ClipboardBackend> = Box::new(MemoryClipboard::default());
    memory.write(&[Flavor::text("held")]).unwrap();
    assert_eq!(memory.formats(), vec![TEXT.to_owned()]);
}

/// A backend that takes longer to drop than the probe's old 250 ms wait, as an X11 handle does
/// while it hands the copy to the clipboard manager.
struct SlowToDrop(Arc<std::sync::atomic::AtomicBool>);

impl ClipboardBackend for SlowToDrop {
    fn write(&mut self, _flavors: &[Flavor]) -> Result<usize, ClipboardError> {
        Ok(0)
    }
    fn read(&mut self, _mimes: &[&str], _limit: u64) -> Result<Option<Flavor>, ClipboardError> {
        Ok(None)
    }
    fn read_image(&mut self, _limit: u64) -> Result<Option<ClipboardImage>, ClipboardError> {
        Ok(None)
    }
    fn read_files(&mut self, _limit: usize) -> Result<Option<Vec<PathBuf>>, ClipboardError> {
        Ok(None)
    }
    fn has(&mut self, _mimes: &[&str]) -> bool {
        false
    }
}

impl Drop for SlowToDrop {
    fn drop(&mut self) {
        std::thread::sleep(std::time::Duration::from_millis(400));
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

// Quitting drops the probe before the app's own handle: a probe that stopped waiting early would
// leave its handle the last one alive, and on X11 the copy would be lost at exit.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn dropping_the_probe_waits_until_its_handle_has_dropped() {
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let handle = dropped.clone();
    let probe =
        ClipboardProbe::start(Box::new(move || Box::new(SlowToDrop(handle)) as Box<dyn ClipboardBackend>), vec![TEXT], craft_host::Wake::new(|| {}))
            .unwrap();
    drop(probe);
    assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
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
