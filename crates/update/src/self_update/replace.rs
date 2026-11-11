//! Swapping the running launcher for a new copy: AppImage, standalone tarball and release tree.

use super::detect::marker;
#[cfg(unix)]
use super::detect::prefix_with_share;
use super::helper::{done_file, failure_file, read_record};
use super::*;

/// Records `release` as just installed, so it isn't offered again for [`super::detect::JUST_UPDATED_FOR`]: its
/// tag, the time, and the version it installs. The outcome records of an earlier update are
/// removed: this update writes its own.
pub(super) fn mark_installed(paths: &Paths, release: &Release) {
    let _ = std::fs::remove_file(failure_file(paths));
    let _ = std::fs::remove_file(done_file(paths));
    let text = format!("{}\n{}\n{}\n", release.tag, crate::now(), release.version_label());
    if let Err(e) = crate::paths::write_atomic(&marker(paths), text.as_bytes()) {
        log::warn!("couldn't record the self-update: {e}");
    }
}

/// Where a self-update records the name and size of the release file it downloads. The file stays
/// in the download folder after a failed install, and the next try checks it against its
/// published checksum and skips the transfer. On Windows the update helper removes the download
/// itself, so nothing is recorded there.
#[cfg(not(windows))]
pub(super) fn kept_download_record(paths: &Paths) -> PathBuf {
    paths.cache.join("self-update-download.txt")
}

/// Records `asset` as this app's self-update download. A finished or partial download of another
/// release file that the record named is removed first, so the download folder holds the files of
/// one self-update download at most. Returns whether the record was written.
#[cfg(not(windows))]
pub(super) fn keep_download(paths: &Paths, asset: &crate::release::Asset) -> bool {
    let record = kept_download_record(paths);
    if let Some(text) = read_record(&record) {
        let mut lines = text.lines();
        let (name, size) = (lines.next().unwrap_or_default(), lines.next().and_then(|s| s.parse::<u64>().ok()));
        if (name, size) != (asset.name.as_str(), Some(asset.size)) && crate::release::valid_file_name(name) {
            let dest = paths.downloads().join(name);
            match size {
                Some(size) => crate::http::discard(&dest, size),
                // A record without a size still names a finished download.
                None => {
                    let _ = std::fs::remove_file(dest);
                }
            }
        }
    }
    let text = format!("{}\n{}\n", asset.name, asset.size);
    match crate::paths::write_atomic(&record, text.as_bytes()) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("couldn't record the self-update download: {e}");
            false
        }
    }
}

/// Keeps the download `file` for the next try when the install's `result` is a failure that
/// [`keeps_download`] allows and [`keep_download`] `recorded` it; otherwise removes the file and
/// its record. A download without a record is removed, since nothing would remove it later.
#[cfg(not(windows))]
pub(super) fn after_install<T>(paths: &Paths, file: &Path, recorded: bool, result: &Result<T, String>) {
    if !(recorded && keeps_download(result)) {
        let _ = std::fs::remove_file(file);
        let _ = std::fs::remove_file(kept_download_record(paths));
    }
}

/// Whether the download stays for the next try after the install's `result`. A failed install
/// keeps it, except on macOS when the disk image could not be opened: an image still attached
/// from an install that ended part way makes hdiutil refuse every attach of the same file with
/// "Resource busy", while a new download of it attaches.
#[cfg(not(windows))]
pub(super) fn keeps_download<T>(result: &Result<T, String>) -> bool {
    match result {
        Ok(_) => false,
        #[cfg(target_os = "macos")]
        Err(e) if e.starts_with(install::macos::IMAGE_NOT_OPENED) => false,
        Err(_) => true,
    }
}

/// Removes the record of [`mark_installed`] when the swap couldn't be scheduled, so the update
/// stays on offer.
pub(super) fn unmark_unless_scheduled<T>(paths: &Paths, scheduled: Result<T, String>) -> Result<T, String> {
    if scheduled.is_err() {
        let _ = std::fs::remove_file(marker(paths));
    }
    scheduled
}

/// Copies the new AppImage beside the running one and renames it over it. The running copy is
/// mounted from its own (now unlinked) file, so it keeps working until it exits.
#[cfg(target_os = "linux")]
pub(super) fn replace_appimage(new: &Path, path: &Path) -> Result<(), String> {
    path.parent().ok_or("the AppImage isn't in a folder")?;
    // The replacement AppImage must be executable; 0o755 is not group/world-writable.
    let staged = stage_copy(new, path, 0o755)?; // NOSONAR
    std::fs::rename(&staged, path).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        format!("couldn't replace {} ({e})", path.display())
    })
}

/// Replaces a standalone copy from a release tarball, along with what the tarball install put
/// beside it: the release's other programs that are already in the executable's folder, and,
/// when that folder is a prefix's `bin` with the app's `share/<id>` folder next to it, everything
/// under the tarball's `share` (data, desktop entry, icons). A copy on its own in some folder gets
/// only its executable. Files the release doesn't ship are never touched. Every file is copied
/// next to its destination before any is renamed over it, so a copy that fails (full disk,
/// unwritable folder) changes nothing; the executable is renamed last.
#[cfg(unix)]
pub(super) fn replace_standalone(app: &App, paths: &Paths, tarball: &Path, exe: &Path) -> Result<(), String> {
    // Per-process names, so two updates running at once can't remove each other's files.
    let staging = paths.cache.join(format!("{}.standalone.{}", app.id, std::process::id()));
    let result = install::tree::unpack(app, tarball, &staging).and_then(|tree| replace_from_tree(app, &tree, exe));
    remove_staging(&staging);
    result
}

