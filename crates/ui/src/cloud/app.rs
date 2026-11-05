//! What this module needs from the application around it.
//!
//! Two operations here belong to the application alone. One turns bytes into an
//! open document. The other turns an open document into bytes. Both need the
//! application's own mutable state. That state is what holds the
//! [`CloudFiles`](crate::cloud::CloudFiles) this module keeps its own state in.
//! The pages are therefore free functions over this trait. An application
//! reaches its cloud state back through [`CloudApp::cloud_mut`].
//!
//! Every method here is something only the application can answer. The ones
//! with a body are the ones a plain application can leave alone.

use egui::{Color32, FontFamily, Painter, Rect, Ui};

use craft_cloud::EntryKind;

/// What one of the application's documents is, as a file list needs to know.
///
/// Four pieces of static data, usually one `const` in the application. These
/// four are the whole of what an application has to say about its own
/// documents. They are data because every one of them as a trait method would
/// have been a pure function of nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentKind {
    /// Extensions this application opens, lower case and without a dot. The
    /// same set its desktop file picker offers.
    pub extensions: &'static [&'static str],
    /// Content types it opens, matched against the start of what the provider
    /// said. The match is a prefix. A provider commonly appends a profile to a
    /// type it is otherwise right about.
    pub mimes: &'static [&'static str],
    /// What one is called in a sentence a user reads. "workbook", "document",
    /// "drawing".
    pub noun: &'static str,
    /// The extension a typed name without a known one gets.
    pub default_extension: &'static str,
}

/// The colours and faces these pages draw in.
///
/// The Open and Save pages sit inside the application's own Backstage view and
/// have to match the surface around them. `ui.visuals()` alone cannot say what
/// that surface is. An application with a palette of its own passes it in. One
/// without takes [`CloudStyle::from_visuals`] and writes nothing.
#[derive(Clone, Debug)]
pub struct CloudStyle {
    /// Behind a bordered column.
    pub surface: Color32,
    pub border: Color32,
    pub text: Color32,
    /// Second lines, hints and the status line.
    pub dim: Color32,
    /// Behind a file row the pointer is over.
    pub hover: Color32,
    /// Behind the panel that offers to link an account. Tinted and never
    /// alarming. No account yet is a state with an obvious next step.
    pub soft: Color32,
    pub family: FontFamily,
    pub bold: FontFamily,
}

impl CloudStyle {
    /// Colours from egui's own visuals, for an application with no palette.
    pub fn from_visuals(visuals: &egui::Visuals) -> CloudStyle {
        CloudStyle {
            surface: visuals.faint_bg_color,
            border: visuals.widgets.noninteractive.bg_stroke.color,
            text: visuals.text_color(),
            dim: visuals.weak_text_color(),
            hover: visuals.widgets.hovered.bg_fill,
            soft: visuals.faint_bg_color,
            family: FontFamily::Proportional,
            bold: FontFamily::Proportional,
        }
    }
}

/// One of the application's own Save a Copy formats, for the second half of
/// the Share menu.
///
/// The application's own. What a spreadsheet exports and what a page layout
/// application exports have nothing in common.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyFormat {
    /// Handed back untouched when this one is chosen.
    pub id: String,
    pub label: String,
}

impl CopyFormat {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> CopyFormat {
        CopyFormat { id: id.into(), label: label.into() }
    }
}

/// An application that opens and saves documents in cloud storage.
/// Where a save is going.
///
/// Carried as a pair because neither half is reliable alone. A provider hands
/// back `application/octet-stream` for a perfectly good document, and a file
/// exported by a web application often has a correct content type and no
/// extension at all. The same reasoning as [`DocumentKind`], which reads both
/// when deciding whether a document opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Destination<'a> {
    /// The file name the bytes are going to.
    pub name: &'a str,
    /// What the provider says is already there, where it says anything. Absent
    /// for a document that does not exist yet.
    pub mime: Option<&'a str>,
}

impl<'a> Destination<'a> {
    pub fn new(name: &'a str) -> Destination<'a> {
        Destination { name, mime: None }
    }

    /// The extension the name carries, lower cased, when it carries one.
    ///
    /// Empty for a bare name. A caller that gets nothing here has the content
    /// type left to read.
    pub fn extension(&self) -> String {
        self.name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).filter(|ext| !ext.is_empty()).unwrap_or_default()
    }
}

pub trait CloudApp {
    // ---- the state these pages keep -------------------------------------

    fn cloud(&self) -> &crate::cloud::CloudFiles;
    fn cloud_mut(&mut self) -> &mut crate::cloud::CloudFiles;

