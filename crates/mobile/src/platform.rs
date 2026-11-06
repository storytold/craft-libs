//! What an app asks of the native shell, and how answers come back.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::OnceLock;

/// Where the app keeps its own data (recovery files, settings), set once by the shell before the
/// app starts: Application Support on iOS, `filesDir` on Android.
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Record the app's data folder. Only the first call counts.
pub(crate) fn set_data_dir(dir: PathBuf) {
    if DATA_DIR.set(dir).is_err() {
        log::debug!("craft-mobile: the data folder was already set");
    }
}

/// Where the shell can write a document back: a security-scoped file URL on iOS, a content URI
/// with write permission on Android. Opaque to the app; only the shell that made it reads it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Location(String);

impl Location {
    /// A location as the shell spells it; `None` when empty.
    #[must_use]
    pub fn new(location: impl Into<String>) -> Option<Self> {
        let location = location.into();
        (!location.is_empty()).then_some(Self(location))
    }

    /// The shell's spelling.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a document path for a [`Location`] starts with.
const IN_PLACE_ROOT: &str = "inplace://";

/// A file's name and contents, as a picker or the system hands it over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickedFile {
    /// The file name, without folders.
    pub name: String,
    /// The whole file.
    pub bytes: Vec<u8>,
    /// Where it can be written back, when the shell may write there; `None` for a copy (a photo,
    /// a share, a provider that grants no writing).
    pub location: Option<Location>,
}

impl PickedFile {
    /// The path the app keeps as the document's own file when it can be written back:
    /// `inplace://<id>/<name>`, ending in the file's name so the app's naming, format and
    /// extension rules work as for a path on disk. [`Platform::save`] to it writes the original.
    /// The id is a hash of the location, so the same file gets the same path in every session.
    #[must_use]
    pub fn document_path(&self) -> Option<String> {
        self.location.as_ref().map(|location| document_path(location, &self.name))
    }
}

/// `text` (a status line, a message) with each document path in it shown as the file's name, as
/// the user knows it: `Saved inplace://0123456789abcdef/a.psd` becomes `Saved a.psd`.
#[must_use]
pub fn with_file_names(text: &str) -> Cow<'_, str> {
    if !text.contains(IN_PLACE_ROOT) {
        return Cow::Borrowed(text);
    }
    let mut shown = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(IN_PLACE_ROOT) {
        let (before, path) = rest.split_at(start);
        shown.push_str(before);
        let after_root = path.get(IN_PLACE_ROOT.len()..).unwrap_or_default();
        // The id is hex up to the next `/`; the name follows it.
        match after_root.split_once('/') {
            Some((id, name)) if !id.is_empty() && id.chars().all(|c| c.is_ascii_hexdigit()) => rest = name,
            _ => {
                shown.push_str(IN_PLACE_ROOT);
                rest = after_root;
            }
        }
    }
    shown.push_str(rest);
    Cow::Owned(shown)
}

/// See [`PickedFile::document_path`]. Separators in the name become `_`, so the path's last
/// component is always the whole name.
fn document_path(location: &Location, name: &str) -> String {
    let name: String = name.chars().map(|c| if matches!(c, '/' | '\\' | '\n' | '\0') { '_' } else { c }).collect();
    format!("{IN_PLACE_ROOT}{:016x}/{name}", fnv1a(location.as_str()))
}

/// FNV-1a, 64-bit: stable across runs and Rust versions, unlike `DefaultHasher`.
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3))
}

/// Where [`Platform::save`] sent the bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveRoute {
    /// Back to the document's own file. The shell reports how it went through
    /// [`crate::CraftApp::written`]; if it fails, the shell offers the exporter instead.
    InPlace,
    /// To the exporter: the user picks a place (or cancels), and nothing reports back.
    Exporter,
}

/// How a write back to a document's own file ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteOutcome {
    /// The document path [`Platform::save`] was given.
    pub path: String,
    /// `Err` with the shell's reason when the file couldn't be written; the shell then offers
    /// the bytes to the exporter, so they aren't lost.
    pub result: Result<(), String>,
}

