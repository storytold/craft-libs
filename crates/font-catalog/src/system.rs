//! The platform's font service through fontique: CoreText's available fonts on macOS (fonts that
//! Adobe Fonts, RightFont and other managers activate from their own folders included),
//! DirectWrite's system font collection on Windows (per-user fonts and font-service fonts
//! included), and fontconfig on Linux and FreeBSD (Flatpak's host fonts included).
//!
//! One fontique collection serves the whole process: the catalog lists its files, and the UI's
//! font fallback ([`collection`]) queries the same collection, so the system's fonts are
//! enumerated once.

#[cfg(all(feature = "system", not(target_arch = "wasm32")))]
mod imp {
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, PoisonError};

    use fontique::{Collection, CollectionOptions, SourceKind};

    use crate::{FontEnv, font_store_dir};

    struct Shared {
        generation: u64,
        collection: Collection,
    }

    static SHARED: Mutex<Option<Shared>> = Mutex::new(None);

    /// A new collection of the system's fonts and the font store's. `None` when fontique panics
    /// (its DirectWrite backend unwraps a failed factory).
    fn build() -> Option<Collection> {
        std::panic::catch_unwind(|| {
            let mut collection = Collection::new(CollectionOptions { shared: false, system_fonts: true });
            if let Some(store) = font_store_dir(&FontEnv::from_process()).filter(|d| d.is_dir()) {
                let files = store_files(&store);
                if !files.is_empty() {
                    collection.load_fonts_from_paths(files);
                }
            }
            collection
        })
        .ok()
    }

    /// Most faces a font-store collection may have to be handed to fontique. fontique 0.11
    /// registers a collection file once for every face in it, each time scanning all its faces
    /// again and rebuilding the family's font list, so the work grows with the fourth power of
    /// the face count. Larger collections are left to the catalog's own scan, which reads each
    /// face once.
    const MAX_COLLECTION_FACES: u32 = 64;
    /// Most font-store files handed to fontique, and how deep its folders are walked.
    const MAX_STORE_FILES: usize = 20_000;
    const MAX_STORE_DEPTH: u32 = 4;

    /// The regular files under the font store `dir` that fontique may import: at most
    /// [`MAX_STORE_FILES`], no deeper than [`MAX_STORE_DEPTH`], without symbolic links, and
    /// without collections of more than [`MAX_COLLECTION_FACES`] faces.
    fn store_files(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut folders = vec![(dir.to_path_buf(), 0u32)];
        while let Some((folder, depth)) = folders.pop() {
            let Ok(rd) = std::fs::read_dir(&folder) else { continue };
            for entry in rd.flatten() {
                if out.len() >= MAX_STORE_FILES {
                    return out;
                }
                let Ok(kind) = entry.file_type() else { continue };
                let path = entry.path();
                if kind.is_dir() {
                    if depth < MAX_STORE_DEPTH {
                        folders.push((path, depth + 1));
                    }
                } else if kind.is_file() && collection_faces(&path) <= MAX_COLLECTION_FACES {
                    out.push(path);
                }
            }
        }
        out
    }

    /// The face count a collection's header declares; 1 for any other file.
    fn collection_faces(path: &Path) -> u32 {
        use std::io::Read;
        let mut head = [0u8; 12];
        match std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)) {
            Ok(()) if head.starts_with(b"ttcf") => u32::from_be_bytes([head[8], head[9], head[10], head[11]]),
            _ => 1,
        }
    }

    fn with<R>(refresh: bool, f: impl FnOnce(&mut Shared) -> R) -> R {
        let mut guard = SHARED.lock().unwrap_or_else(PoisonError::into_inner);
        let generation = guard.as_ref().map_or(0, |s| s.generation);
        let shared = match &mut *guard {
            Some(shared) if !refresh => shared,
            slot => {
                let collection = build().unwrap_or_else(|| Collection::new(CollectionOptions { shared: false, system_fonts: false }));
                slot.insert(Shared { generation: generation + 1, collection })
            }
        };
        f(shared)
    }

    pub fn collection() -> Collection {
        with(false, |s| s.collection.clone())
    }

    pub fn generation() -> u64 {
        SHARED.lock().unwrap_or_else(PoisonError::into_inner).as_ref().map_or(0, |s| s.generation)
    }

    pub fn refresh() {
        with(true, |_| ());
    }

    pub fn font_files(refresh: bool) -> Vec<PathBuf> {
        with(refresh, |s| {
            let names: Vec<String> = s.collection.family_names().map(str::to_owned).collect();
            let mut files: Vec<PathBuf> = names
                .iter()
                .filter_map(|name| s.collection.family_by_name(name))
                .flat_map(|family| {
                    family
                        .fonts()
                        .iter()
                        .filter_map(|font| match font.source().kind() {
                            SourceKind::Path(path) => Some(path.to_path_buf()),
                            SourceKind::Memory(_) => None,
                        })
                        .collect::<Vec<_>>()
                })
                .collect();
            files.sort();
            files.dedup();
            files
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn store_files_leave_out_large_collections_links_and_deep_folders() {
            let dir = std::env::temp_dir().join(format!("craft-font-catalog-store-files-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let family = dir.join("source/family");
            std::fs::create_dir_all(&family).unwrap();
            std::fs::create_dir_all(dir.join("a/b/c/d/e")).unwrap();
            let ttc = |faces: u32| [b"ttcf".as_slice(), &0x0001_0000u32.to_be_bytes(), &faces.to_be_bytes()].concat();
            std::fs::write(family.join("Face.ttf"), b"\0\x01\0\0 a font").unwrap();
            std::fs::write(family.join("Small.ttc"), ttc(4)).unwrap();
            std::fs::write(family.join("Huge.ttc"), ttc(100_000)).unwrap();
            std::fs::write(family.join("tiny"), b"x").unwrap();
            std::fs::write(dir.join("a/b/c/d/e/Deep.ttf"), b"font").unwrap();
            #[cfg(unix)]
            std::os::unix::fs::symlink(family.join("Face.ttf"), family.join("Link.ttf")).unwrap();
            let mut files = store_files(&dir);
            files.sort();
            assert_eq!(files, vec![family.join("Face.ttf"), family.join("Small.ttc"), family.join("tiny")]);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

#[cfg(not(all(feature = "system", not(target_arch = "wasm32"))))]
mod imp {
    use std::path::PathBuf;

    pub fn font_files(_refresh: bool) -> Vec<PathBuf> {
        Vec::new()
    }
}

#[cfg(all(feature = "system", not(target_arch = "wasm32")))]
/// The process's fontique collection of the system's fonts and the font store's, built on the
/// first call (which waits for the enumeration; later calls are cheap clones that share it).
/// Clones taken before a rescan keep the fonts as they were: compare [`generation`] to know when
/// to take a new one.
pub fn collection() -> fontique::Collection {
    imp::collection()
}

#[cfg(all(feature = "system", not(target_arch = "wasm32")))]
/// Counts the builds of [`collection`]: 0 before the first, one more after each rescan that asked
/// the font service again.
pub fn generation() -> u64 {
    imp::generation()
}

#[cfg(all(feature = "system", not(target_arch = "wasm32")))]
/// Enumerate the system's fonts again (a catalog's [`crate::Catalog::rescan`] does this).
pub fn refresh() {
    imp::refresh();
}

/// The font files the platform's font service lists, enumerating again first with `refresh`.
pub(crate) fn font_files(refresh: bool) -> Vec<std::path::PathBuf> {
    imp::font_files(refresh)
}
