//! Faces a web build serves beside its wasm instead of embedding them, fetched when the UI needs
//! them.

use std::sync::Arc;

use egui::FontFamily;
use egui::epaint::text::Blob;

use super::{CjkRows, CraftFace, EM_SQUARE_SCRIPTS, Stack, baseline_fallback, set_weight};

#[cfg(any(test, target_arch = "wasm32"))]
pub(super) mod loader;
#[cfg(target_arch = "wasm32")]
pub(super) mod web;

/// The script tag served emoji faces get: ISO 15924 `Zsye`, emoji.
pub(super) const EMOJI: &str = "Zsye";
/// ISO 15924 `Zsym`, symbols drawn as text: arrows, keyboard keys, dingbats.
pub(super) const SYMBOLS: &str = "Zsym";
/// ISO 15924 `Zmth`, mathematical notation.
pub(super) const MATH: &str = "Zmth";

/// A craft-fonts face served beside the app, one per line of `fonts/manifest.txt`. The tables that
/// `craft-fonts-build`'s `Embed::serve` writes are slices of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServedFace {
    /// Family name, e.g. `"Noto Sans Arabic"`.
    pub family: &'static str,
    /// `"Regular"` or `"Bold"`; faces with other styles are not used for UI text.
    pub style: &'static str,
    /// The manifest's script tags, as in [`CraftFace::scripts`].
    pub scripts: &'static [&'static str],
    /// Face index in a font collection (`.ttc`); faces of one collection share a URL.
    pub index: u32,
    /// Where the file is fetched from, relative to the page or absolute.
    pub url: &'static str,
    /// File size in bytes.
    pub size: u64,
    /// Subresource Integrity value (`sha256-…`) the browser checks the file against; empty for
    /// none.
    pub integrity: &'static str,
    /// The characters the face maps, in `craft_fonts_build::coverage`'s encoding. A character no
    /// served face maps fetches nothing.
    pub coverage: &'static [u8],
}

impl ServedFace {
    /// The egui font name, e.g. `"Noto Sans Arabic Regular"`.
    pub fn name(&self) -> String {
        format!("{} {}", self.family, self.style)
    }

    /// Whether the face maps `c`.
    pub fn covers(&self, c: char) -> bool {
        craft_fonts_build::coverage::contains(self.coverage, c)
    }

    /// The face as the stack orders it, without bytes.
    pub(super) fn craft_face(&self) -> CraftFace {
        CraftFace { family: self.family, style: self.style, scripts: self.scripts, bytes: &[], index: self.index }
    }
}

impl Stack {
    /// The served faces in the stack's chains (see [`super::UiFonts::served_faces`]).
    pub fn served(&self) -> &[ServedFace] {
        &self.served
    }

    /// The served faces the UI language needs, which [`Self::install`] fetches at once on the web:
    /// for each family, the best face for each of the language's scripts that the family's own
    /// faces lack. Empty for a language the app's faces cover.
    pub fn ui_downloads(&self) -> Vec<&ServedFace> {
        self.ui_downloads.iter().filter_map(|name| self.served_face(name)).collect()
    }

    /// The served faces that draw `c` in `family`, in the order the family asks them: those
    /// whose file maps `c` ([`ServedFace::covers`]) and that are tagged with `c`'s script (see
    /// [`script_of`]). The family's order puts the UI language's faces first, so a Han character
    /// takes the face of the UI language's region (Simplified, Traditional, Hong Kong, Japanese or
    /// Korean forms), then the other offered languages' and the rest in manifest order. Letters
    /// of Latin, Greek and Cyrillic take faces such as Noto Sans, not the CJK faces, which carry
    /// those scripts at CJK proportions. Keyboard keys, arrows and other symbols take the symbol
    /// faces (`Zsym`), mathematical notation the math faces (`Zmth`) and then the symbol faces,
    /// and emoji the emoji faces. Empty for characters of no particular script and for
    /// characters no served face maps.
    pub fn downloads_for(&self, family: &FontFamily, c: char) -> Vec<&ServedFace> {
        self.downloads(family, c.encode_utf8(&mut [0; 4]))
    }

