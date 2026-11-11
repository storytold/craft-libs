//! Faces made from a catalog's fonts, for an app that shapes and draws with its own face objects
//! (skrifa, harfrust): each face of a font file is made once, kept while the catalog lists its
//! file unchanged, and let go when a rescan finds the file changed or removed, or when the scope
//! or the added data it came from is removed.
//!
//! Nothing here knows the app's face type `F`.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::scan::{FileStamp, file_stamp};
use crate::{Catalog, FaceInfo, FontData, FontFileError};

/// Which face of which font data a cached face was made from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FaceKey {
    /// A face of a font file: the file and the face's number in it ([`FaceInfo::index`]).
    File(PathBuf, u32),
    /// A face of font data added to the catalog as bytes: the catalog's number for the data,
    /// never given to other data, and the face's number in it.
    Added(u64, u32),
}

impl FaceKey {
    fn of(info: &FaceInfo) -> Option<Self> {
        match (info.blob, &info.path) {
            (Some(blob), _) => Some(Self::Added(blob, info.index)),
            (None, Some(path)) => Some(Self::File(path.clone(), info.index)),
            (None, None) => None,
        }
    }
}

struct Cached<F> {
    face: Arc<F>,
    /// The file's stamp in the listing the face was made for ([`FaceInfo::file`]); `None` for
    /// added data.
    listed: Option<FileStamp>,
    /// The stamp of the file its bytes were read from.
    read: Option<FileStamp>,
    /// A confined folder's file, read without following links.
    confined: bool,
}

impl<F> Cached<F> {
    /// Whether this is the face for the file as a listing with `stamp` lists it.
    fn made_for(&self, stamp: Option<FileStamp>) -> bool {
        self.listed == stamp || (stamp.is_some() && self.read == stamp)
    }
}

struct State<F> {
    faces: HashMap<FaceKey, Cached<F>>,
    /// Counts the faces [`FaceCache::get_or_make`] added or replaced.
    made: u64,
    /// The catalog's [`Catalog::face_changes`] and [`State::made`] at the last full check.
    checked: Option<((u64, u64), u64)>,
    /// Faces [`FaceCache::get_or_make`] made again because the catalog lists their file changed,
    /// for the next [`FaceCache::refresh`] to return.
    replaced: Vec<(FaceKey, Arc<F>)>,
    /// The [`FaceCache::get_or_make`] calls between their load and their insert.
    making: usize,
    /// Counts the [`FaceCache::forget_changed`] calls.
    forgets: u64,
    /// The files [`FaceCache::forget_changed`] was called for while a make ran, with
    /// [`State::forgets`] then; emptied when no make runs.
    forgot: HashMap<PathBuf, u64>,
}

/// The stamp of the file at `path` as a load reads it now (without following a link for a
/// confined file); `None` when that isn't a regular file.
fn stamp_now(path: &Path, confined: bool) -> Option<FileStamp> {
    let meta = if confined { std::fs::symlink_metadata(path) } else { std::fs::metadata(path) };
    meta.ok().filter(std::fs::Metadata::is_file).map(|m| file_stamp(&m))
}

/// The app's faces (`F`) made from a [`Catalog`]'s fonts, by font file and face index, or by
/// added data and face index.
///
/// One cache holds faces of the installed fonts, of fonts added without a scope and of every
/// scope: [`FaceCache::refresh`] keeps a face while any of them lists its file unchanged, so a
/// file that two documents' scopes (or a scope and the installed fonts) share has one face. The
/// key is the file and the face index alone, so an app whose faces differ by where they came
/// from (a document face tagged with its document) keeps one cache for each kind.
///
/// A cache serves one catalog: the numbers of added data in its keys are that catalog's. The app
/// calls [`FaceCache::refresh`] after the catalog changes (a rescan, a document's fonts added or
/// its scope removed), which is also when the faces made again for changed files are let go.
pub struct FaceCache<F> {
    state: Mutex<State<F>>,
}

impl<F> Default for FaceCache<F> {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                faces: HashMap::new(),
                made: 0,
                checked: None,
                replaced: Vec::new(),
                making: 0,
                forgets: 0,
                forgot: HashMap::new(),
            }),
        }
    }
}