/// What the shell read from the clipboard for a paste the user asked for, kept for the frame that
/// handles it ([`Platform::pasted`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pasted {
    /// Text (empty when the clipboard held nothing the app takes).
    Text(String),
    /// An image from another app (or one the app copied that was since copied again elsewhere):
    /// RGBA8, unpremultiplied, exactly as read. egui's `Event::PasteImage` has it premultiplied.
    Image {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// `width × height × 4` bytes.
        rgba: Vec<u8>,
    },
    /// The image the app copied last ([`Platform::copy_image`]) is still on the clipboard: paste
    /// the app's own copy, which keeps what the clipboard image can't (layers, say).
    OwnImage,
}

/// Gets the files the user picked: empty when they cancelled.
pub type PickReply = Box<dyn FnOnce(Vec<PickedFile>)>;

/// A request for the shell, drained after every frame.
#[derive(Debug, PartialEq)]
pub(crate) enum Request {
    /// Show the document picker; answer with the chosen files.
    PickFiles { multiple: bool, extensions: Vec<String> },
    /// Show the exporter for these bytes; `name` is the suggested file name.
    Export { name: String, bytes: Vec<u8> },
    /// Write these bytes over the file at `location` (its name is `name`), then report back.
    WriteInPlace { location: Location, name: String, bytes: Vec<u8> },
    /// The panel state changed (a [`crate::Snapshot`] as JSON).
    State(String),
    /// Put this text on the system clipboard.
    CopyText(String),
    /// Put this image on the system clipboard (RGBA8, unpremultiplied).
    CopyImage { width: u32, height: u32, rgba: Vec<u8> },
    /// Open this URL in the system browser (or the app that handles it).
    OpenUrl(String),
    /// Show (`true`) or hide the on-screen keyboard: a text field gained or lost focus.
    Keyboard(bool),
    /// The app started (`true`) or finished work that must complete even in the background
    /// ([`crate::CraftApp::busy`]): hold background time for exactly that span.
    Busy(bool),
    /// Read the clipboard for a paste the user asked for, and answer with what it holds
    /// ([`crate::Input::PasteText`], [`crate::Input::PasteImage`], [`crate::Input::PasteOwnImage`]).
    Paste,
    /// Show the share sheet for these bytes, named `name`.
    Share { name: String, bytes: Vec<u8> },
    /// Show the system photo picker; the chosen photos open as copies ([`crate::CraftApp::open_file`]).
    PickPhotos,
}

