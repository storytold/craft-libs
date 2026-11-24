//! Faces drawn as families of their own, for text set in a document's fonts.
//!
//! One registry per context serves every caller, so the faces of a frame are counted together. A face is drawn alone
//! ([`DocumentFaces::face`]: a font sample shows what the face itself has) or over the interface fonts
//! ([`DocumentFaces::over_interface`]: document text such as an input method's composition, whose characters the face
//! lacks come from the interface stack, CJK included).
//!
//! A face drawn alone is read from a font catalog: its `load` runs as a job on the context's queue of face loads, and
//! the face is ready from the pass after it lands. A face over the interface fonts comes from bytes the app holds: its
//! `load` runs at once and the face is ready in the same pass, so document text never draws a frame without it. Either
//! way the face reaches egui as a served family (`egui_fonts::serve_face`), which egui parses the first time text is
//! laid out in it: a face arriving drops nothing egui has parsed or laid out.
//!
//! The registry keeps faces up to [`MAX_DOCUMENT_FACE_BYTES`] of font files, counted with the files egui still holds
//! for faces taken out; past it the faces drawn longest ago go, and faces drawn in the last frame always stay. A face a
//! settled view shows by replaying its last run counts as drawn: the replay marks it (`runtime::replay::journal`).
//!
//! A face may name a chain of other document faces ([`FaceBytes::chain`]): a font split over several subsets (Word
//! embeds one subset per run of characters) draws each character from the first subset that has it, before the
//! interface family. egui asks for a family's faces once, so a sibling that lands after the face it follows makes
//! egui's fonts again, and drawing a face marks its chain drawn, so the siblings stay while the face does.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use craft_host::Wake;
use craft_host::queue::{JobQueue, Priority, QueueOptions};
use egui::epaint::text::{Blob, FontTweak, VariationCoords};
use egui::{Context, FontData};

use super::FontRef;
use crate::runtime::egui::{fonts as egui_fonts, memory, pass, repaint};
use crate::runtime::env::Env;
use crate::runtime::{frame, replay};

/// The ceiling on the font file bytes of document faces: the files the registry keeps faces of (a collection counted
/// once) plus the files of faces taken out, which egui keeps parsed until its fonts are made again. When the two pass
/// it, the faces drawn longest ago are taken out until the kept files are at most `1 / LOW_MARK_DIVISOR` of it, and
/// egui's fonts are made again once to free what was taken out. The sum passes the ceiling only by the faces drawn in
/// the last frame, which always stay, and by the faces that land in one pass.
pub const MAX_DOCUMENT_FACE_BYTES: usize = 512 << 20;

/// Eviction takes the kept files down to the budget divided by this, so egui's fonts are made again once per half a
/// budget of new faces, not for each face that lands past the budget.
const LOW_MARK_DIVISOR: usize = 2;

/// The bytes of a face, its index in a collection, its variation coordinates (axis tag, value) and the faces that
/// draw what it lacks.
#[derive(Clone)]
#[non_exhaustive]
pub struct FaceBytes {
    /// The font file.
    pub bytes: Arc<[u8]>,
    /// The face's index in the file.
    pub index: u32,
    /// The variation coordinates the face is drawn at.
    pub coords: Vec<([u8; 4], f32)>,
    /// The keys of the document faces that draw the characters this face has no glyph for, in order, before the
    /// interface family.
    pub chain: Vec<String>,
}

impl FaceBytes {
    /// Face `index` of the file `bytes`, at its default coordinates, with no chain.
    pub fn new(bytes: Arc<[u8]>, index: u32) -> Self {
        FaceBytes { bytes, index, coords: Vec::new(), chain: Vec::new() }
    }

    /// The face followed by the document faces of `keys`, in order, for the characters it has no glyph for: the
    /// sibling subsets of a font split over several (Word). Each key is a face asked for with `document_face` on its
    /// own; one not registered, unavailable or taken out is skipped, and the face's own key is ignored. Drawing the
    /// face marks its chain drawn too, so a sibling the face relies on stays while the face does.
    #[must_use]
    pub fn chain(mut self, keys: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.chain = keys.into_iter().map(Into::into).collect();
        self
    }

    /// The family names of the chain's faces (each drawn alone).
    fn chain_families(&self) -> Vec<String> {
        self.chain.iter().map(|key| Kind::Alone.family_name(key)).collect()
    }

    /// The face drawn at `coords`.
    #[must_use]
    pub fn coords(mut self, coords: impl IntoIterator<Item = ([u8; 4], f32)>) -> Self {
        self.coords = coords.into_iter().collect();
        self
    }

