//! Reading and changing the flags file (desktop only).

use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::{ENV, FILE_ENV, Flags, MAX_FILE_BYTES, default_path, valid_id};

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
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(FlagError::Io(e)),
    };
    let meta = file.metadata().map_err(FlagError::Io)?;
    if !meta.is_file() {
        return Err(FlagError::Unreadable(format!("{} is not a regular file", path.display())));
    }
    // Read at most one byte past the limit from the file opened: one that grew or was replaced
    // after a size check can't bring more.
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes).map_err(FlagError::Io)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(FlagError::Unreadable(format!("{} is larger than {MAX_FILE_BYTES} bytes", path.display())));
    }
    String::from_utf8(bytes).map(Some).map_err(|_| FlagError::Unreadable(format!("{} is not UTF-8 text", path.display())))
}

/// The flags of this run: [`default_path`] and `CRAFT_FLAGS`. Never fails; what could not be
/// read is in [`Flags::problems`].
pub fn load() -> Flags {
    let named = std::env::var_os(FILE_ENV).is_some_and(|p| !p.is_empty());
    load_from(default_path(), named, std::env::var(ENV).ok().as_deref())
}

/// [`load`] from `path`; `named` when `CRAFT_FLAGS_FILE` chose it, so a missing file is worth a
/// word (the usual file not existing yet is not).
pub(crate) fn load_from(path: Option<PathBuf>, named: bool, env: Option<&str>) -> Flags {
    let Some(path) = path else {
        return Flags::from_parts(None, env);
    };
    match read_text(&path) {
        Ok(None) if named => {
            let mut flags = Flags::from_parts(None, env);
            flags.problems.insert(0, format!("{}, named by {FILE_ENV}, does not exist; every flag is at its default", path.display()));
            flags
        }
        Ok(text) => Flags::from_parts(text.as_deref(), env),
        Err(e) => {
            let mut flags = Flags::from_parts(None, env);
            flags.problems.insert(0, format!("{}: {e}; every flag is at its default", path.display()));
            flags
        }
    }
}

/// Turn the flag `id` on or off in the file at `path` (made if missing). Everything else in the
/// file, comments included, is kept; the write is atomic. Two programs changing flags at once
/// (the launcher and an app) take turns, so neither change is lost.
pub fn set(path: &Path, id: &str, on: bool) -> Result<(), FlagError> {
    if !valid_id(id) {
        return Err(FlagError::BadId(id.to_string()));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(FlagError::Io)?;
    }
    // A lock file beside the flags file, which itself is replaced on every write. Released when
    // `_lock` is dropped (and by the system if the process ends).
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    let _lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(PathBuf::from(lock_path)).map_err(FlagError::Io)?;
    _lock.lock().map_err(FlagError::Io)?;
    let mut doc = match read_text(path)? {
        Some(text) => text.parse::<toml_edit::DocumentMut>().map_err(|e| FlagError::Unreadable(e.to_string()))?,
        None => toml_edit::DocumentMut::new(),
    };
    let item = doc.entry("flags").or_insert_with(toml_edit::table);
    let Some(table) = item.as_table_like_mut() else {
        return Err(FlagError::Unreadable("`flags` is not a table".into()));
    };
    // Keep what surrounds the old value, such as a comment after it.
    let mut value = toml_edit::Value::from(on);
    if let Some(old) = table.get(id).and_then(toml_edit::Item::as_value) {
        *value.decor_mut() = old.decor().clone();
    }
    table.insert(id, toml_edit::Item::Value(value));
    let text = doc.to_string();
    // What can't be read back is not written: the file stays as it was.
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(FlagError::Unreadable(format!("the flags file would grow past {MAX_FILE_BYTES} bytes")));
    }
    craft_atomic::atomic_write(path, text.as_bytes()).map_err(FlagError::Io)
}
