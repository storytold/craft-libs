//! `format` contributions: open and save files of a type the app does not know.
//!
//! An app owns one [`FormatBook`] and gives it to `PluginManager::attach_surfaces` (or forwards the
//! surfaces of its own sink to it). When the user opens a file whose extension no built-in reader
//! takes, the app asks [`FormatBook::handles`]. If that is true, it calls [`FormatBook::read`] with
//! the file name and the file bytes and gets an [`ImageDoc`]. [`FormatBook::write`] goes the other
//! way. The wire format is `craft_plugin_abi::format`.
//!
//! Rules this module keeps:
//!
//! - A `format` needs `fs.picked` (default deny). Registration leaves it out without the grant, and
//!   every call checks the grant again, so a revoke between the lookup and the call still denies.
//! - The plugin gets bytes, never a path. It cannot choose which file to open.
//! - Every call is one guarded call: the app's undo scope opens and closes around it, a trap or a
//!   timeout counts toward the circuit, and the user is told once.
//! - A call has a wall deadline ([`FormatBook::with_deadline`], 30 s by default), because a format
//!   runs on the `batch` budget class, which has none of its own.
//! - Two plugins that claim one extension resolve the same way every time: the lowest plugin id,
//!   then the lowest contribution id.
//! - A response is JSON, so bytes in it are base64. The 16 MiB response cap limits a decoded image
//!   to about 12 MiB. A larger one is [`FormatError::Call`] with `BadResponse`.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use craft_command::Owner;
use craft_plugin_abi::Permission;
use craft_plugin_abi::format::{self as wire, FormatDocError, ImageDoc};
use craft_plugin_abi::manifest::Kind;
use serde_json::Value;

use crate::contrib::surface::{Surface, SurfaceSink};
use crate::invoke::{CallError, CallOpts};

/// Wall deadline of one call when the app sets none.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(30);

/// Why a read or a write did not produce a result.
#[derive(Clone, Debug, PartialEq)]
pub enum FormatError {
    /// The file name has no extension.
    NoExtension(String),
    /// No live plugin handles this extension (lower case, no dot).
    NoPlugin { ext: String },
    /// The plugin that handles the extension lacks a permission now.
    Denied { plugin: String, permission: Permission },
    /// The call failed: the plugin refused the file (`CallError::Plugin`), trapped, ran out of time,
    /// or answered something that is not JSON.
    Call(CallError),
    /// The call worked but the answer does not fit the protocol.
    Doc(FormatDocError),
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::NoExtension(name) => write!(f, "{name} has no file extension"),
            FormatError::NoPlugin { ext } => write!(f, "no plugin handles .{ext} files"),
            FormatError::Denied { plugin, permission } => write!(f, "{plugin} is not allowed to use {permission}"),
            FormatError::Call(e) => write!(f, "{e}"),
            FormatError::Doc(e) => write!(f, "the plugin answered wrongly: {e}"),
        }
    }
}

impl std::error::Error for FormatError {}

impl From<CallError> for FormatError {
    fn from(e: CallError) -> Self {
        FormatError::Call(e)
    }
}

impl From<FormatDocError> for FormatError {
    fn from(e: FormatDocError) -> Self {
        FormatError::Doc(e)
    }
}

/// The live `format` surfaces, and the runner that calls them.
pub struct FormatBook {
    items: Mutex<Vec<Surface>>,
    deadline: Duration,
}

impl Default for FormatBook {
    fn default() -> Self {
        FormatBook { items: Mutex::new(Vec::new()), deadline: DEFAULT_DEADLINE }
    }
}

impl FormatBook {
    /// A book with the [`DEFAULT_DEADLINE`].
    pub fn new() -> Arc<FormatBook> {
        Arc::new(FormatBook::default())
    }

    pub fn with_deadline(deadline: Duration) -> Arc<FormatBook> {
        Arc::new(FormatBook { items: Mutex::new(Vec::new()), deadline })
    }

    fn items(&self) -> MutexGuard<'_, Vec<Surface>> {
        self.items.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Live surfaces that list `ext`, in the order the book resolves them.
    fn claims(&self, ext: &str) -> Vec<Surface> {
        let mut found: Vec<Surface> = self
            .items()
            .iter()
            .filter(|s| s.contribution.extensions.as_deref().unwrap_or_default().iter().any(|e| e.eq_ignore_ascii_case(ext)))
            .cloned()
            .collect();
        found.sort_by(|a, b| (&*a.owner.plugin_id, &a.contribution.id).cmp(&(&*b.owner.plugin_id, &b.contribution.id)));
        found
    }

    /// True when a live plugin handles `ext` (any case, no dot).
    pub fn handles(&self, ext: &str) -> bool {
        !self.claims(ext).is_empty()
    }

    /// Every extension a live plugin handles, sorted, for a file dialog filter.
    pub fn extensions(&self) -> Vec<String> {
        let mut all: Vec<String> = self.items().iter().flat_map(|s| s.contribution.extensions.clone().unwrap_or_default()).collect();
        all.sort();
        all.dedup();
        all
    }

    /// Ids of the plugins that handle `ext`, sorted. The first one answers.
    pub fn owners(&self, ext: &str) -> Vec<String> {
        self.claims(ext).iter().map(|s| s.owner.plugin_id.to_string()).collect()
    }

    fn call(&self, ext: &str, request: &Value, blob: &[u8]) -> Result<Value, FormatError> {
        let surface = self.claims(ext).into_iter().next().ok_or_else(|| FormatError::NoPlugin { ext: ext.to_owned() })?;
        let plugin = surface.owner.plugin_id.to_string();
        if !surface.grants()?.contains(&Permission::FsPicked) {
            return Err(FormatError::Denied { plugin, permission: Permission::FsPicked });
        }
        let opts = CallOpts { deadline: Some(self.deadline), ..CallOpts::default() };
        Ok(surface.format_call(request, blob, opts)?)
    }

    /// Decode `bytes`, the contents of the file `name`, with the plugin that handles its extension.
    pub fn read(&self, name: &str, bytes: &[u8]) -> Result<ImageDoc, FormatError> {
        let ext = wire::extension_of(name).ok_or_else(|| FormatError::NoExtension(name.to_owned()))?;
        let answer = self.call(&ext, &wire::read_request(name, &ext), bytes)?;
        Ok(wire::parse_read_response(&answer)?)
    }

    /// Encode `doc` as the file `name` with the plugin that handles its extension.
    pub fn write(&self, name: &str, doc: &ImageDoc) -> Result<Vec<u8>, FormatError> {
        let ext = wire::extension_of(name).ok_or_else(|| FormatError::NoExtension(name.to_owned()))?;
        let answer = self.call(&ext, &wire::write_request(name, &ext, doc.width, doc.height), &doc.rgba8)?;
        Ok(wire::parse_write_response(&answer)?)
    }
}

impl SurfaceSink for FormatBook {
    fn add(&self, surface: Surface) {
        if surface.contribution.kind != Kind::Format {
            return;
        }
        let mut items = self.items();
        items.retain(|s| !(s.owner == surface.owner && s.contribution.id == surface.contribution.id));
        items.push(surface);
    }

    fn remove(&self, owner: &Owner) {
        self.items().retain(|s| &s.owner != owner);
    }
}