/// A [`FaceCache::get_or_make`] between its load and its insert, counted in [`State::making`].
struct Making<'a, F> {
    cache: &'a FaceCache<F>,
    /// [`State::forgets`] before the load.
    forgets: u64,
}

impl<F> Drop for Making<'_, F> {
    fn drop(&mut self) {
        let mut state = self.cache.lock();
        state.making -= 1;
        if state.making == 0 {
            state.forgot.clear();
        }
    }
}

impl<F> FaceCache<F> {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, State<F>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The face made from `info`'s data: the one made before for the same file and face index,
    /// else `make` (called without locks held) from the data `catalog` loads. `make` gets the
    /// data's own index ([`FontData::index`], which differs from [`FaceInfo::index`] for a Mac
    /// suitcase font); `None` from it is [`FontFileError::NotAFont`]. Two threads making the same
    /// face at once both make it, and both get the first one cached.
    ///
    /// When `catalog` lists the file changed since its face was made (a rescan, or the file
    /// added to a scope again), the face is made again, and the next [`FaceCache::refresh`]
    /// returns the old one. An `info` looked up before that change gets the face of the file as
    /// it is now, at the cost of a `stat`: apps resolve their fonts again after
    /// [`Catalog::changes_since`] names them. A face made while [`FaceCache::forget_changed`] is
    /// called for its file is returned but not kept.
    pub fn get_or_make(
        &self,
        catalog: &Catalog,
        info: &FaceInfo,
        make: impl FnOnce(&FaceInfo, &FontData) -> Option<F>,
    ) -> Result<Arc<F>, FontFileError> {
        let key = FaceKey::of(info).ok_or(FontFileError::Gone)?;
        if let Some(face) = self.cached(&key, info) {
            return Ok(face);
        }
        let making = {
            let mut state = self.lock();
            state.making += 1;
            Making { cache: self, forgets: state.forgets }
        };
        // The catalog reads a file replaced since it last read it again.
        let (data, read) = catalog.load_stamped(info)?;
        if let Some(face) = self.cached(&key, info) {
            return Ok(face);
        }
        let face = Arc::new(make(info, &data).ok_or(FontFileError::NotAFont)?);
        let entry = || Cached { face: Arc::clone(&face), listed: info.file, read, confined: info.confined };
        // The file's stamp now, when `info` and a face cached meanwhile disagree about the file.
        let mut now: Option<Option<FileStamp>> = None;
        loop {
            let mut state = self.lock();
            if info.path.as_ref().and_then(|p| state.forgot.get(p)).is_some_and(|&n| n > making.forgets) {
                // The bytes may be from before the change the app was told of.
                return Ok(face);
            }
            let State { faces, replaced, made, .. } = &mut *state;
            let got = match faces.entry(key.clone()) {
                Entry::Vacant(c) => {
                    *made += 1;
                    Some(Arc::clone(&c.insert(entry()).face))
                }
                Entry::Occupied(c) if c.get().made_for(info.file) || (read.is_some() && c.get().read == read) => Some(Arc::clone(&c.get().face)),
                Entry::Occupied(_) if now.is_none() => None,
                // `info` is from an older listing than the face cached meanwhile.
                Entry::Occupied(c) if now.flatten().is_some() && c.get().read == now.flatten() => Some(Arc::clone(&c.get().face)),
                Entry::Occupied(mut c) => {
                    *made += 1;
                    let old = c.insert(entry());
                    replaced.push((key.clone(), old.face));
                    Some(Arc::clone(&face))
                }
            };
            match got {
                Some(face) => return Ok(face),
                None => {
                    drop(state);
                    now = Some(info.path.as_deref().and_then(|p| stamp_now(p, info.confined)));
                }
            }
        }
    }

    /// The cached face for `key` when it was made for the file `info` lists, or from the file as
    /// it is now (`info` came from a listing older than the face's).
    fn cached(&self, key: &FaceKey, info: &FaceInfo) -> Option<Arc<F>> {
        let (read, confined) = {
            let state = self.lock();
            let c = state.faces.get(key)?;
            if c.made_for(info.file) {
                return Some(Arc::clone(&c.face));
            }
            (c.read?, c.confined)
        };
        if stamp_now(info.path.as_deref()?, confined) != Some(read) {
            return None;
        }
        self.lock().faces.get(key).filter(|c| c.read == Some(read)).map(|c| Arc::clone(&c.face))
    }