    // ---- its documents ---------------------------------------------------

    fn kind(&self) -> DocumentKind;

    /// The document in this window, when there is one.
    fn active(&self) -> Option<u64>;

    /// What to call one, for the Save As name field.
    fn title(&self, uid: u64) -> String;

    /// Turn bytes into an open document, answering with the one it opened as.
    ///
    /// `Ok(None)` when the application took the bytes somewhere else, such as
    /// a prompt for a password. Nothing is said about them yet. `Err` is put
    /// to the user.
    fn open_bytes(&mut self, name: &str, bytes: Vec<u8>) -> Result<Option<u64>, String>;

    /// Turn one open document into the bytes to save, for `into`.
    ///
    /// [`Destination`] is where the bytes are going. A document opened from
    /// the cloud as `figures.csv` has to go back as a CSV. Encoding it in the
    /// application's own format under that name hands the user a file no other
    /// application can read, and nothing tells them it happened.
    ///
    /// The name is the first answer. A provider that issued no extension at
    /// all still says what the thing is, so the content type is the second.
    /// Both appear often: a file exported by a web application commonly has a
    /// correct content type and a bare name.
    ///
    /// An application that cannot write what the destination asks for should
    /// refuse and say which formats it can write. A refusal a person can act
    /// on beats a file that silently is not what it claims.
    ///
    /// Takes `&mut self` so the application can record which version of the
    /// document these bytes were. A save that lands clears the dirty flag for
    /// exactly that version. An edit made while the bytes were on the wire
    /// stays unsaved. [`CloudApp::mark_saved`] and [`CloudApp::drop_pending`]
    /// say which happened.
    fn encode(&mut self, uid: u64, into: Destination<'_>) -> Result<Vec<u8>, String>;

    /// Close a document a reload replaced. Called only once the replacement is
    /// open. A read that failed leaves the user's work where it was.
    fn close(&mut self, uid: u64);

    /// The bytes [`CloudApp::encode`] made reached the service.
    fn mark_saved(&mut self, _uid: u64) {}

    /// They did not. Forget the version it recorded.
    fn drop_pending(&mut self, _uid: u64) {}

    /// Finish whatever the user is still typing. False stops the save.
    fn ready_to_save(&mut self) -> bool {
        true
    }

    // ---- the window around them ------------------------------------------

    /// The share dialog this window holds. Opened from a document's overflow
    /// menu and from the Share button. Refreshed when who is signed in
    /// changes.
    fn sharing(&mut self) -> &mut crate::share::ShareDialog;

    /// Who else is in the document. Drawn beside the account badge.
    fn presence(&mut self) -> &mut crate::presence::PresenceBar;

    /// Paint the icon for one row of a file list. Every application already
    /// owns an icon set. None of them agree on how it is reached.
    fn paint_icon(&self, painter: &Painter, rect: Rect, kind: EntryKind, colour: Color32);

    /// Open a URL in the user's browser. A browser is the application's
    /// business. This crate opens nothing.
    fn open_url(&self, url: &str);

    /// Open the provider's conversation for this document.
    ///
    /// Desktop hosts may hand this to the operating system so a registered
    /// chat application can claim it. Browser hosts may navigate the current
    /// tab, which remains reliable after an asynchronous provider lookup.
    fn open_conversation(&self, url: &str) {
        self.open_url(url);
    }

    /// Say something that needs the user's attention, in the application's own
    /// message box.
    fn say(&mut self, text: String);

    /// Say something that does not, in passing.
    fn toast(&mut self, _text: String) {}

    /// Take the user to the application's own Save As page, with the name
    /// [`CloudFiles::save_name`](crate::cloud::CloudFiles::save_name) now
    /// holding. Called when a refused save is answered with Save as a copy.
    fn go_to_save_as(&mut self) {}

    // ---- its words -------------------------------------------------------

    /// What the application is called, for the one sentence that names it.
    fn app_name(&self) -> &str;

    /// Translate one English string. The default leaves it alone. An
    /// application with no translation table writes nothing.
    fn tr(&self, text: &str) -> String {
        text.to_owned()
    }

    /// The host part of the service address, for the account menu's first
    /// line. Somebody with an account on two deployments needs to know which
    /// one a document is about to land in.
    fn service(&self) -> String {
        String::new()
    }

    /// A line under the Save page's name field, about the format this
    /// application saves in.
    fn save_note(&self) -> Option<String> {
        None
    }

    /// The colours and faces to draw in.
    fn style(&self, ui: &Ui) -> CloudStyle {
        CloudStyle::from_visuals(ui.visuals())
    }
}
