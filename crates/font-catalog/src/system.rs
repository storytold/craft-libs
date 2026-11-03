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
    use std::collections::HashSet;
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
                load_store(&mut collection, &store);
            }
            collection
        })
        .ok()
    }

    /// Adds the font store `dir`'s fonts ([`store_files`]) to `collection`, one file at a time:
    /// fontique 0.11 adds every font gathered in one `load_fonts_from_paths` call to its family
    /// again for each face it registers, so one call over the whole store repeats its fonts
    /// quadratically in the number of files.
    fn load_store(collection: &mut Collection, dir: &Path) {
        for file in store_files(dir) {
            collection.load_fonts_from_paths([file]);
        }
    }

    /// Most faces a font-store collection may have to be handed to fontique. fontique 0.11
    /// registers a collection file once for every face in it, each time scanning all its faces
    /// again and rebuilding the family's font list, so the work grows with the fourth power of
    /// the face count. Larger collections are left to the catalog's own scan, which reads each
    /// face once.
    const MAX_COLLECTION_FACES: u32 = 64;
    /// Most font-store files handed to fontique.
    const MAX_STORE_FILES: usize = 20_000;
    /// Most font-store folder entries looked at, as the catalog's scan bounds its own walk.
    const MAX_STORE_ENTRIES: usize = 200_000;
    /// How deep the store's folders are walked: fontique's own depth for a folder it is given.
    const MAX_STORE_DEPTH: u32 = 16;

    /// The regular files under the font store `dir` that fontique may import, in the order
    /// fontique walks a folder: at most [`MAX_STORE_FILES`] of the first [`MAX_STORE_ENTRIES`]
    /// entries, no deeper than [`MAX_STORE_DEPTH`], without collections of more than
    /// [`MAX_COLLECTION_FACES`] faces and without pipes, devices or other special files. Links
    /// are followed, as fontique and the catalog's scan follow them; a folder reached again
    /// through a link is walked once.
    fn store_files(dir: &Path) -> Vec<PathBuf> {
        let mut walk = StoreWalk { files: Vec::new(), entries: 0, seen_dirs: HashSet::new() };
        walk.dir(dir, 0);
        walk.files
    }

    struct StoreWalk {
        files: Vec<PathBuf>,
        entries: usize,
        seen_dirs: HashSet<PathBuf>,
    }

    impl StoreWalk {
        fn dir(&mut self, dir: &Path, depth: u32) {
            if !self.seen_dirs.insert(std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf())) {
                return;
            }
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for entry in rd.flatten() {
                self.entries += 1;
                if self.entries > MAX_STORE_ENTRIES || self.files.len() >= MAX_STORE_FILES {
                    return;
                }
                let path = entry.path();
                let Ok(meta) = std::fs::metadata(&path) else { continue };
                if meta.is_dir() {
                    if depth < MAX_STORE_DEPTH {
                        self.dir(&path, depth + 1);
                    }
                } else if meta.is_file() && collection_faces(&path) <= MAX_COLLECTION_FACES {
                    self.files.push(path);
                }
            }
        }
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
        fn store_files_leave_out_large_collections_and_deep_folders_and_follow_links() {
            let dir = std::env::temp_dir().join(format!("craft-font-catalog-store-files-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let family = dir.join("source/family");
            std::fs::create_dir_all(&family).unwrap();
            let deepest = (0..MAX_STORE_DEPTH).fold(dir.clone(), |d, i| d.join(i.to_string()));
            std::fs::create_dir_all(deepest.join("too-deep")).unwrap();
            let ttc = |faces: u32| [b"ttcf".as_slice(), &0x0001_0000u32.to_be_bytes(), &faces.to_be_bytes()].concat();
            std::fs::write(family.join("Face.ttf"), b"\0\x01\0\0 a font").unwrap();
            std::fs::write(family.join("Small.ttc"), ttc(4)).unwrap();
            std::fs::write(family.join("Limit.ttc"), ttc(MAX_COLLECTION_FACES)).unwrap();
            std::fs::write(family.join("Huge.ttc"), ttc(100_000)).unwrap();
            std::fs::write(family.join("tiny"), b"x").unwrap();
            std::fs::write(deepest.join("Deepest.ttf"), b"font").unwrap();
            std::fs::write(deepest.join("too-deep/Deep.ttf"), b"font").unwrap();
            let mut expected =
                vec![family.join("Face.ttf"), family.join("Small.ttc"), family.join("Limit.ttc"), family.join("tiny"), deepest.join("Deepest.ttf")];
            #[cfg(unix)]
            {
                let other = dir.with_extension("elsewhere");
                let _ = std::fs::remove_dir_all(&other);
                std::fs::create_dir_all(&other).unwrap();
                std::fs::write(other.join("Linked.ttf"), b"font").unwrap();
                std::os::unix::fs::symlink(other.join("Linked.ttf"), family.join("Link.ttf")).unwrap();
                std::os::unix::fs::symlink(&other, dir.join("linked-folder")).unwrap();
                std::os::unix::fs::symlink(other.join("Missing.ttf"), family.join("Dangling.ttf")).unwrap();
                std::os::unix::fs::symlink(&dir, family.join("loop")).unwrap();
                expected.extend([family.join("Link.ttf"), dir.join("linked-folder/Linked.ttf")]);
            }
            let mut files = store_files(&dir);
            files.sort();
            expected.sort();
            assert_eq!(files, expected);
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(dir.with_extension("elsewhere"));
        }

        #[test]
        fn store_fonts_join_their_family_once() {
            use write_fonts::FontBuilder;
            use write_fonts::tables::cmap::Cmap;
            use write_fonts::tables::name::{Name, NameRecord};
            use write_fonts::tables::os2::Os2;
            use write_fonts::types::{GlyphId, NameId};
            let font = |style: &str, weight: u16| {
                let records = [(1, "Craft Store Sans".to_string()), (2, style.to_string()), (6, format!("CraftStoreSans-{style}"))]
                    .map(|(id, text)| NameRecord::new(3, 1, 0x409, NameId::new(id), text.into()));
                let mut builder = FontBuilder::new();
                builder.add_table(&Name::new(records.into())).unwrap();
                builder.add_table(&Os2 { us_weight_class: weight, us_width_class: 5, ..Default::default() }).unwrap();
                builder.add_table(&Cmap::from_mappings([('a', GlyphId::new(1))]).unwrap()).unwrap();
                builder.build()
            };
            let dir = std::env::temp_dir().join(format!("craft-font-catalog-load-store-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let family = dir.join("source/store-sans");
            std::fs::create_dir_all(&family).unwrap();
            for (style, weight) in [("Regular", 400), ("Bold", 700), ("Light", 300)] {
                std::fs::write(family.join(format!("StoreSans-{style}.ttf")), font(style, weight)).unwrap();
            }
            let mut collection = Collection::new(CollectionOptions { shared: false, system_fonts: false });
            load_store(&mut collection, &dir);
            let fonts = collection.family_by_name("Craft Store Sans").unwrap().fonts().len();
            assert_eq!(fonts, 3);
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
