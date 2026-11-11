//! On-demand font registration: a face for a script is read and added to the stack the
//! first time a visible character of that script has no glyph, instead of every candidate face
//! being installed up front. It covers CJK and every other script the same way.

#[cfg(not(target_arch = "wasm32"))]
mod files;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use craft_host::Wake;
#[cfg(not(target_arch = "wasm32"))]
use craft_host::target::Runtime;
use craft_i18n::Han;
use egui::{Context, FontData, Shape};
use skrifa::MetadataProvider as _;
use skrifa::raw::{FileRef, TableProvider as _};

use super::faces::{baseline_fallback, set_weight};
use super::script::{EMOJI, MATH, SYMBOLS, script_of};
use super::stack::is_cjk;
use crate::runtime::egui::fonts as egui_fonts;
use crate::runtime::egui::memory;

/// Candidate font files per script, most preferred first. Each is read only when a character of
/// that script is missing, at most [`MAX_SCRIPT_READS`] files per script and one face per pass.
/// Apps and tests plug in their own source: the system's fonts ([`SystemCandidates`]), a fixed
/// list, or none ([`NoCandidates`]) so a test never depends on the machine.
///
/// Scripts are ISO 15924 codes: `Hani` (ideographs, CJK punctuation and full-width forms), `Kana`,
/// `Hang`, `Bopo`, and the other scripts of [`craft_i18n::Script`] that need a face beyond the
/// Latin, Cyrillic and Greek of the app's own (`Thai`, `Arab`, `Deva`, ...).
pub trait FontCandidates: Send + Sync {
    /// Files that may cover `script` (ISO 15924), most preferred first. For Han the `han`
    /// preference orders the regional faces (Japanese before Simplified for a `ja` locale).
    fn files(&self, script: &str, han: Option<Han>) -> Vec<PathBuf>;
    /// The face index in `data` whose character map has `c`; `None` when none has it.
    fn covers(&self, data: &[u8], c: char) -> Option<u32>;
    /// A background lookup (fontconfig's CJK scan) is still running: the loader waits for the
    /// preferred script instead of falling through to another one.
    fn pending(&self) -> bool {
        false
    }
    /// Call `wake` once when the lookup that [`pending`](FontCandidates::pending) reports ends;
    /// at once when none is running. The loader asks once per lookup and requests a frame from
    /// it, so a pending lookup costs no timer.
    fn wake_when_ready(&self, wake: Wake) {
        wake.wake();
    }
}

/// The installed fonts as candidates: fontconfig on Linux, the font directories on macOS and
/// Windows (`craft_font_catalog::font_dirs`). Not on the web.
#[derive(Clone, Debug, Default)]
pub struct SystemCandidates;

impl FontCandidates for SystemCandidates {
    fn files(&self, script: &str, han: Option<Han>) -> Vec<PathBuf> {
        #[cfg(not(target_arch = "wasm32"))]
        if Runtime::current() != Runtime::Web {
            return files::files(script, han);
        }
        let _ = (script, han);
        Vec::new()
    }

    fn covers(&self, data: &[u8], c: char) -> Option<u32> {
        covering_face(data, c)
    }

    fn pending(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        return Runtime::current() == Runtime::Native && files::pending();
        #[cfg(target_arch = "wasm32")]
        false
    }

    fn wake_when_ready(&self, wake: Wake) {
        #[cfg(not(target_arch = "wasm32"))]
        if Runtime::current() != Runtime::Web {
            return files::wake_when_ready(wake);
        }
        wake.wake();
    }
}

/// No candidates: text draws with the installed stack only (tests, the web build).
#[derive(Clone, Debug, Default)]
pub struct NoCandidates;

impl FontCandidates for NoCandidates {
    fn files(&self, _script: &str, _han: Option<Han>) -> Vec<PathBuf> {
        Vec::new()
    }

    fn covers(&self, _data: &[u8], _c: char) -> Option<u32> {
        None
    }
}

/// How many candidate files a script may read before the loader gives up on it (PhotoCraft's cap).
pub const MAX_SCRIPT_READS: usize = 6;

