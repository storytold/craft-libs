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
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};

    use fontique::{Collection, CollectionOptions, SourceKind};

    use crate::{FontEnv, font_store_dir};

    struct Shared {
        generation: u64,
        collection: Collection,
    }

    static SHARED: Mutex<Option<Shared>> = Mutex::new(None);
    /// `SHARED`'s generation, readable while a build holds the lock (a UI thread checks it every
    /// frame). Stored before the lock is released, so a [`collection`] taken after reading it is
    /// at least that generation.
    static GENERATION: AtomicU64 = AtomicU64::new(0);
    static ON_CHANGE: Mutex<Vec<Arc<dyn Fn() + Send + Sync>>> = Mutex::new(Vec::new());

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
        let built = refresh || guard.is_none();
        let shared = match &mut *guard {
            Some(shared) if !refresh => shared,
            slot => {
                let collection = build().unwrap_or_else(|| Collection::new(CollectionOptions { shared: false, system_fonts: false }));
                GENERATION.store(generation + 1, Ordering::Release);
                slot.insert(Shared { generation: generation + 1, collection })
            }
        };
        let result = f(shared);
        drop(guard);
        if built {
            // A clone of the list, so a callback may take the collection or register another.
            let callbacks = ON_CHANGE.lock().unwrap_or_else(PoisonError::into_inner).clone();
            for callback in callbacks {
                callback();
            }
        }
        result
    }

    pub fn on_change(callback: Box<dyn Fn() + Send + Sync>) {
        ON_CHANGE.lock().unwrap_or_else(PoisonError::into_inner).push(Arc::from(callback));
    }

    /// Run `f` while holding the lock a build holds, as a rescan on another thread would.
    #[cfg(test)]
    pub(super) fn while_building<R>(f: impl FnOnce() -> R) -> R {
        let _guard = SHARED.lock().unwrap_or_else(PoisonError::into_inner);
        f()
    }

    pub fn collection() -> Collection {
        with(false, |s| s.collection.clone())
    }

    pub fn generation() -> u64 {
        GENERATION.load(Ordering::Acquire)
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
/// the font service again. It doesn't wait for a build in progress, so a UI thread can check it
/// every frame.
pub fn generation() -> u64 {
    imp::generation()
}

#[cfg(all(feature = "system", not(target_arch = "wasm32")))]
/// Call `callback` after every build of [`collection`], once [`generation`] has moved on: the first
/// build and every rescan. It runs on the thread that built, after the build's lock is released,
/// so it may take the new [`collection`]. Callbacks stay registered for the life of the process.
pub fn on_change(callback: Box<dyn Fn() + Send + Sync>) {
    imp::on_change(callback);
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

#[cfg(all(test, feature = "system", not(target_arch = "wasm32")))]
mod tests {
    use std::time::Duration;

    #[test]
    fn generation_does_not_wait_for_a_rescan() {
        let read = super::imp::while_building(|| {
            let (sent, received) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = sent.send(super::generation());
            });
            received.recv_timeout(Duration::from_secs(10))
        });
        assert!(read.is_ok(), "generation() returns while a build holds the collection's lock");
    }
}
