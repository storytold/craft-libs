//! FreeBSD installs: the release `/usr/local` tree.

use std::ffi::OsStr;

use super::{App, Path, PathBuf, Paths, pkexec, process, run_as_root, tree};

/// Copies the release's tree into `/usr/local` (asking for permission through polkit), or
/// into `~/.local` when polkit isn't installed. Returns the binary, every file installed,
/// and whether removing them needs root.
pub fn install_tree(app: &App, file: &Path, paths: &Paths) -> Result<(PathBuf, Vec<PathBuf>, bool), String> {
    let staging = paths.cache.join(format!("{}.new", app.id));
    let result = install_from(app, file, &staging);
    let _ = std::fs::remove_dir_all(&staging);
    result
}

fn install_from(app: &App, file: &Path, staging: &Path) -> Result<(PathBuf, Vec<PathBuf>, bool), String> {
    let tree = tree::unpack(app, file, staging)?;
    let rel = tree::files(&tree)?;
    let mut source = tree.into_os_string();
    source.push("/.");
    if pkexec().is_some() {
        let prefix = PathBuf::from("/usr/local");
        run_as_root(&[OsStr::new("/bin/cp"), OsStr::new("-R"), &source, prefix.as_os_str()])?;
        return Ok((prefix.join("bin").join(app.id), rel.iter().map(|r| prefix.join(r)).collect(), true));
    }
    let prefix = crate::folders::home_dir().ok_or("no home folder")?.join(".local");
    std::fs::create_dir_all(&prefix).map_err(|e| e.to_string())?;
    let status = process::command("/bin/cp").arg("-R").arg(&source).arg(&prefix).status().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("couldn't copy the app into ~/.local".into());
    }
    // ~/.local/bin is often not on PATH for the desktop: point the menu entry at the binary.
    let bin = prefix.join("bin").join(app.id);
    let desktop = prefix.join("share/applications").join(format!("{}.desktop", app.bundle_id()));
    if let Ok(text) = std::fs::read_to_string(&desktop) {
        let exec = format!("Exec=\"{}\"", bin.display());
        let fixed: String = text
            .lines()
            .map(|l| {
                if let Some(rest) = l.strip_prefix("Exec=").and_then(|r| r.strip_prefix(app.id)) {
                    format!("{exec}{rest}\n")
                } else {
                    format!("{l}\n")
                }
            })
            .collect();
        let _ = std::fs::write(&desktop, fixed);
    }
    Ok((bin, rel.iter().map(|r| prefix.join(r)).collect(), false))
}

/// Removes the files a tree install put down.
pub fn remove_files(files: &[PathBuf], needs_root: bool) -> Result<(), String> {
    if files.is_empty() {
        return Err("the launcher has no record of these files".into());
    }
    let prefix = if needs_root { PathBuf::from("/usr/local") } else { crate::folders::home_dir().ok_or("no home folder")?.join(".local") };
    let mut safe_files = Vec::new();
    for f in files {
        // Refuse paths that climb or escape install prefix (CWE-59).
        if !f.starts_with(&prefix) || f.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err(format!("refusing to remove file outside install prefix: {}", f.display()));
        }
        let mut has_symlink_ancestor = false;
        if let Ok(rel) = f.strip_prefix(&prefix) {
            let mut check = prefix.clone();
            for part in rel.components() {
                if let Ok(meta) = std::fs::symlink_metadata(&check)
                    && meta.file_type().is_symlink()
                {
                    has_symlink_ancestor = true;
                    break;
                }
                check.push(part);
            }
        }
        if !has_symlink_ancestor {
            safe_files.push(f);
        }
    }
    if safe_files.is_empty() {
        return Ok(());
    }
    if needs_root {
        let mut args: Vec<&OsStr> = vec![OsStr::new("/bin/rm"), OsStr::new("-f"), OsStr::new("--")];
        args.extend(safe_files.iter().map(|f| f.as_os_str()));
        return run_as_root(&args);
    }
    for f in safe_files {
        let _ = std::fs::remove_file(f);
    }
    Ok(())
}