    fn data(&self) -> FontData {
        let blob: Blob = Arc::new(Arc::clone(&self.bytes));
        let tweak = FontTweak { coords: VariationCoords::new(self.coords.iter().copied()), ..FontTweak::default() };
        FontData::from_blob(blob, self.index).tweak(tweak)
    }

    /// Whether egui can draw the face: egui stops on a face it cannot read.
    fn parses(&self) -> bool {
        skrifa::FontRef::from_index(&self.bytes, self.index).is_ok()
    }

    /// The file's identity: faces of one collection share their bytes.
    fn file(&self) -> usize {
        Arc::as_ptr(&self.bytes).cast::<u8>() as usize
    }
}

/// What drawing a document face finds.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum FaceState {
    /// The family that draws the face.
    Ready(FontRef),
    /// The face is being loaded; the asking view runs again in the next pass.
    Loading,
    /// The face could not be loaded.
    Unavailable,
}

/// A registered face: its bytes and the frame it was last drawn in.
struct Registered {
    face: FaceBytes,
    used: u64,
}

/// The queue the catalog loads run on, with its lane count.
struct Queue {
    lanes: usize,
    queue: Arc<JobQueue<String, Option<FaceBytes>>>,
}

/// The context's document faces, by family name.
struct Registry {
    faces: HashMap<String, Registered>,
    failed: HashSet<String>,
    /// Loads queued or running, with the frame each was last asked for in.
    loading: HashMap<String, u64>,
    queue: Option<Queue>,
    /// The bytes of files whose faces were taken out since egui's fonts were last made again.
    withdrawn: usize,
    /// [`MAX_DOCUMENT_FACE_BYTES`], or a trial's own (`testing::document_face_budget`).
    budget: usize,
    frame: u64,
    hooked: bool,
}

impl Default for Registry {
    fn default() -> Self {
        Registry {
            faces: HashMap::new(),
            failed: HashSet::new(),
            loading: HashMap::new(),
            queue: None,
            withdrawn: 0,
            budget: MAX_DOCUMENT_FACE_BYTES,
            frame: 0,
            hooked: false,
        }
    }
}

impl Registry {
    /// Marks the kept face `name` and the faces of its chain drawn in this frame.
    fn mark_drawn(&mut self, name: &str) {
        let now = self.frame;
        let Some(face) = self.faces.get_mut(name) else { return };
        face.used = now;
        for link in face.face.chain_families() {
            if let Some(linked) = self.faces.get_mut(&link) {
                linked.used = now;
            }
        }
    }

    /// Whether a face of `landed` is in the chain of a face kept from before: egui may have asked for that face's
    /// family already, without the sibling.
    fn links_late(&self, landed: &[String]) -> bool {
        self.faces
            .iter()
            .filter(|(name, face)| !face.face.chain.is_empty() && !landed.contains(name))
            .any(|(_, face)| face.face.chain_families().iter().any(|link| landed.contains(link)))
    }

    /// When the kept and withdrawn files pass the budget, takes out the faces drawn longest ago (never one the last
    /// frame drew) down to the low mark, and returns their names and whether egui's fonts are made again to free the
    /// withdrawn files.
    fn evict(&mut self, now: u64) -> (Vec<String>, bool) {
        let mut files: HashMap<usize, (usize, usize)> = HashMap::new();
        for face in self.faces.values() {
            files.entry(face.face.file()).or_insert((face.face.bytes.len(), 0)).1 += 1;
        }
        let mut held: usize = files.values().map(|(bytes, _)| bytes).sum();
        if held + self.withdrawn <= self.budget {
            return (Vec::new(), false);
        }
        let low = self.budget / LOW_MARK_DIVISOR;
        let mut stale: Vec<(u64, String)> =
            self.faces.iter().filter(|(_, face)| face.used + 1 < now).map(|(name, face)| (face.used, name.clone())).collect();
        stale.sort();
        let mut gone = Vec::new();
        for (_, name) in stale {
            if held <= low {
                break;
            }
            let Some(face) = self.faces.remove(&name) else { continue };
            if let Some((bytes, count)) = files.get_mut(&face.face.file()) {
                *count -= 1;
                if *count == 0 {
                    held -= *bytes;
                    self.withdrawn += *bytes;
                }
            }
            gone.push(name);
        }
        (gone, std::mem::take(&mut self.withdrawn) > 0)
    }
}

/// How a face's family is built.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// The face alone.
    Alone,
    /// The face, then the proportional family's chain.
    OverInterface,
}

impl Kind {
    /// The family (and face) name of `key`: one per key and kind.
    fn family_name(self, key: &str) -> String {
        match self {
            Kind::Alone => format!("face:{key}"),
            Kind::OverInterface => format!("face+ui:{key}"),
        }
    }
}