    /// [`Self::downloads_for`] the first character of a cluster, whose variation selector
    /// chooses between emoji and symbol faces (see [`script_of_cluster`]).
    pub(super) fn downloads(&self, family: &FontFamily, cluster: &str) -> Vec<&ServedFace> {
        let (Some(c), Some(script)) = (cluster.chars().next(), script_of_cluster(cluster)) else { return Vec::new() };
        let wanted = if script == MATH { [MATH, SYMBOLS].as_slice() } else { std::slice::from_ref(&script) };
        let alphabet = matches!(script, "Latn" | "Grek" | "Cyrl");
        let configured = self.definitions.families.get(family).or_else(|| self.definitions.families.get(&FontFamily::Proportional));
        let mut faces: Vec<&ServedFace> = Vec::new();
        for want in wanted {
            for name in configured.into_iter().flatten().chain(self.after_system.chain(family)) {
                if let Some(face) = self.served_face(name)
                    && face.craft_face().covers(want)
                    && !(alphabet && face.craft_face().covers("Hani"))
                    && face.covers(c)
                    && !faces.contains(&face)
                {
                    faces.push(face);
                }
            }
        }
        faces
    }

    /// Add the file served at `url` to every served face it holds, for [`Self::install`] to
    /// install. A face in a family of another weight is set to that weight, and renamed as
    /// [`super::UiFonts::stack`] would have. The web build does this for each file it fetches; an
    /// app that gets the files some other way can call it itself.
    pub fn provide(&mut self, url: &str, file: &Blob) {
        let faces: Vec<ServedFace> = self.served.iter().filter(|face| face.url == url).copied().collect();
        for face in faces {
            let name = face.name();
            let data = baseline_fallback(egui::FontData::from_blob(Arc::clone(file), face.index));
            let weight = |family: &FontFamily| self.weights.iter().find(|(f, _)| f == family).map_or(400, |(_, w)| *w);
            let weighted = |family: &FontFamily| {
                let mut data = data.clone();
                let weight = weight(family);
                let name = if set_weight(&mut data, weight) && weight != 400 { format!("{name} @{weight}") } else { name.clone() };
                (name, data)
            };
            let mut configured = Vec::new();
            for (family, chain) in &mut self.definitions.families {
                for entry in chain.iter_mut().filter(|entry| **entry == name) {
                    let (new_name, data) = weighted(family);
                    if *family == FontFamily::Proportional && self.script_face.as_ref() == Some(entry) {
                        self.script_face = Some(new_name.clone());
                    }
                    entry.clone_from(&new_name);
                    configured.push((new_name, data));
                }
            }
            let mut after = Vec::new();
            for (family, chain) in self.after_system.chains_mut() {
                for entry in chain.iter_mut().filter(|entry| **entry == name) {
                    let (new_name, data) = weighted(family);
                    entry.clone_from(&new_name);
                    after.push((new_name, data));
                }
            }
            let mut names: Vec<String> = configured.iter().map(|(name, _)| name.clone()).collect();
            for (name, data) in configured {
                self.definitions.font_data.insert(name, Arc::new(data));
            }
            names.extend(after.iter().map(|(name, _)| name.clone()));
            for (name, data) in after {
                self.after_system.insert(&name, data);
            }
            if self.cjk_rows == CjkRows::Fit && EM_SQUARE_SCRIPTS.iter().any(|script| face.craft_face().covers(script)) {
                self.fit_into_rows(&names);
            }
        }
    }

    fn served_face(&self, name: &str) -> Option<&ServedFace> {
        self.served.iter().find(|face| face.name() == name)
    }
}

/// Whether the UI language's served faces are still being fetched, for an app that waits for them
/// before drawing its interface. Always false outside the web, and once every fetch has finished,
/// failed ones included.
#[cfg(target_arch = "wasm32")]
pub fn loading(ctx: &egui::Context) -> bool {
    web::loading(ctx)
}

/// Whether the UI language's served faces are still being fetched: never, outside the web.
#[cfg(not(target_arch = "wasm32"))]
pub fn loading(_ctx: &egui::Context) -> bool {
    false
}