impl Request {
    /// This request as the shells receive it, the same on every platform:
    ///
    /// | kind | request       | text                           | data     |
    /// |------|---------------|--------------------------------|----------|
    /// | 0    | pick one file | allowed extensions, `,`-joined | —        |
    /// | 1    | pick files    | allowed extensions, `,`-joined | —        |
    /// | 2    | export        | suggested file name            | the file |
    /// | 3    | panel state   | a `Snapshot` as JSON           | —        |
    /// | 4    | copy text     | the text                       | —        |
    /// | 5    | copy image    | `width×height`                 | RGBA8    |
    /// | 6    | open URL      | the URL                        | —        |
    /// | 7    | keyboard      | `1` show, `0` hide             | —        |
    /// | 8    | write in place| location, `\n`, file name      | the file |
    /// | 9    | busy          | `1` busy, `0` idle again       | —        |
    /// | 10   | paste         | —                              | —        |
    /// | 11   | share         | file name                      | the file |
    /// | 12   | pick photos   | —                              | —        |
    ///
    /// Kind 10 is answered with the clipboard's contents (`craft_paste_*`, `CraftNative.paste*`),
    /// text first while a text field is focused, else an image first; an image the app copied
    /// itself and that is still there comes back as `craft_paste_own`, unread; an empty
    /// clipboard as empty text.
    ///
    /// For kind 8 the shell answers with the location and an error or none (`craft_write_done`,
    /// `CraftNative.writeDone`). The name never holds a line break, so it follows the last one.
    pub(crate) fn wire(&self) -> (i32, Cow<'_, str>, Option<&[u8]>) {
        match self {
            Request::PickFiles { multiple, extensions } => (i32::from(*multiple), extensions.join(",").into(), None),
            Request::Export { name, bytes } => (2, name.as_str().into(), Some(bytes)),
            Request::State(json) => (3, json.as_str().into(), None),
            Request::CopyText(text) => (4, text.as_str().into(), None),
            Request::CopyImage { width, height, rgba } => (5, format!("{width}×{height}").into(), Some(rgba)),
            Request::OpenUrl(url) => (6, url.as_str().into(), None),
            Request::Keyboard(show) => (7, if *show { "1" } else { "0" }.into(), None),
            Request::WriteInPlace { location, name, bytes } => (8, format!("{location}\n{name}").into(), Some(bytes)),
            Request::Busy(busy) => (9, if *busy { "1" } else { "0" }.into(), None),
            Request::Paste => (10, "".into(), None),
            Request::Share { name, bytes } => (11, name.as_str().into(), Some(bytes)),
            Request::PickPhotos => (12, "".into(), None),
        }
    }
}

#[derive(Default)]
struct Shared {
    out: Vec<Request>,
    pick: Option<PickReply>,
    /// Document paths handed out this session, and the locations they write to. Only these are
    /// written in place: a path the app derived (another extension) or kept from an earlier
    /// session (crash recovery) goes to the exporter.
    locations: HashMap<String, Location>,
    /// Writes in place the shell hasn't answered yet, oldest first.
    writing: VecDeque<(Location, String)>,
    /// The clipboard as the shell read it for this frame's paste.
    pasted: Option<Rc<Pasted>>,
}

/// The app's handle on the native shell. Cheap to clone; everything runs on the app's thread.
#[derive(Clone, Default)]
pub struct Platform {
    shared: Rc<RefCell<Shared>>,
}

impl Platform {
    /// Ask for files to open. `extensions` without dots; empty for every type the app declares.
    /// One picker at a time: a new request cancels the one still open (its reply gets nothing).
    pub fn pick_files(&self, multiple: bool, extensions: Vec<String>, reply: PickReply) {
        let earlier = {
            let mut shared = self.shared.borrow_mut();
            shared.out.push(Request::PickFiles { multiple, extensions });
            shared.pick.replace(reply)
        };
        if let Some(earlier) = earlier {
            earlier(Vec::new());
        }
    }

    /// Offer `bytes` to save where the user chooses, suggesting `name`. Nothing reports back
    /// whether they saved, so the app must not treat the document as saved.
    pub fn export(&self, name: String, bytes: Vec<u8>) {
        self.shared.borrow_mut().out.push(Request::Export { name, bytes });
    }

    /// Save `bytes` to `path`: written back over the original when `path` is a
    /// [`PickedFile::document_path`] handed out this session, else offered to the exporter
    /// under `path`'s file name. Writes in place report back through [`crate::CraftApp::written`].
    #[must_use = "only a write in place reports back, so only then may the app wait to mark the document saved"]
    pub fn save(&self, path: &str, bytes: Vec<u8>) -> SaveRoute {
        let mut shared = self.shared.borrow_mut();
        let name = file_name(path);
        if let Some(location) = shared.locations.get(path).cloned() {
            shared.writing.push_back((location.clone(), path.to_string()));
            shared.out.push(Request::WriteInPlace { location, name, bytes });
            SaveRoute::InPlace
        } else {
            shared.out.push(Request::Export { name, bytes });
            SaveRoute::Exporter
        }
    }

    /// Whether a write in place is still on its way to the document's file (the shell hasn't
    /// answered yet): the app counts as busy until it has.
    #[must_use]
    pub fn writing(&self) -> bool {
        !self.shared.borrow().writing.is_empty()
    }