/// Skip absurdly large files: a corrupt or non-font path must not eat memory.
const MAX_FONT_BYTES: u64 = 128 << 20;

/// The loader that registers faces on demand, kept in the context's runtime store. A host installs one with
/// [`OnDemandFonts::install`].
///
/// After each pass it looks at the shapes drawn for a missing glyph, picks the script of the first such
/// character, reads that script's candidates until one covers it, and registers that face after
/// the craft faces of every family (lowest priority, so it follows the stack's own chain). A face
/// is read once; a script whose candidates are exhausted is not tried again until the fonts are
/// reinstalled.
pub struct OnDemandFonts {
    candidates: Arc<dyn FontCandidates>,
    han: Option<Han>,
    /// Scripts tried so far and how many files each has read. A script that ran out of
    /// candidates counts [`MAX_SCRIPT_READS`].
    reads: Vec<(String, usize)>,
    /// Faces registered so far: (font name, file, face index).
    registered: Vec<(String, PathBuf, u32)>,
    /// The pass after an install registers nothing: egui applies `set_fonts` at the start of the
    /// next pass, so this pass's shapes were laid out with the old stack.
    skip_pass: bool,
    /// A face was registered by the last pass; egui adds it at the start of the next one.
    adding: bool,
    /// The candidates were asked to wake the UI when their running lookup ends.
    waiting: bool,
}

impl OnDemandFonts {
    /// A loader over `candidates`, with the Han preference of the effective locale.
    pub fn new(candidates: Arc<dyn FontCandidates>, han: Option<Han>) -> Self {
        OnDemandFonts { candidates, han, reads: Vec::new(), registered: Vec::new(), skip_pass: false, adding: false, waiting: false }
    }

    /// Installs the loader on `ctx`, replacing an earlier one: `set_fonts` drops the registered faces, so an earlier
    /// loader's state is replaced rather than kept.
    pub fn install(mut self, ctx: &Context) {
        self.skip_pass = true;
        memory::with::<Loader, _>(ctx, |loader| loader.0 = Some(self));
        egui_fonts::ensure_plugin(ctx);
    }

    fn reads_of(&self, script: &str) -> usize {
        self.reads.iter().find(|(s, _)| s == script).map_or(0, |(_, n)| *n)
    }

    fn wanted(&self, script: &str) -> bool {
        self.reads_of(script) < MAX_SCRIPT_READS
    }

    fn count_read(&mut self, script: &str) {
        match self.reads.iter_mut().find(|(s, _)| s == script) {
            Some((_, n)) => *n += 1,
            None => self.reads.push((script.to_owned(), 1)),
        }
    }

    fn set_reads(&mut self, script: &str, reads: usize) {
        match self.reads.iter_mut().find(|(s, _)| s == script) {
            Some((_, n)) => *n = reads,
            None => self.reads.push((script.to_owned(), reads)),
        }
    }

    fn exhaust(&mut self, script: &str) {
        self.set_reads(script, MAX_SCRIPT_READS);
    }

