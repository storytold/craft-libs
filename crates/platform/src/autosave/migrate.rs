//! Moving a legacy recovery folder into the new one, without ever replacing anything.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// The lock file, beside the recovery folder, that keeps two launches from moving the same copies.
const LOCK: &str = "recovery-migration.lock";

/// How long a launch waits for another one's move.
pub(super) const WAIT: Duration = Duration::from_secs(5);

/// Suffix of a copy staged for publishing: disposable, the original still exists.
const STAGED: &str = ".moving";

/// Suffix of an original set aside while it is copied to another volume: it may be the only
/// copy, so it is put back, never thrown away, if the move doesn't finish.
const HELD: &str = ".held";

/// How deep [`copy_tree`] follows folders.
const MAX_COPY_DEPTH: u32 = 16;

/// Migration locks of the old folders this process uses, kept until it ends.
static KEPT_LOCKS: std::sync::Mutex<Vec<std::fs::File>> = std::sync::Mutex::new(Vec::new());

pub(super) fn adopt_waiting(legacy: &Path, dir: &Path, wait: Duration) -> PathBuf {
    let held_aside = || legacy.parent().is_some_and(|p| !side_entries(p, legacy, HELD).is_empty());
    if legacy == dir || (!legacy.is_dir() && !held_aside()) {
        return dir.to_path_buf();
    }
    // An empty old folder has nothing to move (`remove_dir` only removes an empty one).
    if !held_aside() && std::fs::remove_dir(legacy).is_ok() {
        return dir.to_path_buf();
    }
    let unmoved = || if dir.exists() || !legacy.is_dir() { dir.to_path_buf() } else { legacy.to_path_buf() };
    let Some(parent) = dir.parent() else { return unmoved() };
    let lock =
        std::fs::create_dir_all(parent).and_then(|()| std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(parent.join(LOCK)));
    // No lock file means no other process can be moving: the old folder is safe to use.
    let Ok(lock) = lock else { return unmoved() };
    let deadline = Instant::now() + wait;
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            // Another process is moving and may yet remove the old folder: never write there.
            Err(std::fs::TryLockError::WouldBlock) => return dir.to_path_buf(),
            // Locking fails for every process here alike, so none can be moving.
            Err(std::fs::TryLockError::Error(_)) => return unmoved(),
        }
    }
    // The lock file stays (removing it would race a process about to lock it); the OS
    // releases the lock when `lock` is dropped or the process ends.
    let used = adopt_locked(legacy, dir);
    if used == legacy {
        // This process writes to the old folder: it keeps the lock for as long as it runs, so
        // no other launch moves (and removes) the folder under it.
        KEPT_LOCKS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(lock);
    }
    used
}

/// The move itself, under the lock. First what a process that ended mid-move left is tidied:
/// originals it set aside are put back, copies it staged are removed. An empty `legacy` is
/// then just removed. Otherwise the whole folder moves if `dir` doesn't exist yet; when both
/// exist, each copy moves as a whole (copy first, sidecar last) unless `dir` already has
/// either part, and one that stays is left in `legacy` untouched.
fn adopt_locked(legacy: &Path, dir: &Path) -> PathBuf {
    if let Some(parent) = legacy.parent() {
        for held in side_entries(parent, legacy, HELD) {
            put_back(&held, legacy);
        }
    }
    // Parts held aside inside the old folder by a per-copy move.
    let held_parts: Vec<(PathBuf, String)> = std::fs::read_dir(legacy)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().and_then(|n| side_of(n, HELD)).map(|orig| (e.path(), orig.to_owned())))
        .collect();
    for (held, original) in held_parts {
        put_back(&held, &legacy.join(original));
    }
    for folder in [dir.parent(), Some(dir)].into_iter().flatten() {
        remove_stale_staging(folder);
    }
    // Another process may have finished the move while this one waited.
    if !legacy.is_dir() {
        return dir.to_path_buf();
    }
    if std::fs::remove_dir(legacy).is_ok() {
        return dir.to_path_buf();
    }
    if !dir.exists() {
        // The common case: the whole folder moves at once.
        return match move_path(legacy, dir) {
            Ok(()) => dir.to_path_buf(),
            Err(_) if legacy.is_dir() => legacy.to_path_buf(),
            Err(_) => dir.to_path_buf(),
        };
    }
    move_entries(legacy, dir);
    // Only an emptied folder goes; anything that couldn't move stays for a later launch.
    let _ = std::fs::remove_dir(legacy);
    dir.to_path_buf()
}

