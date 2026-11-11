//! Faces drawn as families of their own, for text set in a document's fonts.
//!
//! One registry per context serves every caller, so the faces of a frame are counted together. A face is added a few
//! per frame; past [`MAX_DOCUMENT_FACES`] the faces the previous frame did not draw are taken out again, which keeps
//! memory to the faces on screen. Faces still on screen stay, however many there are.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use egui::epaint::text::{Blob, FontTweak, VariationCoords};
use egui::{Context, FontData};

use super::FontRef;
use crate::runtime::egui::{fonts as egui_fonts, memory, pass};
use crate::runtime::frame;

/// The faces one frame may add; a new face past it waits for the next frame.
pub const DOCUMENT_FACES_PER_PASS: usize = 6;

/// The faces kept before the ones the previous frame did not draw are taken out.
pub const MAX_DOCUMENT_FACES: usize = 64;

/// The bytes of a face, its index in a collection and its variation coordinates (axis tag, value).
#[derive(Clone)]
#[non_exhaustive]
pub struct FaceBytes {
    /// The font file.
    pub bytes: Arc<[u8]>,
    /// The face's index in the file.
    pub index: u32,
    /// The variation coordinates the face is drawn at.
    pub coords: Vec<([u8; 4], f32)>,
}

impl FaceBytes {
    /// Face `index` of the file `bytes`, at its default coordinates.
    pub fn new(bytes: Arc<[u8]>, index: u32) -> Self {
        FaceBytes { bytes, index, coords: Vec::new() }
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
}

/// What drawing a document face finds.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum FaceState {
    /// The family that draws the face.
    Ready(FontRef),
    /// The face is being added; a repaint is asked.
    Loading,
    /// The face could not be loaded.
    Unavailable,
}

/// A registered face: the frame it was last drawn in and the frame it was added in.
struct Registered {
    used: u64,
    added: u64,
}

/// The context's document faces.
#[derive(Default)]
struct Registry {
    faces: HashMap<String, Registered>,
    failed: HashSet<String>,
    budget: usize,
    frame: u64,
    begun: bool,
    evicted: Option<u64>,
    hooked: bool,
}

impl Registry {
    /// Starts frame `now` unless it has started: the budget is full again.
    fn begin(&mut self, now: u64) {
        if self.begun && self.frame == now {
            return;
        }
        self.begun = true;
        self.frame = now;
        self.budget = DOCUMENT_FACES_PER_PASS;
    }

    /// Past the cap, drops the faces the previous frame did not draw and returns them. Once per frame.
    fn evict(&mut self, now: u64) -> Vec<String> {
        if self.evicted == Some(now) || self.faces.len() <= MAX_DOCUMENT_FACES {
            return Vec::new();
        }
        self.evicted = Some(now);
        let stale: Vec<String> = self.faces.iter().filter(|(_, face)| face.used + 1 < now).map(|(key, _)| key.clone()).collect();
        for key in &stale {
            self.faces.remove(key);
        }
        stale.iter().map(|key| family_name(key)).collect()
    }
}

fn family_name(key: &str) -> String {
    format!("face:{key}")
}

/// What the registry knew of a key.
enum Known {
    Failed,
    Added(u64),
    New,
}

/// The begin-pass step: starts the frame and evicts the faces past the cap. Eviction happens only here, before any
/// install of the pass can queue its definitions, because it rebuilds them from the applied ones.
fn begin_pass(ctx: &Context) {
    let now = pass::root_frame_nr(ctx);
    let stale = memory::with::<Registry, _>(ctx, |registry| {
        registry.begin(now);
        registry.evict(now)
    });
    if !stale.is_empty() {
        egui_fonts::remove_faces(ctx, &stale);
    }
}

/// Hooks the begin-pass step on the context's fonts plugin and starts the frame's budget.
fn begin(ctx: &Context) {
    let hook = memory::with::<Registry, _>(ctx, |registry| !std::mem::replace(&mut registry.hooked, true));
    if hook {
        egui_fonts::on_begin_pass(ctx, begin_pass);
    }
    let now = pass::root_frame_nr(ctx);
    memory::with::<Registry, _>(ctx, |registry| registry.begin(now));
}

/// The document faces: the families drawn from font files a document names. Reached through `FacesCx`.
pub(crate) struct DocumentFaces;

impl DocumentFaces {
    /// The face registered under `key` as a family of its own. `load` runs only for a new key within the frame's budget
    /// of [`DOCUMENT_FACES_PER_PASS`], and for a key whose family a replacement of the definitions removed. A `load`
    /// that returns `None` marks the key unavailable for as long as the context lives. Every call marks the face drawn
    /// in this frame.
    pub(crate) fn face(ctx: &Context, key: &str, load: impl FnOnce() -> Option<FaceBytes>) -> FaceState {
        begin(ctx);
        let family = FontRef::named(&family_name(key));
        let (now, known) = memory::with::<Registry, _>(ctx, |registry| {
            let known = if registry.failed.contains(key) {
                Known::Failed
            } else if let Some(face) = registry.faces.get_mut(key) {
                face.used = registry.frame;
                Known::Added(face.added)
            } else {
                Known::New
            };
            (registry.frame, known)
        });
        match known {
            Known::Failed => return FaceState::Unavailable,
            Known::Added(_) if egui_fonts::has_family(ctx, &family) => return FaceState::Ready(family),
            Known::Added(added) if added == now => {
                frame::request_repaint(ctx);
                return FaceState::Loading;
            }
            Known::Added(_) | Known::New => {}
        }
        frame::request_repaint(ctx);
        let allowed = memory::with::<Registry, _>(ctx, |registry| {
            let allowed = registry.budget > 0;
            registry.budget = registry.budget.saturating_sub(1);
            allowed
        });
        if !allowed {
            return FaceState::Loading;
        }
        let Some(bytes) = load() else {
            memory::with::<Registry, _>(ctx, |registry| {
                registry.faces.remove(key);
                registry.failed.insert(key.to_owned());
            });
            return FaceState::Unavailable;
        };
        egui_fonts::add_family_face(ctx, &family, bytes.data());
        memory::with::<Registry, _>(ctx, |registry| registry.faces.insert(key.to_owned(), Registered { used: now, added: now }));
        FaceState::Loading
    }
}
