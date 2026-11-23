//! Drawing text in a font face itself: each face shown (installed, a downloaded preview file, or
//! a subset from the font preview service) becomes an egui font family of its own.
//!
//! Adding a font makes egui rebuild its fonts, so faces are added a few per frame, and once more
//! than [`MAX_REGISTERED`] are loaded the ones not drawn in the last frame are taken out again,
//! which keeps memory to the faces on screen. Faces still on screen stay, however many there are,
//! so a full screen doesn't load its faces again every frame. Only the panel's own faces are
//! touched: the host's fonts stay as they are.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::remote::PreviewKey;
use crate::{Catalog, FaceInfo};
use egui::epaint::text::{Blob, FontInsert, FontPriority, InsertFontFamily, VariationCoords};
use egui::{Context, FontData, FontFamily, FontTweak};

const MAX_REGISTERED: usize = 64;
/// A frame that adds faces rebuilds every registered font once, however many it adds; the cost
/// grows with the bytes registered (`add_font_cost`: about 4.5 ms with 64 service subsets, 90 ms
/// with 64 whole files of Hack's size, in a release build).
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
        self.show(ctx, key(owner, face), || {
            let too_large = face.path.as_ref().and_then(|p| std::fs::metadata(p).ok()).is_some_and(|m| m.len() > MAX_FILE_BYTES);
            let data = if too_large { None } else { catalog.load(face).ok() }?;
            let blob: Blob = Arc::new(data.bytes);
            let tweak = FontTweak { coords: VariationCoords::new(face.variations.iter().map(|(tag, v)| (*tag, *v))), ..Default::default() };
            Some(FontData::from_blob(blob, data.index).tweak(tweak))
        })
    }

    /// The family that draws a subset from the preview service, the decoded `font` of `subset`.
    pub fn subset(&mut self, ctx: &Context, subset: &PreviewKey, font: &Arc<Vec<u8>>) -> Shown {
        let key = format!("face:{:?}:{}", Owner::Preview, subset.path());
        self.show(ctx, key, || Some(FontData::from_blob(Arc::clone(font) as Blob, 0)))
    }

    /// The family registered as `key`; `load` gives its font the first time, within the frame's
    /// budget.
    fn show(&mut self, ctx: &Context, key: String, load: impl FnOnce() -> Option<FontData>) -> Shown {
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
        let Some(data) = load() else {
            self.failed.insert(key);
            return Shown::Unavailable;
        };
        let family = FontFamily::Name(key.as_str().into());
        let insert = InsertFontFamily { family: family.clone(), priority: FontPriority::Highest };
        ctx.add_font(FontInsert::new(&key, data, vec![insert]));
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

    /// What registering faces costs egui with [`MAX_REGISTERED`] faces on screen, on the CPU:
    /// the fonts rebuilt, the text laid out and its glyphs rasterized again (not the atlas
    /// upload). `cargo test -p craft-font-manager --features egui --release --lib add_font_cost -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn add_font_cost() {
        use std::time::{Duration, Instant};
        let tiny = wuff::decompress_woff2(include_bytes!("../../tests/data/Tiny-glyf.woff2")).unwrap();
        let median = |mut v: Vec<Duration>| {
            v.sort();
            v[v.len() / 2]
        };
        for (what, bytes) in [("Hack, 309 KB", epaint_default_fonts::HACK_REGULAR.to_vec()), ("a 1 KB subset", tiny)] {
            let ctx = Context::default();
            crate::install_interface_fonts(&ctx, &[]);
            let blob: Blob = Arc::new(bytes);
            let frame = |names: &[String]| {
                let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                    for name in names {
                        let family = FontFamily::Name(name.as_str().into());
                        ui.label(egui::RichText::new("Sphinx of black quartz, judge my vow").size(36.0).family(family));
                    }
                });
            };
            let add = |names: &mut Vec<String>, n: usize| {
                for _ in 0..n {
                    let name = format!("face:{}", names.len());
                    let insert = InsertFontFamily { family: FontFamily::Name(name.as_str().into()), priority: FontPriority::Highest };
                    ctx.add_font(FontInsert::new(&name, FontData::from_blob(Arc::clone(&blob), 0), vec![insert]));
                    names.push(name);
                }
            };
            let keep = |names: &mut Vec<String>, n: usize| {
                names.truncate(n);
                let mut definitions = ctx.fonts(|f| f.definitions().clone());
                definitions.font_data.retain(|name, _| !name.starts_with("face:") || names.contains(name));
                definitions
                    .families
                    .retain(|family, _| !matches!(family, FontFamily::Name(n) if n.starts_with("face:") && !names.iter().any(|x| x == &**n)));
                ctx.set_fonts(definitions);
                frame(names);
            };
            let mut names = Vec::new();
            add(&mut names, MAX_REGISTERED);
            frame(&names);
            let timed = |f: &mut dyn FnMut()| {
                let start = Instant::now();
                f();
                start.elapsed()
            };
            let steady = median((0..9).map(|_| timed(&mut || frame(&names))).collect());
            let mut by_count = Vec::new();
            for n in [1, PER_FRAME] {
                let times = (0..9)
                    .map(|_| {
                        keep(&mut names, MAX_REGISTERED - n);
                        timed(&mut || {
                            add(&mut names, n);
                            frame(&names);
                        })
                    })
                    .collect();
                by_count.push((n, median(times)));
            }
            println!("{what}, {MAX_REGISTERED} faces drawn: frame {steady:?}; frame adding n faces: {by_count:?}");
        }
    }
}