/// Moves each copy of `from` into `to` (both folders): the copy first, then its sidecar,
/// skipped if `to` has any part. Temp files of an interrupted write go along too.
fn move_entries(from: &Path, to: &Path) {
    let Ok(entries) = std::fs::read_dir(from) else { return };
    let mut keys: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut temps = Vec::new();
    for name in entries.flatten().filter_map(|e| e.file_name().into_string().ok()) {
        if name.starts_with('.') {
            if name.ends_with(".craft_tmp") {
                temps.push(name);
            }
        } else if let Some((stem, _)) = name.split_once('.') {
            keys.entry(stem.to_owned()).or_default().push(name);
        }
    }
    for mut parts in keys.into_values() {
        if parts.iter().any(|p| std::fs::symlink_metadata(to.join(p)).is_ok()) {
            continue;
        }
        // A sidecar must never move without its copy.
        parts.sort_by_key(|p| p.ends_with(".json"));
        for part in &parts {
            if move_path(&from.join(part), &to.join(part)).is_err() {
                break;
            }
        }
    }
    for name in temps {
        let _ = move_path(&from.join(&name), &to.join(&name));
    }
}

fn already_exists(path: &Path) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::AlreadyExists, format!("{} already exists", path.display()))
}

/// A path beside `of`, named `.<name>.<pid>-<n><suffix>`, that no other call uses.
fn side_path(of: &Path, suffix: &str) -> std::io::Result<PathBuf> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let parent = of.parent().ok_or_else(|| std::io::Error::other(format!("{} has no parent folder", of.display())))?;
    let name = of.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(parent.join(format!(".{name}.{}-{}{suffix}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))))
}

/// The name `side` was made from, if it is a [`side_path`] name with `suffix`.
fn side_of<'a>(side: &'a str, suffix: &str) -> Option<&'a str> {
    let rest = side.strip_prefix('.')?.strip_suffix(suffix)?;
    let (name, ids) = rest.rsplit_once('.')?;
    let (pid, n) = ids.split_once('-')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (!name.is_empty() && digits(pid) && digits(n)).then_some(name)
}

/// The side paths with `suffix` in `folder` made from `of`.
fn side_entries(folder: &Path, of: &Path, suffix: &str) -> Vec<PathBuf> {
    let Some(name) = of.file_name().and_then(|n| n.to_str()) else { return Vec::new() };
    std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_str().and_then(|n| side_of(n, suffix)) == Some(name))
        .map(|e| e.path())
        .collect()
}

/// Moves the file or folder `from` to `to` without ever replacing anything at `to`. On one
/// volume a folder is renamed and a file is hard-linked, then unlinked at `from`. Otherwise
/// (another volume) it is copied ([`move_across`]).
fn move_path(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(already_exists(to));
    }
    let meta = std::fs::symlink_metadata(from)?;
    let moved = if meta.is_dir() { std::fs::rename(from, to) } else { std::fs::hard_link(from, to).and_then(|()| std::fs::remove_file(from)) };
    match moved {
        Ok(()) => Ok(()),
        Err(e) if matches!(e.kind(), std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::DirectoryNotEmpty) => Err(e),
        Err(_) => move_across(from, to),
    }
}

