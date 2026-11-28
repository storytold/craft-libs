//! The files the OS hovers over or drops on the running viewport this pass, and the inbox of dropped files whose bytes
//! arrive after their drop.

use super::memory::Service;
use super::pass;
use crate::runtime::drag::OsFile;
use egui::{Context, DroppedFileHandle, ViewportId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// What the OS reported for the running pass.
pub(crate) struct Seen {
    /// The files held over the window: a drop is under way.
    pub hovered: Vec<OsFile>,
    /// The files dropped on the window this pass. Their media type is empty: egui reports it only while the files hover,
    /// and the drag step carries it over. A file without a path carries its bytes.
    pub dropped: Vec<OsFile>,
    /// Files dropped on the window are still being read: the drop happened, and its files arrive whole in a later pass.
    pub reading: bool,
}

/// The dropped files of each viewport whose bytes are read after the drop: the web reads a pathless file only
/// asynchronously. Each drop is delivered whole, once its last read ends.
#[derive(Default)]
struct Inbox(HashMap<ViewportId, Reads>);

/// The reads of one viewport's drop.
#[derive(Default)]
struct Reads {
    /// Reads not ended yet.
    open: usize,
    /// The drop's files read so far, and those it had at hand, in the order the OS listed them.
    files: Vec<OsFile>,
}

/// Opens `count` reads of files dropped on `viewport`, in the pass of the drop: the drop waits for them.
#[cfg(any(target_arch = "wasm32", feature = "testing"))]
pub(crate) fn open_reads(ctx: &Context, viewport: ViewportId, count: usize) {
    Service::<Inbox>::run(ctx, |inbox| inbox.0.entry(viewport).or_default().open += count);
}

/// Ends one read of a file dropped on `viewport`: `file` carries its bytes, `None` when the read failed. The next pass
/// of `viewport` delivers the drop once no read is open, so this asks for that pass.
#[cfg(any(target_arch = "wasm32", feature = "testing"))]
pub(crate) fn post(ctx: &Context, viewport: ViewportId, file: Option<OsFile>) {
    Service::<Inbox>::run(ctx, |inbox| {
        let reads = inbox.0.entry(viewport).or_default();
        reads.open = reads.open.saturating_sub(1);
        reads.files.extend(file);
    });
    super::repaint::request_of(ctx, viewport);
}

/// The files of `viewport`'s drop once its last read ended, taken; and whether a read is still open.
fn take_read(ctx: &Context, viewport: ViewportId) -> (Vec<OsFile>, bool) {
    Service::<Inbox>::run(ctx, |inbox| {
        let Some(reads) = inbox.0.get_mut(&viewport) else { return (Vec::new(), false) };
        if reads.open > 0 {
            return (Vec::new(), true);
        }
        let files = inbox.0.remove(&viewport).map(|reads| reads.files).unwrap_or_default();
        (files, false)
    })
}

/// The hovered and dropped files of the running pass. egui hands a frame's dropped files to its first pass only, so a
/// second pass of the frame sees none, and the inbox is drained by the pass that finds its drop complete.
pub(crate) fn seen(ctx: &Context) -> Seen {
    let viewport = pass::current_viewport(ctx);
    let (hovered, dropped) = ctx.input(|input| {
        let hovered = input
            .raw
            .hovered_files
            .iter()
            .map(|file| OsFile {
                name: file.path.as_deref().map(name_of).unwrap_or_default(),
                path: file.path.clone(),
                mime: file.mime.clone(),
                bytes: None,
            })
            .collect::<Vec<_>>();
        (hovered, input.raw.dropped_files.clone())
    });
    let mut at_hand = Vec::new();
    let mut later = Vec::new();
    for file in dropped {
        match at_once(&file) {
            Some(os) => at_hand.push(os),
            None => later.push(file),
        }
    }
    if !later.is_empty() {
        // The drop arrives whole: the files at hand wait in the inbox for those still being read.
        Service::<Inbox>::run(ctx, |inbox| inbox.0.entry(viewport).or_default().files.append(&mut at_hand));
        read_later(ctx, viewport, later);
    }
    let (read, reading) = take_read(ctx, viewport);
    at_hand.extend(read);
    Seen { hovered, dropped: at_hand, reading }
}

/// `file` as the drop delivers it at once: its path when that is absolute, or else its bytes. A browser hands a file
/// over by a relative name that locates nothing, with its bytes. `None` for a file without an absolute path whose bytes
/// can be read only asynchronously (the web).
fn at_once(file: &DroppedFileHandle) -> Option<OsFile> {
    let path = file.path();
    let name = name_of(path);
    if path.is_absolute() {
        return Some(OsFile { name, path: Some(path.to_path_buf()), mime: String::new(), bytes: None });
    }
    let bytes = bytes_now(file)?;
    // A relative path whose bytes cannot be read is all the drop has: it is passed on, and opening it reports why it fails.
    let path = (bytes.is_none() && !path.as_os_str().is_empty()).then(|| path.to_path_buf());
    Some(OsFile { name, path, mime: String::new(), bytes })
}

/// The bytes of a pathless dropped file, read now; none inside when the read fails.
#[cfg(not(target_arch = "wasm32"))]
fn bytes_now(file: &DroppedFileHandle) -> Option<Option<std::sync::Arc<[u8]>>> {
    Some(file.bytes().ok().map(Into::into))
}

/// The web reads a dropped file's bytes only asynchronously: never now.
#[cfg(target_arch = "wasm32")]
fn bytes_now(_file: &DroppedFileHandle) -> Option<Option<std::sync::Arc<[u8]>>> {
    None
}

/// Reads `files`, dropped on `viewport`, off the pass: each read posts its file into the inbox when it ends.
#[cfg(target_arch = "wasm32")]
fn read_later(ctx: &Context, viewport: ViewportId, files: Vec<DroppedFileHandle>) {
    open_reads(ctx, viewport, files.len());
    for file in files {
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let bytes = file.bytes_async().await.ok();
            let name = name_of(file.path());
            post(&ctx, viewport, bytes.map(|bytes| OsFile { name, path: None, mime: String::new(), bytes: Some(bytes.into()) }));
        });
    }
}