/// The ISO 15924 script a served face must have to draw `c`, by Unicode block. Symbols with an
/// emoji presentation by default (Unicode's `Emoji_Presentation`) map to [`EMOJI`], other
/// symbols, such as ⌘ ⌥ ⇧ ⌃ ⎋ ⏎ ⌫, to [`SYMBOLS`]. `None` for characters of no block listed.
pub(super) fn script_of(c: char) -> Option<&'static str> {
    // Sorted by start; ranges don't overlap.
    const BLOCKS: &[(u32, u32, &str)] = &[
        (0x0020, 0x036F, "Latn"),
        (0x0370, 0x03FF, "Grek"),
        (0x0400, 0x052F, "Cyrl"),
        (0x0530, 0x058F, "Armn"),
        (0x0590, 0x05FF, "Hebr"),
        (0x0600, 0x06FF, "Arab"),
        (0x0700, 0x074F, "Syrc"),
        (0x0750, 0x077F, "Arab"),
        (0x0780, 0x07BF, "Thaa"),
        (0x0860, 0x086F, "Syrc"),
        (0x0870, 0x08FF, "Arab"),
        (0x0900, 0x097F, "Deva"),
        (0x0980, 0x09FF, "Beng"),
        (0x0A00, 0x0A7F, "Guru"),
        (0x0A80, 0x0AFF, "Gujr"),
        (0x0B00, 0x0B7F, "Orya"),
        (0x0B80, 0x0BFF, "Taml"),
        (0x0C00, 0x0C7F, "Telu"),
        (0x0C80, 0x0CFF, "Knda"),
        (0x0D00, 0x0D7F, "Mlym"),
        (0x0D80, 0x0DFF, "Sinh"),
        (0x0E00, 0x0E7F, "Thai"),
        (0x0E80, 0x0EFF, "Laoo"),
        (0x0F00, 0x0FFF, "Tibt"),
        (0x1000, 0x109F, "Mymr"),
        (0x10A0, 0x10FF, "Geor"),
        (0x1100, 0x11FF, "Hang"),
        (0x1200, 0x139F, "Ethi"),
        (0x1780, 0x17FF, "Khmr"),
        (0x1800, 0x18AF, "Mong"),
        (0x19E0, 0x19FF, "Khmr"),
        (0x1C80, 0x1C8F, "Cyrl"),
        (0x1C90, 0x1CBF, "Geor"),
        (0x1D00, 0x1EFF, "Latn"),
        (0x1F00, 0x1FFF, "Grek"),
        (0x2000, 0x20CF, "Latn"),
        (0x2100, 0x214F, SYMBOLS),
        (0x2150, 0x218F, "Latn"),
        (0x2190, 0x21FF, SYMBOLS),
        (0x2200, 0x22FF, MATH),
        (0x2300, 0x27BF, SYMBOLS),
        (0x27C0, 0x27EF, MATH),
        (0x27F0, 0x297F, SYMBOLS),
        (0x2980, 0x2AFF, MATH),
        (0x2B00, 0x2BFF, SYMBOLS),
        (0x2C60, 0x2C7F, "Latn"),
        (0x2D00, 0x2D2F, "Geor"),
        (0x2D80, 0x2DDF, "Ethi"),
        (0x2DE0, 0x2DFF, "Cyrl"),
        (0x2E00, 0x2E7F, "Latn"),
        (0x2E80, 0x303F, "Hani"),
        (0x3040, 0x309F, "Hira"),
        (0x30A0, 0x30FF, "Kana"),
        (0x3100, 0x312F, "Bopo"),
        (0x3130, 0x318F, "Hang"),
        (0x31A0, 0x31BF, "Bopo"),
        (0x31C0, 0x31EF, "Hani"),
        (0x31F0, 0x31FF, "Kana"),
        (0x3200, 0x9FFF, "Hani"),
        (0xA640, 0xA69F, "Cyrl"),
        (0xA720, 0xA7FF, "Latn"),
        (0xA8E0, 0xA8FF, "Deva"),
        (0xA960, 0xA97F, "Hang"),
        (0xAB00, 0xAB2F, "Ethi"),
        (0xAB30, 0xAB6F, "Latn"),
        (0xAC00, 0xD7FF, "Hang"),
        (0xF900, 0xFAFF, "Hani"),
        (0xFB00, 0xFB06, "Latn"),
        (0xFB13, 0xFB17, "Armn"),
        (0xFB1D, 0xFB4F, "Hebr"),
        (0xFB50, 0xFDFF, "Arab"),
        (0xFE30, 0xFE4F, "Hani"),
        (0xFE70, 0xFEFF, "Arab"),
        (0xFF00, 0xFF65, "Hani"),
        (0xFF66, 0xFF9F, "Kana"),
        (0xFFA0, 0xFFDC, "Hang"),
        (0x1D400, 0x1D7FF, MATH),
        (0x1F000, 0x1FAFF, EMOJI),
        (0x1FB00, 0x1FBFF, SYMBOLS),
        (0x20000, 0x3FFFF, "Hani"),
    ];
    // The symbols among the BLOCKS' symbols that are emoji by default.
    const EMOJI_PRESENTATION: &[(u32, u32)] = &[
        (0x231A, 0x231B),
        (0x23E9, 0x23EC),
        (0x23F0, 0x23F0),
        (0x23F3, 0x23F3),
        (0x25FD, 0x25FE),
        (0x2614, 0x2615),
        (0x2648, 0x2653),
        (0x267F, 0x267F),
        (0x2693, 0x2693),
        (0x26A1, 0x26A1),
        (0x26AA, 0x26AB),
        (0x26BD, 0x26BE),
        (0x26C4, 0x26C5),
        (0x26CE, 0x26CE),
        (0x26D4, 0x26D4),
        (0x26EA, 0x26EA),
        (0x26F2, 0x26F3),
        (0x26F5, 0x26F5),
        (0x26FA, 0x26FA),
        (0x26FD, 0x26FD),
        (0x2705, 0x2705),
        (0x270A, 0x270B),
        (0x2728, 0x2728),
        (0x274C, 0x274C),
        (0x274E, 0x274E),
        (0x2753, 0x2755),
        (0x2757, 0x2757),
        (0x2795, 0x2797),
        (0x27B0, 0x27B0),
        (0x27BF, 0x27BF),
        (0x2B1B, 0x2B1C),
        (0x2B50, 0x2B50),
        (0x2B55, 0x2B55),
    ];
    // The emoji BLOCKS' characters that are symbols, drawn as text by default: mahjong, domino
    // and playing card faces, enclosed letters, ornaments, geometric shapes, arrows, chess.
    const TEXT_PRESENTATION: &[(u32, u32)] = &[
        (0x1F000, 0x1F003),
        (0x1F005, 0x1F0CE),
        (0x1F0D0, 0x1F18D),
        (0x1F18F, 0x1F190),
        (0x1F19B, 0x1F1E5),
        (0x1F650, 0x1F67F),
        (0x1F780, 0x1F7DF),
        (0x1F800, 0x1F8FF),
        (0x1FA00, 0x1FA6F),
    ];
    let c = u32::from(c);
    let script = find(BLOCKS, c, |(start, end, _)| (*start, *end)).map(|(.., script)| *script)?;
    Some(match script {
        SYMBOLS if find(EMOJI_PRESENTATION, c, |r| *r).is_some() => EMOJI,
        EMOJI if find(TEXT_PRESENTATION, c, |r| *r).is_some() => SYMBOLS,
        script => script,
    })
}

/// [`script_of`] the cluster's first character, with an emoji or text variation selector
/// (U+FE0F, U+FE0E) choosing between [`EMOJI`] and [`SYMBOLS`].
pub(super) fn script_of_cluster(cluster: &str) -> Option<&'static str> {
    let script = script_of(cluster.chars().next()?)?;
    Some(if cluster.contains('\u{FE0F}') {
        EMOJI
    } else if script == EMOJI && cluster.contains('\u{FE0E}') {
        SYMBOLS
    } else {
        script
    })
}

/// The entry of `ranges` (sorted by start, not overlapping) that holds `c`.
fn find<T>(ranges: &[T], c: u32, range: impl Fn(&T) -> (u32, u32)) -> Option<&T> {
    let i = ranges.partition_point(|r| range(r).0 <= c);
    let entry = ranges.get(i.checked_sub(1)?)?;
    (c <= range(entry).1).then_some(entry)
}