/// What the registry knew of a key.
enum Known {
    Failed,
    Ready,
    Loading,
    New,
}

/// The queue of `ctx`'s catalog loads: made on first use on the context's jobs (`Env::jobs_of`) and kept
/// for the context's life, as `TextureLoader::in_context` keeps its own.
fn queue_of(ctx: &Context, registry: &mut Registry) -> Arc<JobQueue<String, Option<FaceBytes>>> {
    if let Some(queue) = &registry.queue {
        return Arc::clone(&queue.queue);
    }
    let jobs = Env::jobs_of(ctx);
    let wake_ctx = ctx.clone();
    let wake = Wake::new(move || repaint::request(&wake_ctx));
    let lanes = QueueOptions::default_lanes();
    let queue = Arc::new(JobQueue::new(jobs, wake, QueueOptions { lanes, name: "craft-document-faces", ..QueueOptions::default() }));
    registry.queue = Some(Queue { lanes, queue: Arc::clone(&queue) });
    queue
}

/// The begin-pass step: drops the loads no frame asked for lately, serves the faces that landed, marks unavailable the
/// faces whose load ended without landing (a job that panicked) and takes out the faces past the budget. Before anything of the pass is laid out, so a rebuild of egui's fonts it asks for applies at once.
fn begin_pass(ctx: &Context) {
    let now = pass::root_frame_nr(ctx);
    let inline = memory::with::<Registry, _>(ctx, |registry| {
        registry.frame = now;
        registry.queue.as_ref().filter(|queue| queue.lanes == 0).map(|queue| Arc::clone(&queue.queue))
    });
    // Without lanes (wasm32) one load runs per pass, here.
    if let Some(queue) = inline {
        queue.run_inline(1);
    }
    let (landed, gone, rebuild) = memory::with::<Registry, _>(ctx, |registry| {
        let mut landed = Vec::new();
        if let Some(queue) = registry.queue.as_ref().map(|queue| Arc::clone(&queue.queue)) {
            let loading = &mut registry.loading;
            loading.retain(|_, asked| *asked + 1 >= now);
            queue.reprioritize(|key, priority| loading.contains_key(key).then_some(priority));
            // A job leaves the running ones and files its result under the queue's one lock, so a key neither queued
            // nor running here is among the results taken after, or its job ended without one.
            let mut lost: Vec<String> = loading.keys().filter(|key| !queue.contains(key)).cloned().collect();
            while let Some(finished) = queue.try_recv() {
                lost.retain(|key| *key != finished.key);
                if loading.remove(&finished.key).is_some() {
                    landed.push((finished.key, finished.result));
                }
            }
            for key in lost {
                loading.remove(&key);
                registry.failed.insert(key);
            }
        }
        for (name, result) in &landed {
            match result {
                Some(face) => {
                    registry.faces.insert(name.clone(), Registered { face: face.clone(), used: now });
                }
                None => {
                    registry.failed.insert(name.clone());
                }
            }
        }
        let served: Vec<String> = landed.iter().filter(|(_, face)| face.is_some()).map(|(name, _)| name.clone()).collect();
        let late = registry.links_late(&served);
        let (gone, rebuild) = registry.evict(now);
        (landed, gone, rebuild || late)
    });
    for (name, face) in landed {
        if let Some(face) = face {
            egui_fonts::serve_face(ctx, &FontRef::named(&name), face.data(), false, face.chain_families());
        }
    }
    if !gone.is_empty() {
        egui_fonts::withdraw_faces(ctx, &gone);
    }
    if rebuild {
        egui_fonts::reload_providers(ctx);
    }
}

/// Marks the kept face `name` drawn in this frame: a replay's journal entry.
fn mark_drawn(ctx: &Context, name: &str) {
    memory::with::<Registry, _>(ctx, |registry| registry.mark_drawn(name));
}

/// Sets the budget of `ctx`'s document faces to `bytes` in place of [`MAX_DOCUMENT_FACE_BYTES`], so a trial reaches
/// it with a few faces.
#[cfg(feature = "testing")]
pub fn set_budget(ctx: &Context, bytes: usize) {
    memory::with::<Registry, _>(ctx, |registry| registry.budget = bytes);
}

/// Hooks the begin-pass step on the context's fonts plugin.
fn begin(ctx: &Context) {
    let hook = memory::with::<Registry, _>(ctx, |registry| !std::mem::replace(&mut registry.hooked, true));
    if hook {
        egui_fonts::on_begin_pass(ctx, begin_pass);
        memory::with::<Registry, _>(ctx, |registry| registry.frame = pass::root_frame_nr(ctx));
    }
}

/// The document faces: the families drawn from font files a document names. Reached through `FacesCx`.
pub(crate) struct DocumentFaces;