    /// Whether [`Self::save`] to `path` writes in place.
    #[must_use]
    pub fn writes_in_place(&self, path: &str) -> bool {
        self.shared.borrow().locations.contains_key(path)
    }

    /// Put an image on the system clipboard (RGBA8, unpremultiplied, `width × height`).
    pub fn copy_image(&self, width: u32, height: u32, rgba: Vec<u8>) {
        self.push(Request::CopyImage { width, height, rgba });
    }

    /// Show the share sheet for `bytes` as a file named `name` (nearby devices, messages, another app).
    /// Nothing reports back, so the app must not treat the document as saved.
    pub fn share(&self, name: String, bytes: Vec<u8>) {
        self.push(Request::Share { name, bytes });
    }

    /// Show the system photo picker (Photos on iPadOS, the Photo Picker on Android). The photos
    /// chosen arrive through [`crate::CraftApp::open_file`] as copies, in their original encoding.
    pub fn pick_photos(&self) {
        self.push(Request::PickPhotos);
    }

    /// Whether the shell has a photo picker for [`Self::pick_photos`]: both shells do.
    #[must_use]
    pub fn has_photo_picker(&self) -> bool {
        cfg!(any(target_os = "ios", target_os = "android"))
    }

    /// Ask the shell to read the clipboard for a paste the user asked for. The answer arrives as
    /// input on a later frame: egui's `Event::Paste` or `Event::PasteImage`, with
    /// [`Self::pasted`] holding what was read during that frame. egui's
    /// `ViewportCommand::RequestPaste` does the same. On iPadOS reading another app's copy may ask
    /// the user to allow it, which is why the clipboard is read only when they paste.
    pub fn request_paste(&self) {
        self.push(Request::Paste);
    }

    /// What the shell read from the clipboard for a paste, during the frame that handles it (the
    /// one whose input carries the paste event); `None` in any other frame.
    #[must_use]
    pub fn pasted(&self) -> Option<Rc<Pasted>> {
        self.shared.borrow().pasted.clone()
    }

    pub(crate) fn set_pasted(&self, pasted: Option<Pasted>) {
        self.shared.borrow_mut().pasted = pasted.map(Rc::new);
    }

    /// Open a link in the system browser (or the app that handles it).
    pub fn open_url(&self, url: String) {
        self.push(Request::OpenUrl(url));
    }

    /// The app's own data folder (recovery files, settings), when the shell gave one.
    #[must_use]
    pub fn data_dir(&self) -> Option<&'static Path> {
        DATA_DIR.get().map(PathBuf::as_path)
    }

    pub(crate) fn push(&self, request: Request) {
        self.shared.borrow_mut().out.push(request);
    }

    pub(crate) fn state(&self, json: String) {
        self.shared.borrow_mut().out.push(Request::State(json));
    }

    /// Note a file that arrived with a location, so saves to its document path write it back.
    pub(crate) fn remember(&self, file: &PickedFile) {
        if let (Some(location), Some(path)) = (&file.location, file.document_path()) {
            self.shared.borrow_mut().locations.insert(path, location.clone());
        }
    }

    /// The shell finished the oldest write to `location`: the document path it was for.
    pub(crate) fn finish_write(&self, location: &Location) -> Option<String> {
        let mut shared = self.shared.borrow_mut();
        let index = shared.writing.iter().position(|(l, _)| l == location)?;
        shared.writing.remove(index).map(|(_, path)| path)
    }

    pub(crate) fn answer_pick(&self, files: Vec<PickedFile>) {
        for file in &files {
            self.remember(file);
        }
        let reply = self.shared.borrow_mut().pick.take();
        if let Some(reply) = reply {
            reply(files);
        }
    }

    pub(crate) fn take_requests(&self) -> Vec<Request> {
        std::mem::take(&mut self.shared.borrow_mut().out)
    }
}