/// Removes an unpacked release. The archive sets each folder's mode, and a folder its owner can't
/// list or write can't be emptied, so those folders get owner access back first.
#[cfg(unix)]
pub(super) fn remove_staging(staging: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if std::fs::remove_dir_all(staging).is_ok() || std::fs::symlink_metadata(staging).is_err() {
        return;
    }
    let mut pending = vec![staging.to_path_buf()];
    while let Some(dir) = pending.pop() {
        if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.file_type().is_dir()) {
            continue;
        }
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                pending.push(entry.path());
            }
        }
    }
    let _ = std::fs::remove_dir_all(staging);
}

#[cfg(unix)]
pub(super) fn replace_from_tree(app: &App, tree: &Path, exe: &Path) -> Result<(), String> {
    let new_exe = tree.join("bin").join(app.id);
    if !is_plain_file(&new_exe) {
        return Err(format!("release archive did not contain binary for {}", app.id));
    }
    // A `bin` that is a link would bring in programs from outside the release.
    let release_bin = tree.join("bin");
    if !std::fs::symlink_metadata(&release_bin).is_ok_and(|m| m.file_type().is_dir()) {
        return Err(format!("{} isn't a folder", release_bin.display()));
    }
    let bin = exe.parent().ok_or_else(|| "executable isn't in a folder".to_string())?;
    let mut moves = Vec::new();
    for entry in std::fs::read_dir(&release_bin).map_err(|e| format!("couldn't read the release's programs ({e})"))? {
        let entry = entry.map_err(|e| format!("couldn't read {} ({e})", release_bin.display()))?;
        let name = entry.file_name();
        if name != app.id && is_plain_file(&entry.path()) && bin.join(&name).is_file() {
            moves.push((entry.path(), bin.join(&name)));
        }
    }
    if let Some(prefix) = prefix_with_share(app, bin) {
        for file in plain_files(&tree.join("share"))? {
            let relative = file.strip_prefix(tree).map_err(|_| "staged file escapes the release".to_string())?;
            moves.push((file.clone(), prefix.join(relative)));
        }
    }
    moves.push((new_exe, exe.to_path_buf()));
    // A folder where a file goes would fail its rename after earlier files were replaced.
    if let Some((_, to)) = moves.iter().find(|(_, to)| to.symlink_metadata().is_ok_and(|m| m.is_dir())) {
        return Err(format!("couldn't replace {} (it's a folder)", to.display()));
    }
    // Copy everything next to its destination first, so a full disk or an unwritable folder
    // stops the update before any installed file changes. Renames within a folder don't fail
    // for those reasons, and the executable is renamed last.
    let mut staged = Vec::new();
    for (i, (from, to)) in moves.iter().enumerate() {
        match stage(from, to, i == moves.len() - 1) {
            Ok(s) => staged.push((s, to)),
            Err(e) => {
                for (s, _) in &staged {
                    let _ = std::fs::remove_file(s);
                }
                return Err(e);
            }
        }
    }
    let mut result = Ok(());
    for (s, to) in &staged {
        if result.is_ok() {
            result = std::fs::rename(s, to).map_err(|e| format!("couldn't replace {} ({e})", to.display()));
        }
        if result.is_err() {
            let _ = std::fs::remove_file(s);
        }
    }
    result
}

/// Copies `from` to a hidden file next to `to` and returns its path. Group and world write and
/// the set-id bits are never carried over from the archive; the executable is always 0755.
#[cfg(unix)]
pub(super) fn stage(from: &Path, to: &Path, executable: bool) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    let dir = to.parent().ok_or_else(|| format!("{} isn't in a folder", to.display()))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {} ({e})", dir.display()))?;
    let mode = std::fs::metadata(from).map(|m| m.permissions().mode()).unwrap_or(0o644);
    stage_copy(from, to, if executable { 0o755 } else { mode & 0o755 })
}

/// Copies `from` to a new file beside `to` and returns its path. The file is created exclusively
/// under an unpredictable name, so a link someone left at a guessable name is never followed,
/// and its bytes and `mode` go through the open handle. Removed again when any step fails.
#[cfg(unix)]
pub(super) fn stage_copy(from: &Path, to: &Path, mode: u32) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    let dir = to.parent().ok_or_else(|| format!("{} isn't in a folder", to.display()))?;
    let write_error = |e: std::io::Error| format!("couldn't write next to {} ({e})", to.display());
    // fixed prefix: the temp name's length must not grow with the destination's basename
    let mut staged = tempfile::Builder::new().prefix(".staged-").suffix(".update").tempfile_in(dir).map_err(write_error)?;
    let mut source = std::fs::File::open(from).map_err(|e| format!("couldn't read {} ({e})", from.display()))?;
    std::io::copy(&mut source, staged.as_file_mut()).map_err(write_error)?;
    staged.as_file().set_permissions(std::fs::Permissions::from_mode(mode)).map_err(|e| e.to_string())?;
    staged.into_temp_path().keep().map_err(|e| e.to_string())
}

/// A regular file, not a link to one.
pub(super) fn is_plain_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file())
}

/// The regular files under `dir`, without following links; none when `dir` doesn't exist. Links
/// and other entries that aren't files or folders are skipped. A `dir` that is a link or a file, and
/// a folder that can't be read, are errors, so an update never goes ahead with its files left out.
#[cfg(unix)]
pub(super) fn plain_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    match std::fs::symlink_metadata(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("couldn't read {} ({e})", dir.display())),
        Ok(m) if !m.file_type().is_dir() => return Err(format!("{} isn't a folder", dir.display())),
        Ok(_) => {}
    }
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("couldn't read {} ({e})", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("couldn't read {} ({e})", dir.display()))?;
            let kind = entry.file_type().map_err(|e| format!("couldn't read {} ({e})", entry.path().display()))?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                files.push(entry.path());
            }
        }
    }
    Ok(files)
}
