//! Faces a web build serves beside its wasm instead of embedding them, fetched when the UI needs them.

pub(in crate::fonts) mod loader;

use std::sync::Arc;

use craft_fonts_build::ServedFace;
use egui::FontFamily;
use egui::epaint::text::Blob;

use super::faces::baseline_fallback;
use super::faces::set_weight;
use super::script::{MATH, SYMBOLS, script_of_cluster};
use super::stack::{EM_SQUARE_SCRIPTS, covers};
use super::{FontRef, Stack};
use crate::fonts::stack::CjkRows;

impl Stack {
    /// The served faces in the stack's chains (see [`super::UiFonts::served_faces`]).
    pub fn served(&self) -> &[ServedFace] {
        &self.served
    }

    /// The served faces the UI language needs, which [`Self::install`] fetches at once: for each
    /// family, the best face for each of the language's scripts that the family's own faces lack.
    /// Empty for a language the app's faces cover.
    pub fn ui_downloads(&self) -> Vec<&ServedFace> {
        self.ui_downloads.iter().filter_map(|name| self.served_face(name)).collect()
    }

    /// The served faces that draw `c` in `family`, in the order the family asks them: those
    /// whose file maps `c` ([`ServedFace::covers`]) and that are tagged with `c`'s script. The
    /// family's order puts the UI language's faces first, so a Han character takes the face of the
    /// UI language's region (Simplified, Traditional, Hong Kong, Japanese or Korean forms), then
    /// the other offered languages' and the rest in manifest order. Letters of Latin, Greek and
    /// Cyrillic take faces such as Noto Sans, not the CJK faces, which carry those scripts at CJK
    /// proportions. Keyboard keys, arrows and other symbols take the symbol faces (`Zsym`),
    /// mathematical notation the math faces (`Zmth`) and then the symbol faces, and emoji the emoji
    /// faces. Empty for characters of no particular script and for characters no served face maps.
    pub fn downloads_for(&self, family: &FontRef, c: char) -> Vec<&ServedFace> {
        self.downloads(&family.family(), c.encode_utf8(&mut [0; 4]))
    }

    /// [`Self::downloads_for`] the first character of a cluster, whose variation selector chooses
    /// between emoji and symbol faces.
    pub(in crate::fonts) fn downloads(&self, family: &FontFamily, cluster: &str) -> Vec<&ServedFace> {
        let (Some(c), Some(script)) = (cluster.chars().next(), script_of_cluster(cluster)) else { return Vec::new() };
        let wanted = if script == MATH { [MATH, SYMBOLS].as_slice() } else { std::slice::from_ref(&script) };
        let alphabet = matches!(script, "Latn" | "Grek" | "Cyrl");
        let configured = self.definitions.families.get(family).or_else(|| self.definitions.families.get(&FontFamily::Proportional));
        let after = self.after_system.chain(&FontRef::from_family(family.clone()));
        let mut faces: Vec<&ServedFace> = Vec::new();
        for want in wanted {
            for name in configured.into_iter().flatten().chain(after) {
                if let Some(face) = self.served_face(name)
                    && covers(face.scripts, want)
                    && !(alphabet && covers(face.scripts, "Hani"))
                    && face.covers(c)
                    && !faces.iter().any(|known| known.name() == face.name())
                {
                    faces.push(face);
                }
            }
        }
        faces
    }

    /// Add the file served at `url` to every served face it holds, for [`Self::install`] to
    /// install. A face in a family of another weight is set to that weight, and renamed as
    /// [`super::UiFonts::stack`] would have. The loader does this for each file it fetches; an app
    /// that gets the files some other way can call it itself.
    pub fn provide(&mut self, url: &str, file: &Blob) {
        self.provide_to(file, |face| face.url == url);
    }

    /// [`Self::provide`] for bytes fetched and checked against `integrity`: only the faces that
    /// require that integrity (or none, when it is empty) take them.
    pub(in crate::fonts) fn provide_fetched(&mut self, url: &str, integrity: &str, file: &Blob) {
        self.provide_to(file, |face| face.url == url && face.integrity == integrity);
    }

    fn provide_to(&mut self, file: &Blob, takes: impl Fn(&ServedFace) -> bool) {
        let faces: Vec<ServedFace> = self.served.iter().filter(|face| takes(face)).copied().collect();
        for face in faces {
            let name = face.name();
            let data = baseline_fallback(egui::FontData::from_blob(Arc::clone(file), face.index));
            let weight = |family: &FontFamily| self.weights.iter().find(|(f, _)| f.family() == *family).map_or(400, |(_, w)| *w);
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
            if self.cjk_rows == CjkRows::Fit && EM_SQUARE_SCRIPTS.iter().any(|script| covers(face.scripts, script)) {
                self.fit_into_rows(&names);
            }
        }
    }

    fn served_face(&self, name: &str) -> Option<&ServedFace> {
        self.served.iter().find(|face| face.name() == name)
    }
}

/// Whether the UI language's served faces are still being fetched, for an app that waits for them
/// before drawing its interface. False without a fetcher, and once every fetch has finished,
/// failed ones included.
pub fn loading(ctx: &egui::Context) -> bool {
    loader::loading(ctx)
}