/// Moves `from` to `to` by copying. `from` is first renamed aside on its own volume, so a
/// process still writing to the old path starts a new `from` there instead of adding to what
/// is copied. The held original is copied to a staged path, published without replacing
/// anything, and only then removed. If the copy or publish fails, the original is put back.
fn move_across(from: &Path, to: &Path) -> std::io::Result<()> {
    let held = side_path(from, HELD)?;
    std::fs::rename(from, &held)?;
    match stage_copy(&held, to).and_then(|staged| publish(&staged, to)) {
        Ok(()) => {
            if std::fs::symlink_metadata(&held)?.is_dir() {
                std::fs::remove_dir_all(&held)
            } else {
                std::fs::remove_file(&held)
            }
        }
        Err(e) => {
            put_back(&held, from);
            Err(e)
        }
    }
}

/// Returns the held original `held` to `original`. If `original` has been made again
/// meanwhile, the held folder's entries move into it without replacing anything; whatever
/// can't move stays held and is tried again under the next move's lock. Nothing held is ever
/// removed with data in it.
fn put_back(held: &Path, original: &Path) {
    if std::fs::symlink_metadata(original).is_err() && std::fs::rename(held, original).is_ok() {
        return;
    }
    if held.is_dir() && original.is_dir() {
        move_entries(held, original);
        let _ = std::fs::remove_dir(held);
    }
}

/// Copies `from` to a staging path beside `to` that this call creates exclusively. On failure
/// the staged copy is removed.
fn stage_copy(from: &Path, to: &Path) -> std::io::Result<PathBuf> {
    let staged = side_path(to, STAGED)?;
    let is_dir = std::fs::symlink_metadata(from)?.is_dir();
    let copied = if is_dir { copy_tree(from, &staged, 0) } else { copy_file(from, &staged) };
    if let Err(e) = copied {
        // `AlreadyExists` means `staged` was someone else's to keep.
        if e.kind() != std::io::ErrorKind::AlreadyExists {
            let _ = if is_dir { std::fs::remove_dir_all(&staged) } else { std::fs::remove_file(&staged) };
        }
        return Err(e);
    }
    Ok(staged)
}

/// Removes copies a process left staged in `folder` when it ended mid-move: disposable, since
/// the original is still in place or held aside. Links are never followed.
fn remove_stale_staging(folder: &Path) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        if entry.file_name().to_str().is_none_or(|n| side_of(n, STAGED).is_none()) {
            continue;
        }
        let _ = match entry.file_type() {
            Ok(t) if t.is_dir() => std::fs::remove_dir_all(entry.path()),
            Ok(t) if t.is_file() => std::fs::remove_file(entry.path()),
            _ => continue,
        };
    }
}

/// Gives the staged copy its final name `to` without replacing anything there. On failure the
/// staged copy is removed and `to` is untouched.
fn publish(staged: &Path, to: &Path) -> std::io::Result<()> {
    let is_dir = std::fs::symlink_metadata(staged)?.is_dir();
    let published = if std::fs::symlink_metadata(to).is_ok() {
        Err(already_exists(to))
    } else if is_dir {
        std::fs::rename(staged, to)
    } else {
        std::fs::hard_link(staged, to).and_then(|()| std::fs::remove_file(staged))
    };
    if published.is_err() {
        let _ = if is_dir { std::fs::remove_dir_all(staged) } else { std::fs::remove_file(staged) };
    }
    published
}

/// Copies the folder `from` to the new folder `to`: plain files and folders only, at most
/// [`MAX_COPY_DEPTH`] levels.
fn copy_tree(from: &Path, to: &Path, depth: u32) -> std::io::Result<()> {
    if depth > MAX_COPY_DEPTH {
        return Err(std::io::Error::other(format!("{} is nested too deep", from.display())));
    }
    std::fs::create_dir(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target, depth.saturating_add(1))?;
        } else {
            copy_file(&entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Copies one plain file to the new file `to`, flushed to disk.
fn copy_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if !std::fs::symlink_metadata(from)?.file_type().is_file() {
        return Err(std::io::Error::other(format!("{} is not a plain file", from.display())));
    }
    let mut src = std::fs::File::open(from)?;
    let mut dst = std::fs::OpenOptions::new().write(true).create_new(true).open(to)?;
    std::io::copy(&mut src, &mut dst)?;
    dst.sync_all()
}
