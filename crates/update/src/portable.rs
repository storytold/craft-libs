//! Portable Windows packages: bounded extraction and replacement of recorded files only.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use crate::manifest::Record;
use crate::paths::Paths;
use crate::{App, PackageKind};

const MAX_FILES: usize = 10_000;
const MAX_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Custom destinations may have OS-managed links in their host ancestors.
pub fn validate_directory(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return Err("Choose an absolute installation folder without parent-directory components.".into());
    }
    // Resolve the closest existing ancestor: /home and /var can be OS-managed
    // symlinks, and a custom destination need not exist yet.
    for ancestor in path.ancestors() {
        match std::fs::canonicalize(ancestor) {
            Ok(resolved) => {
                if !resolved.is_dir() {
                    return Err("The installation folder contains a non-directory component.".into());
                }
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // A dangling link is not a missing directory we can safely create.
                if std::fs::symlink_metadata(ancestor).is_ok() {
                    return Err("The installation folder contains a dangling link.".into());
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("The installation folder has no existing ancestor.".into())
}

/// Links are allowed above the ownership boundary, never inside it (including root).
fn validate_owned_directory(root: &Path, path: &Path) -> Result<(), String> {
    validate_directory(path)?;
    if !path.starts_with(root) {
        return Err("An installation path points outside its folder.".into());
    }
    for ancestor in path.ancestors().take_while(|ancestor| ancestor.starts_with(root)) {
        match std::fs::symlink_metadata(ancestor) {
            Ok(meta) if linked(&meta) || !meta.is_dir() => {
                return Err("The installation folder contains a link or a non-directory component.".into());
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.to_string()),
            _ => {}
        }
    }
    Ok(())
}

fn resolve_missing(path: &Path) -> Result<PathBuf, String> {
    for ancestor in path.ancestors() {
        match std::fs::canonicalize(ancestor) {
            Ok(resolved) => {
                let suffix = path.strip_prefix(ancestor).map_err(|e| e.to_string())?;
                return Ok(resolved.join(suffix));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if std::fs::symlink_metadata(ancestor).is_ok() {
                    return Err("The installation folder contains a dangling link.".into());
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("The installation folder has no existing ancestor.".into())
}

/// Whether two paths name the same Windows folder or file: case, trailing separators and the
/// verbatim `\\?\` form that `canonicalize` returns don't count.
fn same_windows_path(left: &Path, right: &Path) -> bool {
    /// `canonicalize` writes `\\?\C:\…` and, for a network share, `\\?\UNC\server\share\…`;
    /// Windows reads either prefix in any case.
    fn normalized(path: &Path) -> String {
        let text = path.to_string_lossy().to_lowercase();
        let text = match text.strip_prefix(r"\\?\unc\") {
            Some(share) => format!(r"\\{share}"),
            None => text.strip_prefix(r"\\?\").unwrap_or(&text).to_string(),
        };
        text.trim_end_matches(['\\', '/']).to_string()
    }
    normalized(left) == normalized(right)
}

fn linked(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

struct Staging(PathBuf);
impl Staging {
    /// Stop cleaning up and hand the directory back: used when a rollback failed and the backup
    /// is the only copy left for manual recovery.
    fn keep(self) -> PathBuf {
        let path = self.0.clone();
        // Deliberately skipping Drop here is the whole point — it would delete the recovery
        // files. The directory is handed to the operator, not leaked into nothing.
        std::mem::forget(self); // NOSONAR
        path
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn extract(file: &Path, destination: &Path) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
    let file = std::fs::File::open(file).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("couldn't read the portable ZIP ({e})"))?;
    if archive.len() > MAX_FILES {
        return Err("The portable ZIP contains too many files.".into());
    }
    let mut files = Vec::new();
    let mut entries = Vec::new();
    let mut total = 0u64;
    let mut seen = std::collections::BTreeSet::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let relative = entry.enclosed_name().ok_or("The portable ZIP contains an unsafe path.")?;
        if relative.components().any(|c| !matches!(c, Component::Normal(_)))
            || entry.name().map_or(true, |name| name.contains(['\\', ':']))
            || entry.unix_mode().is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("The portable ZIP contains an unsafe path or link.".into());
        }
        // Windows treats case variants as the same file; reject duplicate and colliding names.
        if !seen.insert(relative.to_string_lossy().to_lowercase()) {
            return Err("The portable ZIP contains duplicate paths.".into());
        }
        let target = destination.join(&relative);
        entries.push(relative.clone());
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        total = total.checked_add(entry.size()).filter(|size| *size <= MAX_BYTES).ok_or("The portable ZIP is too large.")?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(&target).map_err(|e| e.to_string())?;
        use std::io::Read;
        let declared = entry.size();
        let copied = std::io::copy(&mut (&mut entry).take(declared.saturating_add(1)), &mut output).map_err(|e| e.to_string())?;
        if copied != declared {
            return Err("The portable ZIP has an unexpected unpacked size.".into());
        }
        files.push(relative);
    }
    Ok((files, entries))
}

fn recorded_files<'a>(app: &App, record: &'a Record) -> Result<(&'a Path, Vec<PathBuf>), String> {
    let root = record.path.parent().ok_or("The portable installation has no parent folder.")?;
    if record.kind != PackageKind::PortableZip || root.file_name().is_none_or(|n| n != app.id) {
        return Err("The launcher has no valid ownership record for this portable folder.".into());
    }
    let resolved = resolve_missing(root)?;
    if !same_windows_path(root, &resolved) {
        return Err("The recorded portable folder resolves through a changed alias. Reinstall it to choose a safe location.".into());
    }
    validate_owned_directory(root, root)?;
    let mut files = Vec::new();
    for file in &record.files {
        let relative = file.strip_prefix(root).map_err(|_| "An installation record points outside its folder.")?;
        if relative.as_os_str().is_empty() || relative.components().any(|c| !matches!(c, Component::Normal(_))) {
            return Err("The installation record contains an unsafe path.".into());
        }
        validate_owned_directory(root, file.parent().ok_or("A recorded file has no parent folder.")?)?;
        if let Ok(meta) = std::fs::symlink_metadata(file)
            && (linked(&meta) || !meta.is_file())
        {
            return Err("A recorded file was replaced by a link or directory.".into());
        }
        files.push(relative.to_path_buf());
    }
    Ok((root, files))
}

/// Keeps existing portable installations at their location and preserves unrecorded user files,
/// unless `replace` allows writing over them.
#[allow(clippy::too_many_arguments)]
pub fn install(
    app: &App,
    file: &Path,
    version: &str,
    previous: Option<&Record>,
    paths: &Paths,
    directory: Option<&Path>,
    replace: bool,
    commit: &mut dyn FnMut(&Record) -> Result<(), String>,
) -> Result<Record, String> {
    let (destination, mut old_files) = match previous {
        Some(record) => {
            let (root, files) = recorded_files(app, record)?;
            (root.to_path_buf(), files)
        }
        None => {
            let parent = directory.map(Path::to_path_buf).unwrap_or_else(|| paths.apps());
            validate_directory(&parent)?;
            let parent = resolve_missing(&parent)?;
            (parent.join(app.id), Vec::new())
        }
    };
    validate_owned_directory(&destination, &destination)?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos();
    let staging = paths.cache.join("staging").join(format!("{}-{}-{stamp}", app.id, std::process::id()));
    validate_owned_directory(&paths.cache, &staging)?;
    std::fs::create_dir_all(staging.parent().ok_or("no staging folder")?).map_err(|e| e.to_string())?;
    std::fs::create_dir(&staging).map_err(|e| e.to_string())?;
    let staging = Staging(staging);
    let unpacked = staging.0.join("unpacked");
    std::fs::create_dir(&unpacked).map_err(|e| e.to_string())?;
    let (files, entries) = extract(file, &unpacked)?;
    // Accept both a flat layout (files at the ZIP root) and a single wrapper-folder layout,
    // but never arbitrary nesting.
    let (unpacked, files) = flatten_zip_layout(app, &unpacked, files, &entries)?;
    let executable = files
        .iter()
        .find(|relative| is_root_executable(relative, app))
        .ok_or("The portable ZIP does not contain the expected executable at its root.")?
        .clone();
    // Validate the entire transaction before moving any existing program file.
    for relative in &files {
        let target = destination.join(relative);
        validate_owned_directory(&destination, target.parent().ok_or("no destination folder")?)?;
        if let Ok(meta) = std::fs::symlink_metadata(&target) {
            let recorded = old_files.iter().any(|old| same_windows_path(old, relative));
            if linked(&meta) || !meta.is_file() || !(recorded || replace) {
                return Err("The installation would overwrite an unrecorded file.".into());
            }
            if !recorded {
                old_files.push(relative.clone());
            }
        }
    }
    std::fs::create_dir_all(&destination).map_err(|e| e.to_string())?;
    let backup = staging.0.join("backup");
    let record = Record {
        version: version.to_string(),
        kind: PackageKind::PortableZip,
        path: destination.join(executable),
        installed_at: crate::now(),
        files: files.iter().map(|relative| destination.join(relative)).collect(),
        needs_root: false,
    };
    let mut moved = Vec::new();
    let mut copied = Vec::new();
    let transaction = (|| {
        for relative in &old_files {
            let old = destination.join(relative);
            if !old.exists() {
                continue;
            }
            let saved = backup.join(relative);
            std::fs::create_dir_all(saved.parent().ok_or("no backup folder")?).map_err(|e| e.to_string())?;
            // Copy the rollback copy: cache and custom installation can be on different drives.
            std::fs::copy(&old, &saved).map_err(|e| e.to_string())?;
            std::fs::remove_file(&old).map_err(|e| e.to_string())?;
            moved.push(relative.clone());
        }
        for relative in &files {
            let target = destination.join(relative);
            std::fs::create_dir_all(target.parent().ok_or("no destination folder")?).map_err(|e| e.to_string())?;
            let mut input = std::fs::File::open(unpacked.join(relative)).map_err(|e| e.to_string())?;
            let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(&target).map_err(|e| e.to_string())?;
            copied.push(relative.clone());
            std::io::copy(&mut input, &mut output).map_err(|e| e.to_string())?;
        }
        commit(&record)?;
        Ok::<(), String>(())
    })();
    if let Err(error) = transaction {
        for relative in copied {
            let _ = std::fs::remove_file(destination.join(relative));
        }
        for relative in moved {
            if let Err(restore) = std::fs::copy(backup.join(&relative), destination.join(&relative)) {
                // Preserve backups for manual recovery if the disk won't accept a rollback.
                let backup_path = staging.keep();
                return Err(format!("{error}; could not restore the previous version ({restore}). Recovery files: {}", backup_path.display()));
            }
        }
        return Err(error);
    }
    Ok(record)
}

/// Whether a ZIP-relative path sits at the archive root and is one of the app's executables.
fn is_root_executable(relative: &Path, app: &App) -> bool {
    relative.parent().is_some_and(|parent| parent.as_os_str().is_empty())
        && relative.file_name().and_then(OsStr::to_str).is_some_and(|name| app.stems().any(|stem| name.eq_ignore_ascii_case(&format!("{stem}.exe"))))
}

/// Flattens a portable ZIP whose files all sit in one top-level folder (the official Windows
/// packaging script's `Compress-Archive -Path $Portable` puts everything under a versioned
/// directory) into the flat layout a root launcher exe requires, passing already-flat layouts
/// through. ZIPs mixing unrelated top-level folders are rejected.
fn flatten_zip_layout(app: &App, unpacked: &Path, files: Vec<PathBuf>, entries: &[PathBuf]) -> Result<(PathBuf, Vec<PathBuf>), String> {
    if files.iter().any(|relative| is_root_executable(relative, app)) {
        return Ok((unpacked.to_path_buf(), files));
    }
    let prefix = files.first().and_then(|path| path.components().next()).ok_or("The portable ZIP contains no files.")?.as_os_str().to_os_string();
    let prefix = PathBuf::from(prefix);
    // Include explicit ZIP directory entries as well as extracted files.
    // The wrapper directory itself is allowed; unrelated roots are not.
    if !entries.iter().all(|path| path.starts_with(&prefix))
        || !files.iter().all(|path| path.strip_prefix(&prefix).is_ok_and(|relative| relative.components().next().is_some()))
    {
        return Err("The portable ZIP has an unsupported directory layout.".into());
    }
    let unpacked = unpacked.join(&prefix);
    let files: Vec<PathBuf> =
        files.into_iter().map(|path| path.strip_prefix(&prefix).map(Path::to_path_buf).map_err(|e| e.to_string())).collect::<Result<Vec<_>, _>>()?;
    Ok((unpacked, files))
}

/// Removes only recorded program files. Personal settings and documents remain.
pub fn remove(app: &App, record: &Record) -> Result<(), String> {
    let (root, files) = recorded_files(app, record)?;
    for relative in &files {
        let file = root.join(relative);
        if file.exists() {
            std::fs::remove_file(&file).map_err(|e| e.to_string())?;
        }
    }
    for relative in &files {
        let mut directory = root.join(relative).parent().map(Path::to_path_buf);
        while let Some(path) = directory.filter(|p| p.starts_with(root)) {
            if std::fs::remove_dir(&path).is_err() || path == root {
                break;
            }
            directory = path.parent().map(Path::to_path_buf);
        }
    }
    Ok(())
}

/// Unpacks the launcher's own portable zip into `staging`, with the same checks as an app's, and
/// copies it over `folder` for a self-update. `check` must accept every unpacked file before
/// any is copied, and again before the backup is removed. The archive must include `executable`.
/// When every file sits in one top-level folder, that folder's contents are the program. A copy
/// that fails puts the previous files back. When one of them can't be put back,
/// the previous files move to a `<staging>-recovery-<time>` folder beside `staging`, which no
/// later update removes, and the error names it. `staging` is removed afterwards.
pub fn unpack_over(
    zip: &Path,
    staging: &Path,
    folder: &Path,
    executable: &std::ffi::OsStr,
    check: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let backup = staging.join("backup");
    // A failed update whose files couldn't be moved to a recovery folder: never clear them.
    if backup.exists() {
        return Err(format!("the previous files of a failed update are in {}; move them out of the way first", backup.display()));
    }
    let _ = std::fs::remove_dir_all(staging);
    let mut keep = false;
    let result = replace_with(zip, staging, folder, executable, check, &mut keep);
    if !keep {
        let _ = std::fs::remove_dir_all(staging);
    }
    result
}

fn replace_with(
    zip: &Path,
    staging: &Path,
    folder: &Path,
    executable: &std::ffi::OsStr,
    check: &dyn Fn(&Path) -> Result<(), String>,
    keep: &mut bool,
) -> Result<(), String> {
    let unpacked = staging.join("unpacked");
    let backup = staging.join("backup");
    std::fs::create_dir_all(&unpacked).map_err(|e| e.to_string())?;
    let (files, _) = extract(zip, &unpacked)?;
    let top = single_top_folder(&files);
    let files = files
        .into_iter()
        .map(|relative| {
            let inside = match &top {
                Some(top) => relative.strip_prefix(top).map_err(|e| e.to_string())?.to_path_buf(),
                None => relative.clone(),
            };
            Ok((relative, inside))
        })
        .collect::<Result<Vec<_>, String>>()?;
    if !files.iter().any(|(_, inside)| inside == Path::new(executable)) {
        return Err(format!("the update doesn't contain {}", executable.to_string_lossy()));
    }
    for (relative, _) in &files {
        check(&unpacked.join(relative))?;
    }
    let mut saved = Vec::new();
    let mut created = Vec::new();
    let copied = (|| {
        for (relative, inside) in &files {
            let target = folder.join(inside);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            if target.is_file() {
                let previous = backup.join(inside);
                std::fs::create_dir_all(previous.parent().ok_or("no backup folder")?).map_err(|e| e.to_string())?;
                std::fs::copy(&target, &previous).map_err(|e| e.to_string())?;
                saved.push(inside.to_path_buf());
            } else {
                created.push(target.clone());
            }
            std::fs::copy(unpacked.join(relative), &target).map_err(|e| format!("couldn't replace {} ({e})", target.display()))?;
        }
        // Keep the previous files until the installed copies have passed the same checks.
        for (_, inside) in &files {
            check(&folder.join(inside))?;
        }
        Ok::<(), String>(())
    })();
    if let Err(error) = copied {
        for target in &created {
            let _ = std::fs::remove_file(target);
        }
        for inside in &saved {
            if let Err(restore) = std::fs::copy(backup.join(inside), folder.join(inside)) {
                let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
                let mut recovery = staging.as_os_str().to_owned();
                recovery.push(format!("-recovery-{stamp}"));
                let recovery = PathBuf::from(recovery);
                let kept = if std::fs::rename(&backup, &recovery).is_ok() {
                    recovery
                } else {
                    *keep = true;
                    backup.clone()
                };
                return Err(format!(
                    "{error}; couldn't put back {} ({restore}). The previous files are in {}",
                    folder.join(inside).display(),
                    kept.display()
                ));
            }
        }
        return Err(error);
    }
    Ok(())
}

/// The one folder that holds every file, when there is one.
fn single_top_folder(files: &[PathBuf]) -> Option<PathBuf> {
    let first = files.first()?.components().next()?;
    files
        .iter()
        .all(|file| {
            let mut parts = file.components();
            parts.next() == Some(first) && parts.next().is_some()
        })
        .then(|| PathBuf::from(first.as_os_str()))
}

#[cfg(test)]
mod tests;