/// The last component of `path` (all of it when it has none).
fn file_name(path: &str) -> String {
    Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn located(name: &str, location: &str) -> PickedFile {
        PickedFile { name: name.into(), bytes: vec![1], location: Location::new(location) }
    }

    /// The path ends in the file's name (the app names, typed and formats the document by it) and
    /// is the same in every session for the same file, different for another.
    #[test]
    fn a_document_path_names_the_file_and_stays_put() {
        let file = located("a.psd", "content://downloads/document/33");
        let path = file.document_path().unwrap_or_default();
        assert!(path.starts_with(IN_PLACE_ROOT) && path.ends_with("/a.psd"), "{path}");
        assert_eq!(Path::new(&path).file_name().and_then(|n| n.to_str()), Some("a.psd"));
        assert_eq!(Path::new(&path).extension().and_then(|n| n.to_str()), Some("psd"));
        assert_eq!(file.document_path(), located("a.psd", "content://downloads/document/33").document_path());
        assert_ne!(file.document_path(), located("a.psd", "content://downloads/document/34").document_path());
        assert!(located("a/b.png", "file:///x").document_path().is_some_and(|p| p.ends_with("/a_b.png")));
        assert_eq!(PickedFile { location: None, ..file }.document_path(), None, "a copy has no path to write back to");
        assert_eq!(Location::new(""), None);
    }

    /// Only a document path handed out this session writes in place; the same folder with
    /// another extension (Save in another format) or any other path goes to the exporter.
    #[test]
    fn saves_write_back_only_to_files_the_shell_handed_over() {
        let platform = Platform::default();
        let file = located("a.png", "file:///docs/a.png");
        let path = file.document_path().unwrap_or_default();
        assert_eq!(platform.save(&path, vec![9]), SaveRoute::Exporter, "not handed over yet");
        platform.remember(&file);
        assert!(platform.writes_in_place(&path));
        assert_eq!(platform.save(&path, vec![7]), SaveRoute::InPlace);
        let as_psd = Path::new(&path).with_extension("psd").to_string_lossy().into_owned();
        assert_eq!(platform.save(&as_psd, vec![8]), SaveRoute::Exporter);
        assert_eq!(
            platform.take_requests(),
            vec![
                Request::Export { name: "a.png".into(), bytes: vec![9] },
                Request::WriteInPlace { location: Location("file:///docs/a.png".into()), name: "a.png".into(), bytes: vec![7] },
                Request::Export { name: "a.psd".into(), bytes: vec![8] },
            ]
        );
    }

    /// Answers come back by location, oldest write first, each once.
    #[test]
    fn each_write_in_place_is_answered_once_in_order() {
        let platform = Platform::default();
        let (a, b) = (located("a.png", "loc-a"), located("b.png", "loc-b"));
        platform.answer_pick(vec![a.clone(), b.clone()]);
        let (pa, pb) = (a.document_path().unwrap_or_default(), b.document_path().unwrap_or_default());
        for (path, byte) in [(&pa, 1), (&pb, 2), (&pa, 3)] {
            assert_eq!(platform.save(path, vec![byte]), SaveRoute::InPlace);
        }
        let loc = |s: &str| Location(s.into());
        assert_eq!(platform.finish_write(&loc("loc-b")), Some(pb));
        assert_eq!(platform.finish_write(&loc("loc-a")), Some(pa.clone()));
        assert_eq!(platform.finish_write(&loc("loc-a")), Some(pa));
        assert_eq!(platform.finish_write(&loc("loc-a")), None, "nothing more was written there");
    }

    #[test]
    fn a_write_in_place_reaches_the_shell_with_its_name() {
        let request = Request::WriteInPlace { location: Location("content://x/1".into()), name: "a.psd".into(), bytes: vec![5] };
        assert_eq!(request.wire(), (8, "content://x/1\na.psd".into(), Some(&[5_u8][..])));
    }

    #[test]
    fn document_paths_are_shown_as_file_names() {
        let path = located("a b.psd", "file:///x").document_path().unwrap_or_default();
        assert_eq!(with_file_names(&format!("Saved {path}")), "Saved a b.psd");
        assert_eq!(with_file_names(&format!("{path} and {path}")), "a b.psd and a b.psd");
        assert!(matches!(with_file_names("Saved /tmp/a.psd"), Cow::Borrowed("Saved /tmp/a.psd")), "other paths stay");
        assert_eq!(with_file_names("see inplace://notanid/x"), "see inplace://notanid/x", "not one of ours");
        assert_eq!(with_file_names("inplace://"), "inplace://");
    }

    #[test]
    fn a_write_in_place_keeps_the_app_busy_until_answered() {
        let platform = Platform::default();
        let file = located("a.png", "loc-a");
        platform.remember(&file);
        assert!(!platform.writing());
        let _ = platform.save(&file.document_path().unwrap_or_default(), vec![1]);
        assert!(platform.writing());
        platform.finish_write(&Location("loc-a".into()));
        assert!(!platform.writing());
    }

    #[test]
    fn busy_reaches_the_shell_as_kind_nine() {
        assert_eq!(Request::Busy(true).wire(), (9, "1".into(), None));
        assert_eq!(Request::Busy(false).wire(), (9, "0".into(), None));
    }

    #[test]
    fn paste_share_and_photos_reach_the_shell_as_kinds_ten_to_twelve() {
        let platform = Platform::default();
        platform.request_paste();
        platform.share("a.psd".into(), vec![3]);
        platform.pick_photos();
        let wired: Vec<(i32, String, Option<Vec<u8>>)> =
            platform.take_requests().iter().map(|r| r.wire()).map(|(k, t, d)| (k, t.into_owned(), d.map(<[u8]>::to_vec))).collect();
        assert_eq!(wired, [(10, String::new(), None), (11, "a.psd".into(), Some(vec![3])), (12, String::new(), None)]);
    }

    /// What the shell read is the app's for the frame that handles the paste, and only then.
    #[test]
    fn a_paste_is_kept_until_cleared() {
        let platform = Platform::default();
        assert_eq!(platform.pasted(), None);
        platform.set_pasted(Some(Pasted::OwnImage));
        assert_eq!(platform.pasted().as_deref(), Some(&Pasted::OwnImage));
        platform.set_pasted(None);
        assert_eq!(platform.pasted(), None);
    }

    fn recorder() -> (Rc<RefCell<Option<Vec<PickedFile>>>>, PickReply) {
        let got = Rc::new(RefCell::new(None));
        let sink = got.clone();
        (got, Box::new(move |files| *sink.borrow_mut() = Some(files)))
    }

    #[test]
    fn a_pick_is_requested_once_and_answered_once() {
        let platform = Platform::default();
        let (got, reply) = recorder();
        platform.pick_files(true, vec!["png".into()], reply);
        assert_eq!(platform.take_requests(), vec![Request::PickFiles { multiple: true, extensions: vec!["png".into()] }]);
        assert!(platform.take_requests().is_empty(), "drained");
        let file = PickedFile { name: "a.png".into(), bytes: vec![1, 2], location: None };
        platform.answer_pick(vec![file.clone()]);
        assert_eq!(*got.borrow(), Some(vec![file]));
        platform.answer_pick(vec![]);
        assert_eq!(got.borrow().as_ref().map(Vec::len), Some(1), "a second answer goes nowhere");
    }

    /// The app must never wait forever on a picker the user can no longer see.
    #[test]
    fn a_new_pick_cancels_the_one_still_open() {
        let platform = Platform::default();
        let (first, reply) = recorder();
        platform.pick_files(false, vec![], reply);
        let (_second, reply) = recorder();
        platform.pick_files(false, vec![], reply);
        assert_eq!(*first.borrow(), Some(vec![]), "the first reply heard 'cancelled'");
    }
}
