//! Preset files: Load…, Replace… and Save… as data the host drives. The pattern says which file to open or save
//! ([`PresetFileRequest`]); the host runs its file dialog and answers with the result, which comes back as a
//! [`PresetFileEvent`].

use super::panel_menu::PresetCommands;
use super::words::PresetWords;

/// What a Save… writes: the app's bytes and the suggested file name.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PresetExport {
    /// The suggested file name with its extension: "Contours.shc".
    pub file_name: String,
    /// The file's contents in the app's format; the library has no preset file format.
    pub bytes: Vec<u8>,
}

impl PresetExport {
    /// An export of `bytes` suggested as `file_name`.
    pub fn new(file_name: impl Into<String>, bytes: Vec<u8>) -> Self {
        PresetExport { file_name: file_name.into(), bytes }
    }
}

/// Why a read or write failed, worded through [`PresetWords`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PresetFileFailure {
    /// The file was not found.
    NotFound,
    /// Access was denied.
    Denied,
    /// File access is not available here.
    Unsupported,
    /// Any other error of the file system.
    Other,
}

/// The file dialog a command asks the host to run.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PresetFileRequest {
    /// Pick a file to read: Load… (`replace` false) or Replace… (`replace` true).
    #[non_exhaustive]
    Open {
        /// The pick's purpose, which also keeps its last folder.
        purpose: String,
        /// The name of the file filter.
        filter: String,
        /// The extensions the filter admits, without dots.
        extensions: Vec<String>,
        /// Replace… rather than Load….
        replace: bool,
    },
    /// Pick a file to write the export to.
    #[non_exhaustive]
    Save {
        /// The pick's purpose, which also keeps its last folder.
        purpose: String,
        /// The name of the file filter.
        filter: String,
        /// The extensions the filter admits, without dots.
        extensions: Vec<String>,
        /// The suggested file name.
        file_name: String,
        /// What to write.
        bytes: Vec<u8>,
    },
}

impl PresetFileRequest {
    /// The event for a file the host read after an `Open`; `None` for a `Save`.
    pub fn loaded(&self, file_name: impl Into<String>, bytes: Vec<u8>) -> Option<PresetFileEvent> {
        match self {
            PresetFileRequest::Open { replace, .. } => Some(PresetFileEvent::Loaded { file_name: file_name.into(), bytes, replace: *replace }),
            _ => None,
        }
    }

    /// The event for the file the host wrote after a `Save`; `None` for an `Open`.
    pub fn saved(&self, file_name: impl Into<String>) -> Option<PresetFileEvent> {
        match self {
            PresetFileRequest::Save { .. } => Some(PresetFileEvent::Saved { file_name: file_name.into() }),
            _ => None,
        }
    }

    /// The event for a read or write that failed.
    pub fn failed(&self, file_name: impl Into<String>, failure: PresetFileFailure) -> PresetFileEvent {
        PresetFileEvent::Failed { file_name: file_name.into(), failure, writing: matches!(self, PresetFileRequest::Save { .. }) }
    }
}

/// What a finished file pick did.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PresetFileEvent {
    /// A file was read for Load… or Replace…; the app parses `bytes` in its own format and makes one undo step.
    #[non_exhaustive]
    Loaded {
        /// The picked file's name.
        file_name: String,
        /// The file's contents.
        bytes: Vec<u8>,
        /// True for Replace…, false for Load….
        replace: bool,
    },
    /// Save… wrote the export. Nothing in the document changes.
    #[non_exhaustive]
    Saved {
        /// The name of the file written.
        file_name: String,
    },
    /// A read or write failed.
    #[non_exhaustive]
    Failed {
        /// The name of the file.
        file_name: String,
        /// Why.
        failure: PresetFileFailure,
        /// Whether the failure was a write (Save…) rather than a read.
        writing: bool,
    },
}

impl PresetFileEvent {
    /// The line for the app's status bar: "Loaded Contours.shc", or the failure with its reason.
    pub fn status(&self, words: &PresetWords) -> String {
        match self {
            PresetFileEvent::Loaded { file_name, .. } => words.loaded.fill(&[("file", file_name)]),
            PresetFileEvent::Saved { file_name } => words.saved.fill(&[("file", file_name)]),
            PresetFileEvent::Failed { file_name, failure, writing } => {
                let reason = match failure {
                    PresetFileFailure::NotFound => &words.reason_not_found,
                    PresetFileFailure::Denied => &words.reason_denied,
                    PresetFileFailure::Unsupported => &words.reason_unsupported,
                    PresetFileFailure::Other => &words.reason_other,
                };
                let template = if *writing { &words.write_failed } else { &words.read_failed };
                template.fill(&[("file", file_name), ("reason", reason.as_str())])
            }
        }
    }
}