    /// Let go of the faces whose file `catalog` no longer lists, or lists changed, and of those
    /// whose added data or scope was removed, and return them with the faces
    /// [`FaceCache::get_or_make`] made again (for the app to drop what it made from them). A
    /// face stays while the installed fonts, the fonts added without a scope or any scope list
    /// its file as it was. Nothing to do while `catalog` is unchanged (nothing added or removed,
    /// no rescan changed it) and no face was made since the last refresh; it never waits for a
    /// scan and reads no files.
    pub fn refresh(&self, catalog: &Catalog) -> Vec<(FaceKey, Arc<F>)> {
        type Judged<F> = (FaceKey, Option<FileStamp>, Option<FileStamp>, Arc<F>);
        let changes = catalog.face_changes();
        let (judged, made): (Vec<Judged<F>>, u64) = {
            let mut state = self.lock();
            if state.checked == Some((changes, state.made)) {
                return std::mem::take(&mut state.replaced);
            }
            (state.faces.iter().map(|(k, c)| (k.clone(), c.listed, c.read, Arc::clone(&c.face))).collect(), state.made)
        };
        let mut by_path: HashMap<&Path, Vec<usize>> = HashMap::new();
        let mut by_blob: HashMap<u64, Vec<usize>> = HashMap::new();
        for (n, (key, ..)) in judged.iter().enumerate() {
            match key {
                FaceKey::File(path, _) => by_path.entry(path).or_default().push(n),
                FaceKey::Added(blob, _) => by_blob.entry(*blob).or_default().push(n),
            }
        }
        let mut keep = vec![false; judged.len()];
        let index = |key: &FaceKey| match key {
            FaceKey::File(_, i) | FaceKey::Added(_, i) => *i,
        };
        let changes = catalog.visit_faces(|info| {
            let found = match (info.blob, &info.path) {
                (Some(blob), _) => by_blob.get(&blob),
                (None, Some(path)) => by_path.get(path.as_path()),
                (None, None) => None,
            };
            for &n in found.into_iter().flatten() {
                let (key, listed, read, _) = &judged[n];
                let made_for = *listed == info.file || (info.file.is_some() && *read == info.file);
                if index(key) == info.index && (info.blob.is_some() || made_for) {
                    keep[n] = true;
                }
            }
        });
        let mut state = self.lock();
        // A face made after the snapshot moved `made`: the next refresh checks it.
        state.checked = Some((changes, made));
        let mut gone = std::mem::take(&mut state.replaced);
        for ((key, _, _, face), keep) in judged.into_iter().zip(keep) {
            // A face made again meanwhile is checked next time.
            if !keep && state.faces.get(&key).is_some_and(|c| Arc::ptr_eq(&c.face, &face)) {
                state.faces.remove(&key);
                gone.push((key, face));
            }
        }
        gone
    }

    /// Let go of the faces made from the font file at `path` if it changed (its length or
    /// modification time) since their bytes were read, and return them: the next
    /// [`FaceCache::get_or_make`] reads the file as it is now. For an app that learns of a
    /// change from its own file watcher, before the catalog lists the file again. A face being
    /// made from the file meanwhile isn't kept.
    pub fn forget_changed(&self, path: &Path) -> Vec<(FaceKey, Arc<F>)> {
        let now = [stamp_now(path, false), stamp_now(path, true)];
        let mut state = self.lock();
        state.forgets += 1;
        if state.making > 0 {
            let n = state.forgets;
            state.forgot.insert(path.to_path_buf(), n);
        }
        let changed: Vec<FaceKey> = state
            .faces
            .iter()
            .filter(|(k, c)| matches!(k, FaceKey::File(p, _) if p == path) && c.read != now[usize::from(c.confined)])
            .map(|(k, _)| k.clone())
            .collect();
        changed.into_iter().filter_map(|key| state.faces.remove(&key).map(|c| (key, c.face))).collect()
    }
}
