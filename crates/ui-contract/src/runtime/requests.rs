//! What a control asks of its host and what the host answers: the colour picker, the file dialog and the eyedropper.
//! A request is a plain value with a `key` naming what it is for; a component reports it for the one pass in which the
//! user asked, and the host answers with a [`HostAnswer`]. `patterns::form_dialogs::HostRequests` keeps them until the
//! host takes them.

use crate::runtime::env::Label;
use craft_theme::Srgba;
use std::path::PathBuf;

/// The image file extensions a file dialog offers for pictures, without the dot, in lower case.
pub const IMAGE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "tif", "tiff"];

/// A request for a colour: the host opens its colour picker with `title` and `color` and writes the answer into
/// `key` as a `FieldValue::Color`.
///
/// Rows report it as `pick_color: Option<ColorRequest>`, set for the one pass in which the user asked. `key` names
/// what is being coloured so one form can ask for several colours.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ColorRequest {
    /// What the colour is for, returned with the answer.
    pub key: String,
    /// The colour picker's title; `None` for the picker's default.
    pub title: Option<Label>,
    /// The colour the picker opens with; `None` opens it with no current colour (a mixed swatch): no current swatch
    /// and empty fields, and the first pick sets the colour.
    pub color: Option<Srgba>,
}

impl ColorRequest {
    /// A request for `key`, opening the picker at `color`, or with no colour for `None`.
    pub fn new(key: impl Into<String>, color: impl Into<Option<Srgba>>) -> Self {
        Self { key: key.into(), title: None, color: color.into() }
    }

    /// The picker's title.
    #[must_use]
    pub fn title(mut self, title: impl Into<Label>) -> Self {
        self.title = Some(title.into());
        self
    }
}

/// What a text row's Browse button picks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PathKind {
    /// Picks a folder.
    #[default]
    Folder,
    /// An existing file. `extensions` are the ones the file dialog offers, without the dot and in any letter case
    /// (`["cube", "3dl"]`); empty offers every file.
    #[non_exhaustive]
    OpenFile {
        /// The extensions offered.
        extensions: Vec<String>,
    },
    /// Chooses a file to write.
    SaveFile,
}

impl PathKind {
    /// An existing file of one of `extensions`.
    pub fn open_file(extensions: &[&str]) -> Self {
        Self::OpenFile { extensions: extensions.iter().map(|e| (*e).to_owned()).collect() }
    }
}

/// One entry of a file dialog's type filter.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PathFilter {
    /// The filter's name ("PNG image").
    pub name: String,
    /// The extensions it admits, without the dot.
    pub extensions: Vec<String>,
}

impl PathFilter {
    /// A filter named `name` for `extensions`.
    pub fn new(name: impl Into<String>, extensions: &[&str]) -> Self {
        Self { name: name.into(), extensions: extensions.iter().map(|e| (*e).to_owned()).collect() }
    }
}

/// A pressed Browse button: the host opens its file dialog for `kind` and writes the answer into `key` as a
/// `FieldValue::Text`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PathRequest {
    /// The key of the text parameter the path belongs to.
    pub key: String,
    /// What the picker should offer.
    pub kind: PathKind,
    /// The file name a save dialog suggests; `None` for the dialog's default.
    pub file_name: Option<String>,
    /// The types a file dialog offers; empty offers every file.
    pub filters: Vec<PathFilter>,
}

impl PathRequest {
    /// A request for the path of `key`.
    pub fn new(key: impl Into<String>, kind: PathKind) -> Self {
        Self { key: key.into(), kind, file_name: None, filters: Vec::new() }
    }

    /// A request for a file to write under `key`, suggesting `file_name` and offering `filters`.
    pub fn save(key: impl Into<String>, file_name: impl Into<String>, filters: impl IntoIterator<Item = PathFilter>) -> Self {
        Self { key: key.into(), kind: PathKind::SaveFile, file_name: Some(file_name.into()), filters: filters.into_iter().collect() }
    }
}

/// An eyedropper toggle of a colour row after a press: the host answers it with the colour the sampler picked for the
/// colour parameter `key` (`HostRequests::answer_eyedropper`), written into `key` as a `FieldValue::Color`. The rows
/// hold which dropper is armed (one at a time); `Param::armed` arms and disarms a row from the host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct EyedropperRequest {
    /// The key of the colour parameter the dropper fills.
    pub key: String,
}

impl EyedropperRequest {
    /// A request for the dropper of `key`.
    pub fn new(key: impl Into<String>) -> Self {
        Self { key: key.into() }
    }
}

/// What the host answered to a request, kept until the dialog applies it to its values in its next pass.
#[derive(Clone, Debug, PartialEq)]
pub enum HostAnswer {
    /// The colour the picker returned for the colour request `key`.
    Color {
        /// The request's key.
        key: String,
        /// The picked colour.
        color: Srgba,
    },
    /// The path the file dialog returned for the path request `key`.
    Path {
        /// The request's key.
        key: String,
        /// The chosen path.
        path: PathBuf,
    },
    /// The colour the eyedropper sampled for the eyedropper request `key`.
    Eyedropper {
        /// The request's key.
        key: String,
        /// The sampled colour.
        color: Srgba,
    },
}