/// The app's Save… bytes for the selected key.
type Export<'a, K> = Box<dyn Fn(Option<&K>) -> Option<PresetExport> + 'a>;

/// Whether Save… is enabled for the selected key.
type CanExport<'a, K> = Box<dyn Fn(Option<&K>) -> bool + 'a>;

/// The file entries of a preset kind: which of Load…, Replace… and Save… the panel menu offers, and the dialog each
/// one asks the host for. The host shows the dialog, reads or writes the file and reports the result with
/// [`PresetFileRequest::loaded`], [`saved`](PresetFileRequest::saved) or [`failed`](PresetFileRequest::failed).
pub struct PresetFiles<'a, K> {
    purpose: String,
    name: String,
    extensions: Vec<String>,
    replace: bool,
    export: Option<Export<'a, K>>,
    can_export: Option<CanExport<'a, K>>,
}

impl<'a, K> PresetFiles<'a, K> {
    /// File entries for `purpose` (the pick's purpose, which also keeps its last folder) with a filter named `name`
    /// over `extensions` (without dots). Load… only, until [`replace`](Self::replace) or [`export`](Self::export)
    /// add the others.
    pub fn new(purpose: impl Into<String>, name: impl Into<String>, extensions: &[&str]) -> Self {
        PresetFiles {
            purpose: purpose.into(),
            name: name.into(),
            extensions: extensions.iter().map(|e| (*e).to_owned()).collect(),
            replace: false,
            export: None,
            can_export: None,
        }
    }

    /// Offers Replace… beside Load….
    #[must_use]
    pub fn replace(mut self, replace: bool) -> Self {
        self.replace = replace;
        self
    }

    /// Offers Save…: `export` makes the bytes for the selected key when Save… is chosen (`None` when nothing is
    /// selected or the bytes cannot be made: no dialog is asked for).
    #[must_use]
    pub fn export(mut self, export: impl Fn(Option<&K>) -> Option<PresetExport> + 'a) -> Self {
        self.export = Some(Box::new(export));
        self
    }

    /// Whether Save… is enabled for the selected key (default: always; the export may save the current settings with
    /// nothing selected). A cheap test the menu reads every pass; `export` runs only when Save… is chosen.
    #[must_use]
    pub fn can_export(mut self, can_export: impl Fn(Option<&K>) -> bool + 'a) -> Self {
        self.can_export = Some(Box::new(can_export));
        self
    }

    /// The commands the panel menu lists, in order.
    pub fn commands(&self) -> Vec<PresetCommands> {
        let mut commands = vec![PresetCommands::Load];
        if self.replace {
            commands.push(PresetCommands::Replace);
        }
        if self.export.is_some() {
            commands.push(PresetCommands::Save);
        }
        commands
    }

    /// Whether `command` is enabled for the selected key.
    pub fn enabled(&self, command: PresetCommands, selected: Option<&K>) -> bool {
        match command {
            PresetCommands::Save => self.can_export.as_ref().is_none_or(|can| can(selected)),
            _ => true,
        }
    }

    /// The dialog `command` asks the host for; `None` for another command, for Replace… without
    /// [`replace`](Self::replace), and for Save… when the export has nothing to write.
    pub fn request(&self, command: PresetCommands, selected: Option<&K>) -> Option<PresetFileRequest> {
        let (purpose, filter, extensions) = (self.purpose.clone(), self.name.clone(), self.extensions.clone());
        match command {
            PresetCommands::Load => Some(PresetFileRequest::Open { purpose, filter, extensions, replace: false }),
            PresetCommands::Replace if self.replace => Some(PresetFileRequest::Open { purpose, filter, extensions, replace: true }),
            PresetCommands::Save => {
                let export = self.export.as_ref().and_then(|export| export(selected))?;
                Some(PresetFileRequest::Save { purpose, filter, extensions, file_name: export.file_name, bytes: export.bytes })
            }
            _ => None,
        }
    }
}