/// Every native dropped file gives its path or its bytes at once (`bytes_now`), so nothing is read later.
#[cfg(not(target_arch = "wasm32"))]
fn read_later(_ctx: &Context, _viewport: ViewportId, _files: Vec<DroppedFileHandle>) {}

/// The file name of `path`, empty when it has none.
fn name_of(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

/// A dropped file that exists only as a path: what injected input and the test fixture drop on a window.
#[derive(Debug)]
pub(crate) struct DroppedPath(pub(crate) PathBuf);

impl egui::DroppedFile for DroppedPath {
    fn path(&self) -> &Path {
        &self.0
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|error| error.to_string())
    }

    #[cfg(target_arch = "wasm32")]
    fn bytes_async(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, String>> + '_>> {
        Box::pin(std::future::ready(Err("there is no file system on the web".to_owned())))
    }
}

/// A dropped file the test fixture hands over by a relative name with its bytes at hand, as a browser does.
#[cfg(feature = "testing")]
#[derive(Debug)]
pub(crate) struct DroppedBytes {
    pub(crate) name: PathBuf,
    pub(crate) bytes: Vec<u8>,
}

#[cfg(feature = "testing")]
impl egui::DroppedFile for DroppedBytes {
    fn path(&self) -> &Path {
        &self.name
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn bytes(&self) -> Result<Vec<u8>, String> {
        Ok(self.bytes.clone())
    }

    #[cfg(target_arch = "wasm32")]
    fn bytes_async(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, String>> + '_>> {
        Box::pin(std::future::ready(Ok(self.bytes.clone())))
    }
}
