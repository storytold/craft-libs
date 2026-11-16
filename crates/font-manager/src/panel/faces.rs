//! Drawing text in a font face itself: each face shown becomes an egui font family of its own.
//!
//! Adding a font makes egui rebuild its fonts, so faces are added a few per frame, and once more
//! than [`MAX_REGISTERED`] are loaded the ones not drawn in the last frame are taken out again,
//! which keeps memory to the faces on screen. Faces still on screen stay, however many there are,
//! so a full screen doesn't load its faces again every frame. Only the panel's own faces are
//! touched: the host's fonts stay as they are.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::{Catalog, FaceInfo};
use egui::epaint::text::{Blob, FontInsert, FontPriority, InsertFontFamily, VariationCoords};
use egui::{Context, FontData, FontFamily, FontTweak};

const MAX_REGISTERED: usize = 64;
const PER_FRAME: usize = 6;
/// Larger files are not drawn: a whole CJK family can be hundreds of megabytes.
const MAX_FILE_BYTES: u64 = 48 << 20;

/// Whose catalog a face belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// The installed fonts.
    Installed,
    /// Files downloaded for a preview.
    Preview,
}

pub enum Shown {
    Ready(FontFamily),
    Loading,
    Unavailable,
}

#[derive(Default)]
pub struct FaceFonts {
    /// Each face's family and the frame it was last drawn in.
    registered: HashMap<String, (FontFamily, u64)>,
    failed: HashSet<String>,
    budget: usize,
    frame: u64,
}

fn key(owner: Owner, face: &FaceInfo) -> String {
    let file = match &face.path {
        Some(path) => path.display().to_string(),
        None => face.postscript_name.clone().or_else(|| face.full_name.clone()).unwrap_or_else(|| face.family.clone()),
    };
    let coords: String = face.variations.iter().map(|(tag, v)| format!(" {}={v}", String::from_utf8_lossy(tag))).collect();
    format!("face:{owner:?}:{file}#{}{coords}", face.index)
}

impl FaceFonts {
    pub fn begin_frame(&mut self, ctx: &Context) {
        self.budget = PER_FRAME;
        let stale = self.next_frame();
        if !stale.is_empty() {
            let mut definitions = ctx.fonts(|f| f.definitions().clone());
            definitions.font_data.retain(|name, _| !stale.contains_key(name));
            definitions.families.retain(|family, _| !stale.values().any(|f| f == family));
            ctx.set_fonts(definitions);
        }
    }

    /// Starts a frame, and once more than [`MAX_REGISTERED`] faces are registered takes out the
    /// ones the last frame didn't draw, returning them.
    fn next_frame(&mut self) -> HashMap<String, FontFamily> {
        self.frame += 1;
        if self.registered.len() <= MAX_REGISTERED {
            return HashMap::new();
        }
        let last = self.frame - 1;
        let stale: Vec<String> = self.registered.iter().filter(|(_, (_, used))| *used < last).map(|(k, _)| k.clone()).collect();
        stale.into_iter().filter_map(|k| self.registered.remove(&k).map(|(family, _)| (k, family))).collect()
    }

    /// The family that draws `face`, once egui has loaded it.
    pub fn family(&mut self, ctx: &Context, catalog: &Catalog, owner: Owner, face: &FaceInfo) -> Shown {
        let key = key(owner, face);
        if self.failed.contains(&key) {
            return Shown::Unavailable;
        }
        if let Some((family, used)) = self.registered.get_mut(&key) {
            *used = self.frame;
            let loaded = ctx.fonts(|f| f.definitions().families.contains_key(family));
            if loaded {
                return Shown::Ready(family.clone());
            }
            ctx.request_repaint();
            return Shown::Loading;
        }
        if self.budget == 0 {
            ctx.request_repaint();
            return Shown::Loading;
        }
        self.budget -= 1;
        let too_large = face.path.as_ref().and_then(|p| std::fs::metadata(p).ok()).is_some_and(|m| m.len() > MAX_FILE_BYTES);
        let data = if too_large { None } else { catalog.load(face).ok() };
        let Some(data) = data else {
            self.failed.insert(key);
            return Shown::Unavailable;
        };
        let blob: Blob = Arc::new(data.bytes);
        let tweak = FontTweak { coords: VariationCoords::new(face.variations.iter().map(|(tag, v)| (*tag, *v))), ..Default::default() };
        let family = FontFamily::Name(key.as_str().into());
        let insert = InsertFontFamily { family: family.clone(), priority: FontPriority::Highest };
        ctx.add_font(FontInsert::new(&key, FontData::from_blob(blob, data.index).tweak(tweak), vec![insert]));
        self.registered.insert(key, (family, self.frame));
        ctx.request_repaint();
        Shown::Loading
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_faces_off_screen_are_taken_out() {
        let mut faces = FaceFonts::default();
        let register = |faces: &mut FaceFonts, range: std::ops::Range<usize>| {
            for n in range {
                faces.registered.insert(format!("face:{n}"), (FontFamily::Name(format!("face:{n}").into()), faces.frame));
            }
        };
        faces.next_frame();
        register(&mut faces, 0..MAX_REGISTERED);
        assert!(faces.next_frame().is_empty());
        // A screen of more faces than the limit keeps them all, frame after frame.
        register(&mut faces, 0..MAX_REGISTERED + 10);
        assert!(faces.next_frame().is_empty());
        register(&mut faces, 0..MAX_REGISTERED + 10);
        assert!(faces.next_frame().is_empty());
        // Scrolled on: the faces the last frame didn't draw go, and only those.
        register(&mut faces, 20..MAX_REGISTERED + 20);
        let stale = faces.next_frame();
        assert_eq!(stale.len(), 20);
        assert!(stale.keys().all(|k| k[5..].parse::<usize>().unwrap() < 20));
        assert_eq!(faces.registered.len(), MAX_REGISTERED);
    }
}