    /// The first character with no glyph among `shapes`, if any: what the next pass loads for.
    /// Only characters of scripts that still have candidates to read count.
    fn missing_char(&self, ctx: &Context, shapes: &[egui::epaint::ClippedShape]) -> Option<char> {
        fn collect(shape: &Shape, out: &mut Vec<char>, wanted: &dyn Fn(char) -> bool) {
            match shape {
                Shape::Text(t) => {
                    let text = &t.galley.job.text;
                    if !text.is_ascii() {
                        for c in text.chars().filter(|c| wanted(*c)) {
                            if out.len() >= 256 {
                                return;
                            }
                            if !out.contains(&c) {
                                out.push(c);
                            }
                        }
                    }
                }
                Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out, wanted)),
                _ => {}
            }
        }
        let wanted = |c: char| face_script(c).is_some_and(|script| self.wanted(script));
        let mut chars = Vec::new();
        for shape in shapes {
            collect(&shape.shape, &mut chars, &wanted);
        }
        egui_fonts::first_without_glyph(ctx, chars)
    }

    /// Whether the stack was reinstalled since the last face was registered: egui's definitions
    /// no longer hold it.
    fn faces_dropped(&self, ctx: &Context) -> bool {
        let Some((name, ..)) = self.registered.first() else { return false };
        !self.adding && !egui_fonts::has_face(ctx, name)
    }

    /// The next face to register for `c`: the first unread candidate of its script whose
    /// character map has it. `None` when nothing is left, or while a background lookup may still
    /// add candidates for the preferred script.
    fn next_face(&mut self, ctx: &Context, c: char, script: &'static str) -> Option<(String, FontData)> {
        let cjk = is_cjk(script);
        let pending = cjk && self.candidates.pending();
        if pending && !self.waiting {
            self.waiting = true;
            let ctx = ctx.clone();
            self.candidates.wake_when_ready(Wake::new(move || egui_fonts::wake(&ctx)));
        } else if !pending {
            self.waiting = false;
        }
        let reads_before = self.reads_of(script);
        let region = match script {
            "Kana" => Some(Han::Japanese),
            "Hang" => Some(Han::Korean),
            "Bopo" => Some(Han::Traditional),
            _ => self.han,
        };
        for path in self.candidates.files(script, self.han) {
            if self.reads_of(script) >= MAX_SCRIPT_READS {
                break;
            }
            if self.registered.iter().any(|(_, p, _)| *p == path) {
                continue;
            }
            let Some(bytes) = read_font(&path) else { continue };
            self.count_read(script);
            if !file_is_readable(&bytes) {
                continue;
            }
            let Some(mut index) = self.candidates.covers(&bytes, c) else { continue };
            if let Some(region) = region.filter(|_| cjk) {
                index = regional_face(&bytes, c, region).unwrap_or(index);
            }
            if !face_is_readable(&bytes, index) {
                continue;
            }
            let name = format!("on-demand-{}", self.registered.len());
            log::info!("UI font fallback: registered {} (face {index}) for {script} as {name}", path.display());
            self.registered.push((name.clone(), path, index));
            let mut data = FontData::from_owned(bytes);
            data.index = index;
            return Some((name, data));
        }
        if pending {
            // The lookup may add a better candidate: try again, reading the same files.
            self.set_reads(script, reads_before);
            return None;
        }
        log::info!("UI font fallback: no installed font covers {script}");
        self.exhaust(script);
        None
    }
}

impl OnDemandFonts {
    /// What a pass leaves to do: registers one face for the first drawn character without a glyph.
    fn scan(&mut self, ctx: &Context, shapes: &[egui::epaint::ClippedShape]) {
        if std::mem::take(&mut self.skip_pass) {
            egui_fonts::wake(ctx);
            return;
        }
        if self.faces_dropped(ctx) {
            *self = OnDemandFonts::new(self.candidates.clone(), self.han);
            egui_fonts::wake(ctx);
            return;
        }
        self.adding = false;
        let Some(c) = self.missing_char(ctx, shapes) else { return };
        let Some(script) = face_script(c) else { return };
        let pending = is_cjk(script) && self.candidates.pending();
        match self.next_face(ctx, c, script) {
            Some((name, data)) => {
                register(ctx, name, data);
                self.adding = true;
                egui_fonts::wake(ctx);
            }
            // The lookup's wake requests the next frame.
            None if pending => {}
            // The script ran out of candidates for this character; another script may still load.
            None => egui_fonts::wake(ctx),
        }
    }
}

/// The context's loader, if one is installed.
#[derive(Default)]
struct Loader(Option<OnDemandFonts>);

/// Runs the installed loader after a pass that painted `shapes`. The loader is out of the store while it runs, so it
/// reads and writes the context without holding the store's lock.
pub(crate) fn after_pass(ctx: &Context, shapes: &[egui::epaint::ClippedShape]) {
    let Some(mut loader) = memory::with::<Loader, _>(ctx, |loader| loader.0.take()) else { return };
    loader.scan(ctx, shapes);
    memory::with::<Loader, _>(ctx, |slot| {
        if slot.0.is_none() {
            slot.0 = Some(loader);
        }
    });
}

