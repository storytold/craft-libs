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
    use std::path::PathBuf;
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
                collection.load_fonts_from_paths([store]);
            }
            collection
        })
        .ok()
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
