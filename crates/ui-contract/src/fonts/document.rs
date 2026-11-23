//! Faces drawn as families of their own, for text set in a document's fonts.
//!
//! One registry per context serves every caller, so the faces of a frame are counted together. A face is added a few
//! per frame; past [`MAX_DOCUMENT_FACES`] the faces the previous frame did not draw are taken out again, which keeps
//! memory to the faces on screen. Faces still on screen stay, however many there are.
//!
<<<<<<< HEAD
//! A face may name a chain of other document faces ([`FaceBytes::chain`]): a font split over several subsets (Word
//! embeds one subset per run of characters) draws each character from the first subset that has it, before the
//! interface family. The chains are linked in the begin-pass step after a face was added or taken out, when egui has
//! applied every face the pass before added.
=======
//! A face is drawn alone ([`DocumentFaces::face`]: a font sample shows what the face itself has) or over the interface
//! fonts ([`DocumentFaces::over_interface`]: document text such as an input method's composition, whose characters
//! the face lacks come from the interface stack, CJK included). A face over the interface fonts is installed by the
//! begin-pass step of the frame after it is asked for, as a family whose chain is the face followed by the
//! proportional family's: egui builds a family's chain from the definitions alone, so the chain is written into them
//! where nothing of the pass has queued definitions of its own.
>>>>>>> origin/next

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

<<<<<<< HEAD
/// The bytes of a face, its index in a collection, its variation coordinates (axis tag, value) and the faces that
/// draw what it lacks.
=======
/// The largest font file a face over the interface fonts is drawn from, in bytes (16 MiB). egui parses every face of
/// the definitions again whenever they change, and a document's CJK fallback can be far larger (Microsoft YaHei is
/// about 19 MB): text in a larger face is drawn in the interface fonts.
pub const MAX_INTERFACE_FACE_BYTES: usize = 16 << 20;

/// The bytes of a face, its index in a collection and its variation coordinates (axis tag, value).
>>>>>>> origin/next
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

/// A registered face: the frame it was last drawn in, the frame it was added in and the keys of its chain.
struct Registered {
    used: u64,
    added: u64,
    chain: Vec<String>,
}

/// The context's document faces, by family name.
#[derive(Default)]
struct Registry {
    faces: HashMap<String, Registered>,
    failed: HashSet<String>,
    /// Faces over the interface fonts waiting for the next begin-pass step: family name and face.
    pending: Vec<(String, FontData)>,
    budget: usize,
    frame: u64,
    begun: bool,
    evicted: Option<u64>,
    hooked: bool,
    /// A face was added or taken out since the chains were last linked.
    relink: bool,
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
        let stale: Vec<String> = self.faces.iter().filter(|(_, face)| face.used + 1 < now).map(|(name, _)| name.clone()).collect();
        for name in &stale {
            self.faces.remove(name);
        }
<<<<<<< HEAD
        self.relink |= !stale.is_empty();
        stale.iter().map(|key| family_name(key)).collect()
=======
        self.pending.retain(|(name, _)| !stale.contains(name));
        stale
>>>>>>> origin/next
    }

    /// The family of every face with a chain, with the families of its chain's registered faces in order.
    fn chains(&self) -> Vec<(String, Vec<String>)> {
        self.faces
            .iter()
            .filter(|(_, face)| !face.chain.is_empty())
            .map(|(key, face)| {
                let chain = face.chain.iter().filter(|link| *link != key && self.faces.contains_key(*link)).map(|link| family_name(link)).collect();
                (family_name(key), chain)
            })
            .collect()
    }

    /// Marks the face `key` and the faces of its chain drawn in this frame.
    fn mark_drawn(&mut self, key: &str) {
        let frame = self.frame;
        let Some(face) = self.faces.get_mut(key) else { return };
        face.used = frame;
        let chain = face.chain.clone();
        for link in &chain {
            if let Some(face) = self.faces.get_mut(link) {
                face.used = frame;
            }
        }
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
    Added(u64),
    New,
}

