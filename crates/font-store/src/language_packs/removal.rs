//! Removing Language packs: the pack folder moves into a recovery stage under
//! `language-packs/.removing/`, the record commits the removal with a receipt for that stage, and
//! the stage is deleted then or by a later locked sweep. A failed record write restores the pack.

use super::*;

impl FontStore {
    /// Removes the pack for `locale`: its folder disappears at once (one rename) and is deleted
    /// after. No network needed. Then call `craft_proof::packs::reload()`.
    pub fn remove_language_pack(&self, locale: &str) -> Result<(), Error> {
        if !is_locale(locale) {
            return Err(Error::NotInstalled(locale.chars().take(40).collect()));
        }
        let _lock = self.lock()?;
        let dest = self.language_packs_dir().join(locale);
        let mut installed = self.read_installed()?;
        let known = installed.packs.iter().any(|p| p.locale == locale);
        let has_folder = std::fs::symlink_metadata(&dest).is_ok();
        if !known && !has_folder {
            return Err(Error::NotInstalled(locale.to_string()));
        }
        if !has_folder {
            installed.packs.retain(|pack| pack.locale != locale);
            return self.write_installed(&installed);
        }
        let stage = self.language_pack_removal_stage(&installed.committed_removals)?;
        let backup = stage.dir.join(locale);
        if has_folder {
            let id = stage
                .dir
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| Stage::valid_name(name))
                .ok_or_else(|| io("recording a Language pack removal")(std::io::Error::other("invalid backup name")))?
                .to_string();
            #[cfg(test)]
            {
                tests::before_pack_move(&stage.dir).map_err(io(format!("preparing the {locale} pack removal")))?;
                tests::fail_io(tests::IoFailure::Rename).map_err(io(format!("removing the {locale} pack")))?;
            }
            stage.recovery_root().and_then(|root| root.move_pack(locale)).map_err(io(format!("removing the {locale} pack")))?;
            installed.committed_removals.push(id);
        }
        installed.packs.retain(|p| p.locale != locale);
        let saved = stage
            .recovery_root()
            .map_err(io("using the Language pack recovery stage"))
            .and_then(|root| self.write_installed_with_recovery(&installed, Some(root)));
        finish_pack_removal(stage, &backup, &dest, has_folder, saved)
    }

    /// Retain the move, rollback, and cleanup capabilities; remove the empty stage if opening fails.
    pub(super) fn language_pack_removal_stage(&self, excluded: &[String]) -> Result<Stage, Error> {
        // A failed rollback or interrupted removal must not lose its backup to the download
        // sweep. Locale scans ignore this hidden directory.
        let parent = self.pack_removal_dir()?;
        std::fs::create_dir_all(&parent).map_err(io("creating a temporary folder"))?;
        // Open the anchor before creating the stage, so a failure leaves no stage behind, and
        // create, use and remove the stage through it. Ordinary download stages keep their
        // existing policy.
        let root = PackRemovalRoot::open(self.dir()).map_err(io("opening the Language pack recovery folder"))?;
        Stage::recovery_in(&parent, excluded, root)
    }

    /// Reject redirected or non-directory recovery ancestors before staging a removal.
    pub(super) fn pack_removal_dir(&self) -> Result<PathBuf, Error> {
        let packs = self.language_packs_dir();
        match std::fs::symlink_metadata(&packs) {
            Ok(metadata) if !metadata.is_dir() || metadata.file_type().is_symlink() => {
                return Err(io("using the Language pack folder")(std::io::Error::other("not a plain directory")));
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(io("reading the Language pack folder")(error)),
            _ => {}
        }
        let parent = packs.join(".removing");
        match std::fs::symlink_metadata(&parent) {
            Ok(metadata) if !metadata.is_dir() || metadata.file_type().is_symlink() => {
                return Err(io("using the Language pack recovery folder")(std::io::Error::other("not a plain directory")));
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(io("reading the Language pack recovery folder")(error)),
            _ => {}
        }
        Ok(parent)
    }

    /// Called only after acquiring the store lock: a snapshot must not delete a newly reused ID.
    pub(crate) fn sweep_committed_pack_removals(&self) {
        let Some(installed) = self.read_record() else { return };
        if installed.version != 1 {
            return;
        }
        let Ok(parent) = self.pack_removal_dir() else { return };
        let Ok(root) = PackRemovalRoot::open(self.dir()) else { return };
        let ids = &installed.committed_removals;
        if ids.is_empty() {
            return;
        }
        // All scheduler state is read and updated under the store lock. Complete finite
        // rounds so appended receipts cannot keep old failures waiting behind a growing tail.
        const ATTEMPTS: usize = 4;
        let mut end = self.pack_removal_round_end.load(std::sync::atomic::Ordering::Relaxed).min(ids.len());
        let mut next = self.pack_removal_cursor.load(std::sync::atomic::Ordering::Relaxed);
        if next >= end {
            next = 0;
            end = ids.len();
        }
        for _ in 0..ATTEMPTS.min(ids.len()) {
            if next == end {
                next = 0;
                end = ids.len();
            }
            let id = &ids[next];
            next += 1;
            if let Err(error) = root.remove(id, &parent)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                log::warn!("cannot clean up committed Language pack removal {id}: {error}");
            }
        }
        self.pack_removal_cursor.store(next, std::sync::atomic::Ordering::Relaxed);
        self.pack_removal_round_end.store(end, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Keep recovery ancestors open until cleanup ends. Unix operations use directory-relative
/// handles; Windows directory handles deny deletion and renaming while they remain open.
/// The caller-selected store is the ambient authority boundary, not an untrusted child path.
pub(crate) struct PackRemovalRoot {
    _store: std::fs::File,
    packs: std::fs::File,
    recovery: std::fs::File,
    stage: Option<std::fs::File>,
}

impl PackRemovalRoot {
    /// Open each named child without following a symlink; preserve every ancestor handle.
    pub(super) fn open(store: &Path) -> std::io::Result<Self> {
        use cap_primitives::fs::{open_ambient_dir, open_dir_nofollow};
        let root = open_ambient_dir(store, cap_primitives::ambient_authority())?;
        let packs = open_dir_nofollow(&root, Path::new(LANGUAGE_PACKS))?;
        let recovery = open_dir_nofollow(&packs, Path::new(".removing"))?;
        Ok(Self { _store: root, packs, recovery, stage: None })
    }

    /// Create a stage folder in the retained recovery directory.
    pub(crate) fn create_stage(&self, id: &str) -> std::io::Result<()> {
        cap_primitives::fs::create_dir(&self.recovery, Path::new(id), &cap_primitives::fs::DirOptions::new())
    }

    /// Pin the new stage itself so pack moves and rollback share the same destination.
    /// If the stage cannot be opened, remove it through the retained recovery directory. The
    /// removal is not recursive, so it leaves a folder that holds anything.
    pub(crate) fn with_stage(mut self, id: &str) -> std::io::Result<Self> {
        let open = || {
            #[cfg(test)]
            tests::fail_io(tests::IoFailure::OpenStage)?;
            cap_primitives::fs::open_dir_nofollow(&self.recovery, Path::new(id))
        };
        match open() {
            Ok(stage) => {
                self.stage = Some(stage);
                Ok(self)
            }
            Err(error) => {
                let _ = cap_primitives::fs::remove_dir(&self.recovery, Path::new(id));
                Err(error)
            }
        }
    }

    /// Move the locale folder into the retained stage without resolving its display path.
    fn move_pack(&self, locale: &str) -> std::io::Result<()> {
        let stage = self.stage.as_ref().ok_or_else(|| std::io::Error::other("missing recovery stage"))?;
        cap_primitives::fs::rename(&self.packs, Path::new(locale), stage, Path::new(locale))
    }

    /// Restore to the original pack parent through the same handles used by the move.
    fn restore_pack(&self, locale: &str) -> std::io::Result<()> {
        let stage = self.stage.as_ref().ok_or_else(|| std::io::Error::other("missing recovery stage"))?;
        cap_primitives::fs::rename(stage, Path::new(locale), &self.packs, Path::new(locale))
    }

    /// Absence must be observed relative to this directory, never through its display path.
    pub(super) fn retains_receipt(&self, id: &str) -> bool {
        !matches!(
            cap_primitives::fs::stat(&self.recovery, Path::new(id), cap_primitives::fs::FollowSymlinks::No),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        )
    }

    /// Release the child lock before deletion while retaining every recovery ancestor.
    pub(crate) fn remove_stage(mut self, id: &str, display_parent: &Path) -> std::io::Result<()> {
        drop(self.stage.take());
        self.remove(id, display_parent)
    }

    /// Resolve only a validated receipt relative to the retained recovery directory.
    pub(crate) fn remove(&self, id: &str, _display_parent: &Path) -> std::io::Result<()> {
        #[cfg(test)]
        tests::before_cleanup(&_display_parent.join(id))?;
        cap_primitives::fs::remove_dir_all(&self.recovery, Path::new(id))
    }
}

/// Restore through the retained stage on a failed commit; retain the backup if rollback fails.
pub(super) fn finish_pack_removal(stage: Stage, backup: &Path, dest: &Path, moved: bool, saved: Result<(), Error>) -> Result<(), Error> {
    if let Err(error) = saved {
        let restore = || {
            let locale = dest.file_name().and_then(|name| name.to_str()).ok_or_else(|| std::io::Error::other("invalid pack name"))?;
            stage.recovery_root()?.restore_pack(locale)
        };
        if moved && let Err(restore) = restore() {
            let id = stage.dir.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_owned();
            stage.keep_for_recovery();
            return Err(io(format!(
                "saving the Language pack list failed ({error}); the original pack is preserved in recovery stage {id}, last known at {}",
                backup.display()
            ))(restore));
        }
        return Err(error);
    }
    // The record committed the removal; dropping the stage cleans up its old files.
    Ok(())
}
