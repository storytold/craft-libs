//! The documents open when the app last quit, to reopen at the next launch (a "Reopen Last
//! Session" preference). Egui-free: the app supplies what is open and opens the files itself.

use serde::{Deserialize, Serialize};

/// More documents than anyone keeps open. Settings are untrusted, so a longer list is cut here.
// pdfcraft: crates/ui-egui/src/persist/last_session.rs:10 `MAX_FILES` (64).
pub const MAX_DOCUMENTS: usize = 64;

/// One open document and where the user was in it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionDocument {
    /// Absolute ([`absolute`]), so a file opened from the command line by a relative path is
    /// found again.
    pub path: String,
    /// The window it was in, by its craft-window key; `None` for the main window.
    pub window: Option<String>,
    /// The app's view state for the document (PdfCraft's page, PhotoCraft's zoom and scroll), as
    /// the app wrote it; `Null` when it keeps none.
    pub view: serde_json::Value,
}

/// The documents open when the app last quit, stored in the app's settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    /// In tab order. Documents never saved to a file are left out: crash recovery keeps their
    /// unsaved work.
    pub documents: Vec<SessionDocument>,
    /// The path of the document in front.
    pub active: Option<String>,
}

impl Session {
    /// Reads a stored session leniently: entries with no path are skipped, a missing `view` is
    /// `Null`, and the list is cut at [`MAX_DOCUMENTS`].
    // pdfcraft: crates/ui-egui/src/persist/last_session.rs:32 `from_json`.
    pub fn from_json(value: &serde_json::Value) -> Session {
        let documents = value["documents"]
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| {
                        let path = entry["path"].as_str().filter(|p| !p.is_empty())?.to_string();
                        let window = entry["window"].as_str().map(str::to_string);
                        let view = entry.get("view").cloned().unwrap_or_default();
                        Some(SessionDocument { path, window, view })
                    })
                    .take(MAX_DOCUMENTS)
                    .collect()
            })
            .unwrap_or_default();
        Session { documents, active: value["active"].as_str().map(str::to_string) }
    }

    /// The documents to reopen at launch, in order: those whose file still exists, that aren't in
    /// `skip` (files the app opens anyway, from the command line, or that crash recovery offers)
    /// and that aren't open already (`is_open`). Paths are compared after [`absolute`].
    // pdfcraft: crates/ui-egui/src/persist/last_session.rs:102 `reopen_last_files`.
    pub fn to_reopen<'s>(&'s self, skip: &[String], is_open: impl Fn(&str) -> bool) -> Vec<&'s SessionDocument> {
        let skip: Vec<String> = skip.iter().map(|p| absolute(p)).collect();
        self.documents
            .iter()
            .filter(|doc| {
                let path = absolute(&doc.path);
                !skip.contains(&path) && std::path::Path::new(&path).is_file() && !is_open(&path)
            })
            .collect()
    }
}

/// What to save as the session across a quit. A quit closes tabs one by one (asking about unsaved
/// changes), so what is open at the end isn't what was open when the user quit: the keeper
/// remembers the session when the quit starts and forgets it when the quit is cancelled.
#[derive(Clone, Debug, Default)]
pub struct SessionKeeper {
    quit: Option<Session>,
}

impl SessionKeeper {
    /// A quit is about to close its first document: remember `open` (once per quit).
    // pdfcraft: crates/ui-egui/src/persist/last_session.rs:84 `note_quit_session`.
    pub fn quit_started(&mut self, open: impl FnOnce() -> Session) {
        if self.quit.is_none() {
            self.quit = Some(open());
        }
    }

    /// The quit was cancelled: what is open from now on is the session again.
    // pdfcraft: crates/ui-egui/src/persist/last_session.rs:91 `forget_quit_session`.
    pub fn quit_cancelled(&mut self) {
        self.quit = None;
    }

    /// What to store: the session remembered at the quit's start once the app is really quitting
    /// (`quitting`), else `open`.
    // pdfcraft: crates/ui-egui/src/persist/last_session.rs:76 `session_to_save`.
    pub fn to_save(&self, quitting: bool, open: Session) -> Session {
        match &self.quit {
            Some(session) if quitting => session.clone(),
            _ => open,
        }
    }
}

/// `path` made absolute against the current directory, without resolving links (and without
/// Windows' `\\?\` prefix, which `canonicalize` adds). Unchanged on the web.
// pdfcraft: crates/ui-egui/src/persist/last_session.rs:55 `absolute`.
pub fn absolute(path: &str) -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::path::absolute(path).map_or_else(|_| path.to_string(), |p| p.to_string_lossy().into_owned())
    }
    #[cfg(target_arch = "wasm32")]
    {
        path.to_string()
    }
}