<<<<<<< HEAD
/// The begin-pass step: starts the frame, evicts the faces past the cap and, after a face was added or taken out,
/// links the chains. Both happen only here, before any install of the pass can queue its definitions, because they
/// rebuild them from the applied ones; egui has applied the faces the previous pass added before this step runs.
fn begin_pass(ctx: &Context) {
    let now = pass::root_frame_nr(ctx);
    let changed = memory::with::<Registry, _>(ctx, |registry| {
        registry.begin(now);
        let stale = registry.evict(now);
        std::mem::take(&mut registry.relink).then(|| (stale, registry.chains()))
    });
    if let Some((stale, chains)) = changed {
        egui_fonts::set_document_faces(ctx, &stale, &chains);
=======
/// The begin-pass step: starts the frame, evicts the faces past the cap and installs the faces over the interface fonts
/// the last frame loaded. Both happen only here, in one edit, before any install of the pass can queue its
/// definitions, because they rebuild them from the applied ones.
fn begin_pass(ctx: &Context) {
    let now = pass::root_frame_nr(ctx);
    let (stale, pending) = memory::with::<Registry, _>(ctx, |registry| {
        registry.begin(now);
        let stale = registry.evict(now);
        (stale, std::mem::take(&mut registry.pending))
    });
    if !stale.is_empty() || !pending.is_empty() {
        egui_fonts::edit_faces(ctx, &stale, pending);
>>>>>>> origin/next
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
        Self::register(ctx, key, Kind::Alone, load)
    }

    /// The face registered under `key` as a family of its own whose characters the face lacks come from the interface
    /// fonts (the proportional family's chain, CJK included). As [`face`](Self::face), with two differences: the face
    /// is ready from the frame after the one that loaded it, and a file larger than [`MAX_INTERFACE_FACE_BYTES`] marks
    /// the key unavailable. Its keys are apart from `face`'s: the same key may name a face drawn alone too.
    pub(crate) fn over_interface(ctx: &Context, key: &str, load: impl FnOnce() -> Option<FaceBytes>) -> FaceState {
        Self::register(ctx, key, Kind::OverInterface, load)
    }

    fn register(ctx: &Context, key: &str, kind: Kind, load: impl FnOnce() -> Option<FaceBytes>) -> FaceState {
        begin(ctx);
        let name = kind.family_name(key);
        let family = FontRef::named(&name);
        let (now, known) = memory::with::<Registry, _>(ctx, |registry| {
            let known = if registry.failed.contains(&name) {
                Known::Failed
<<<<<<< HEAD
            } else if let Some(added) = registry.faces.get(key).map(|face| face.added) {
                registry.mark_drawn(key);
                Known::Added(added)
=======
            } else if let Some(face) = registry.faces.get_mut(&name) {
                face.used = registry.frame;
                Known::Added(face.added)
>>>>>>> origin/next
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
        let loaded = load().filter(|bytes| kind == Kind::Alone || bytes.bytes.len() <= MAX_INTERFACE_FACE_BYTES);
        let Some(bytes) = loaded else {
            memory::with::<Registry, _>(ctx, |registry| {
                registry.faces.remove(&name);
                registry.failed.insert(name.clone());
            });
            return FaceState::Unavailable;
        };
<<<<<<< HEAD
        egui_fonts::add_family_face(ctx, &family, bytes.data());
        memory::with::<Registry, _>(ctx, |registry| {
            registry.faces.insert(key.to_owned(), Registered { used: now, added: now, chain: bytes.chain });
            registry.relink = true;
=======
        if kind == Kind::Alone {
            egui_fonts::add_family_face(ctx, &family, bytes.data());
        }
        memory::with::<Registry, _>(ctx, |registry| {
            if kind == Kind::OverInterface {
                registry.pending.retain(|(pending, _)| *pending != name);
                registry.pending.push((name.clone(), bytes.data()));
            }
            registry.faces.insert(name.clone(), Registered { used: now, added: now })
>>>>>>> origin/next
        });
        FaceState::Loading
    }
}