impl DocumentFaces {
    /// The face registered under `key` as a family of its own. The load `job()` makes reads it from a font catalog:
    /// `job` is called only for a key the registry does not know, and its load runs as a job, once per key while the
    /// face is kept; the face is ready from the pass after the job ends. A load that returns `None`, bytes egui cannot
    /// read, or a job that ends without a result marks the key unavailable for as long as the context lives. Every call
    /// marks the face drawn in this frame; a load no frame asks for any more is dropped.
    pub(crate) fn face<L>(ctx: &Context, key: &str, job: impl FnOnce() -> L) -> FaceState
    where
        L: FnOnce() -> Option<FaceBytes> + Send + 'static,
    {
        let name = Kind::Alone.family_name(key);
        match Self::known(ctx, &name) {
            Known::Failed => FaceState::Unavailable,
            Known::Ready => Self::ready(ctx, &name),
            Known::Loading => {
                // The asking view runs again in the next pass (its replay would hold the loading state) and keeps the
                // load asked for.
                frame::request_repaint(ctx);
                FaceState::Loading
            }
            Known::New => {
                // A measuring run starts the load too: the registry is a cache, and a face loads the same whichever
                // run asks.
                frame::request_repaint(ctx);
                let load = job();
                memory::with::<Registry, _>(ctx, |registry| {
                    let queue = queue_of(ctx, registry);
                    registry.loading.insert(name.clone(), registry.frame);
                    queue.submit(name, Priority::VISIBLE, move |cancel| {
                        if cancel.is_cancelled() {
                            return None;
                        }
                        load().filter(FaceBytes::parses)
                    });
                });
                FaceState::Loading
            }
        }
    }

    /// The face registered under `key` as a family of its own whose characters the face lacks come from the interface
    /// fonts (the proportional family's chain, CJK included). `load` gives bytes the app holds: it runs at once, the
    /// first time the key is drawn while the face is kept, and the face is ready in the same pass. A `load` that returns
    /// `None`, or bytes egui cannot read, marks the key unavailable. Its keys are apart from `face`'s: the same key may
    /// name a face drawn alone too.
    pub(crate) fn over_interface(ctx: &Context, key: &str, load: impl FnOnce() -> Option<FaceBytes>) -> FaceState {
        let name = Kind::OverInterface.family_name(key);
        match Self::known(ctx, &name) {
            Known::Failed => FaceState::Unavailable,
            Known::Ready => Self::ready(ctx, &name),
            Known::Loading | Known::New => {
                let Some(face) = load().filter(FaceBytes::parses) else {
                    memory::with::<Registry, _>(ctx, |registry| registry.failed.insert(name));
                    return FaceState::Unavailable;
                };
                let (data, chain) = (face.data(), face.chain_families());
                memory::with::<Registry, _>(ctx, |registry| {
                    let used = registry.frame;
                    registry.faces.insert(name.clone(), Registered { face, used });
                });
                if egui_fonts::serve_face(ctx, &FontRef::named(&name), data, true, chain) {
                    FaceState::Ready(FontRef::named(&name))
                } else {
                    frame::request_repaint(ctx);
                    FaceState::Loading
                }
            }
        }
    }

    /// The bytes of the face registered under `key` to be drawn alone, once it is ready.
    pub(crate) fn bytes(ctx: &Context, key: &str) -> Option<FaceBytes> {
        let name = Kind::Alone.family_name(key);
        memory::with::<Registry, _>(ctx, |registry| registry.faces.get(&name).map(|face| face.face.clone()))
    }

    /// What the registry knows of `name`, marking a kept face drawn and a load asked for in this frame. The mark of a
    /// kept face is journaled, so a settled view's replay marks the faces its run drew.
    fn known(ctx: &Context, name: &str) -> Known {
        begin(ctx);
        let known = memory::with::<Registry, _>(ctx, |registry| {
            let now = registry.frame;
            if registry.failed.contains(name) {
                Known::Failed
            } else if registry.faces.contains_key(name) {
                registry.mark_drawn(name);
                Known::Ready
            } else if let Some(asked) = registry.loading.get_mut(name) {
                *asked = now;
                Known::Loading
            } else {
                Known::New
            }
        });
        if matches!(known, Known::Ready) {
            replay::journal_distinct(ctx, || name.to_owned(), |ctx, name: &String| mark_drawn(ctx, name));
        }
        known
    }

    /// A kept face's state: ready once egui lays text out with the served faces.
    fn ready(ctx: &Context, name: &str) -> FaceState {
        if egui_fonts::is_served(ctx) {
            FaceState::Ready(FontRef::named(name))
        } else {
            frame::request_repaint(ctx);
            FaceState::Loading
        }
    }
}