/// The faces the context's loader registered so far (font name, file, face index), for the font list in
/// Preferences and the tests.
pub fn on_demand_registered(ctx: &Context) -> Vec<(String, PathBuf, u32)> {
    memory::with::<Loader, _>(ctx, |loader| loader.0.as_ref().map(|loader| loader.registered.clone()).unwrap_or_default())
}

/// Adds `data` to every font family at the lowest priority, on the primary face's baseline and at
/// regular weight (variable system faces often default to Thin).
fn register(ctx: &Context, name: String, data: FontData) {
    let mut data = baseline_fallback(data);
    set_weight(&mut data, 400);
    egui_fonts::add_face(ctx, name, data);
}

/// The ISO 15924 script of a character that may need an on-demand face, or `None` for Latin,
/// Cyrillic, Greek, symbols and everything the stack's own faces are expected to draw. Hiragana
/// is looked up as `Kana`.
fn face_script(c: char) -> Option<&'static str> {
    match script_of(c)? {
        "Hira" => Some("Kana"),
        "Latn" | "Cyrl" | "Grek" | "Syrc" | "Thaa" | "Mong" | EMOJI | SYMBOLS | MATH => None,
        script => Some(script),
    }
}

/// A font file's bytes, or `None` for a missing, empty, oversized or unreadable file.
fn read_font(path: &Path) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_FONT_BYTES {
        return None;
    }
    std::fs::read(path).ok()
}

/// Whether the text engine can parse the file: it is a font or a collection with a face.
fn file_is_readable(bytes: &[u8]) -> bool {
    match FileRef::new(bytes) {
        Ok(FileRef::Font(_)) => true,
        Ok(FileRef::Collection(collection)) => !collection.is_empty(),
        Err(_) => false,
    }
}

/// Whether face `index` parses and has the tables egui reads; egui panics on a face whose table
/// directory points outside the file, so a face is checked before it is registered.
fn face_is_readable(bytes: &[u8], index: u32) -> bool {
    let Ok(font) = skrifa::FontRef::from_index(bytes, index) else { return false };
    font.head().is_ok() && font.hhea().is_ok() && font.maxp().is_ok() && font.cmap().is_ok()
}

/// The index of the first face in `bytes` (a font or a collection) whose character map has `c`.
/// `None` for fonts without it and for data that doesn't parse.
fn covering_face(bytes: &[u8], c: char) -> Option<u32> {
    match FileRef::new(bytes).ok()? {
        FileRef::Font(font) => font.charmap().map(c).map(|_| 0),
        FileRef::Collection(collection) => {
            collection.iter().position(|font| font.is_ok_and(|font| font.charmap().map(c).is_some())).and_then(|i| u32::try_from(i).ok())
        }
    }
}

/// In a collection of regional faces (Noto Sans CJK JP, SC, TC, HK, KR), the first face that has
/// `c` and is the `region`'s: its family name ends with the region's code. `None` for a single
/// font or a collection without such a face, which leaves the first covering face.
fn regional_face(bytes: &[u8], c: char, region: Han) -> Option<u32> {
    let FileRef::Collection(collection) = FileRef::new(bytes).ok()? else { return None };
    let suffix = match region {
        Han::Simplified => "SC",
        Han::Traditional => "TC",
        Han::HongKong => "HK",
        Han::Japanese => "JP",
        Han::Korean => "KR",
        _ => return None,
    };
    let index = collection.iter().position(|font| {
        font.is_ok_and(|font| {
            font.charmap().map(c).is_some()
                && font
                    .localized_strings(skrifa::string::StringId::FAMILY_NAME)
                    .english_or_first()
                    .is_some_and(|name| name.to_string().ends_with(suffix))
        })
    })?;
    u32::try_from(index).ok()
}

/// The Han preference for the interface: the UI language's when it is Japanese, Korean or Chinese,
/// else the OS locale's, else none (`Hani` falls to the manifest order). PhotoCraft's rule for
/// which regional CJK face wins a shared code point.
pub fn han_for_locale(ui_language: &str, os_locale: Option<&str>) -> Option<Han> {
    craft_i18n::han(ui_language).or_else(|| os_locale.and_then(craft_i18n::han))
}
