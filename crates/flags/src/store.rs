//! Reading and changing the flags file (desktop only).

use std::fmt;
use std::path::Path;

use crate::{ENV, Flags, MAX_FILE_BYTES, default_path, valid_id};

/// Why a flag could not be changed.
#[derive(Debug)]
pub enum FlagError {
    /// Not a flag name (see [`valid_id`]).
    BadId(String),
    /// The file exists but is not a flags file this crate can change without losing what is in
    /// it (it does not parse, is too big, or `flags` is not a table). It is left as it is.
    Unreadable(String),
    /// Reading or writing the file failed.
    Io(std::io::Error),
}

impl fmt::Display for FlagError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FlagError::BadId(id) => write!(f, "`{}` is not a flag name", id.escape_debug()),
            FlagError::Unreadable(why) => write!(f, "the flags file was left unchanged: {why}"),
            FlagError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for FlagError {}

/// The file's text, `None` when it does not exist. A file that isn't a regular file, is too big
/// or isn't UTF-8 is an error.
fn read_text(path: &Path) -> Result<Option<String>, FlagError> {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(FlagError::Io(e)),
    };
    if !meta.is_file() {
        return Err(FlagError::Unreadable(format!("{} is not a regular file", path.display())));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(FlagError::Unreadable(format!("{} is larger than {MAX_FILE_BYTES} bytes", path.display())));
    }
    let bytes = std::fs::read(path).map_err(FlagError::Io)?;
    String::from_utf8(bytes).map(Some).map_err(|_| FlagError::Unreadable(format!("{} is not UTF-8 text", path.display())))
}

/// The flags of this run: [`default_path`] and `CRAFT_FLAGS`. Never fails; what could not be
/// read is in [`Flags::problems`].
pub fn load() -> Flags {
    let env = std::env::var(ENV).ok();
    let Some(path) = default_path() else {
        return Flags::from_parts(None, env.as_deref());
    };
    match read_text(&path) {
        Ok(text) => Flags::from_parts(text.as_deref(), env.as_deref()),
        Err(e) => {
            let mut flags = Flags::from_parts(None, env.as_deref());
            flags.problems.insert(0, format!("{}: {e}; every flag is at its default", path.display()));
            flags
        }
    }
}

/// Turn the flag `id` on or off in the file at `path` (made if missing). Everything else in the
/// file, comments included, is kept; the write is atomic.
pub fn set(path: &Path, id: &str, on: bool) -> Result<(), FlagError> {
    if !valid_id(id) {
        return Err(FlagError::BadId(id.to_string()));
    }
    let mut doc = match read_text(path)? {
        Some(text) => text.parse::<toml_edit::DocumentMut>().map_err(|e| FlagError::Unreadable(e.to_string()))?,
        None => toml_edit::DocumentMut::new(),
    };
    let item = doc.entry("flags").or_insert_with(toml_edit::table);
    let Some(table) = item.as_table_like_mut() else {
        return Err(FlagError::Unreadable("`flags` is not a table".into()));
    };
    table.insert(id, toml_edit::value(on));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(FlagError::Io)?;
    }
    craft_atomic::atomic_write(path, doc.to_string().as_bytes()).map_err(FlagError::Io)
}
