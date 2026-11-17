//! The display transform (F04.11): how a canvas shows a document's colours on a monitor. Pixels
//! in the document's profile go through one cached transform to the monitor's profile; when that
//! transform changes nothing, the canvas skips it.
//!
//! The monitor profile comes from the platform (craft-platform's display reading, or the ICC
//! bytes of a `craft_platform::screen::ScreenImage`) or from the app's Color Settings; a
//! profile that is missing, unreadable, not RGB or unusable as a destination falls back to sRGB,
//! and [`MonitorStatus`] says why.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use craft_cms::transform::{TransformOptions, cached};
use craft_cms::{Builtin, CmsError, ColorSpace, Profile, Transform};

/// Rendering intent and black point compensation of every display transform: relative
/// colorimetric with BPC, as Photoshop's default monitor conversion and PhotoCraft's.
pub const DISPLAY_INTENT: craft_cms::Intent = craft_cms::Intent::RelativeColorimetric;

/// A display, as the platform numbers it; `None` is "the main window's display".
pub type DisplayId = Option<u32>;

/// Bytes per conversion chunk of [`DisplayTransform::apply_rgba8`] (a whole number of pixels).
const CHUNK_BYTES: usize = 4 * 65536;

/// Most cached transforms before the cache is cleared.
const MAX_TRANSFORMS: usize = 32;

fn hash_of(v: impl std::hash::Hash) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

/// The [`DisplayTransform::key`] of a source and monitor pair. Whether the transform is the
/// identity follows from the pair, so it is not part of the key.
fn signature(source: &Profile, monitor: &Profile) -> u64 {
    hash_of((source.content_hash(), monitor.content_hash()))
}

/// Does `t` map every lattice point (the neutral axis only for a grey source) to itself within
/// half an 8-bit step?
fn is_identity(t: &Transform, neutral_only: bool) -> bool {
    const N: usize = 9;
    let s = (N - 1) as f32;
    let mut out = [0.0f32; 16];
    let mut check = |v: [f32; 3]| match t.eval(&v, &mut out) {
        Ok(()) => (0..3).all(|i| (out[i] - v[i]).abs() <= 0.5 / 255.0),
        Err(_) => false,
    };
    if neutral_only {
        return (0..=32).all(|i| check([i as f32 / 32.0; 3]));
    }
    (0..N).all(|b| (0..N).all(|g| (0..N).all(|r| check([r as f32 / s, g as f32 / s, b as f32 / s]))))
}

/// `profile` as a monitor profile: it must be RGB and usable as the destination of a transform
/// from sRGB, otherwise the canvas would silently stay unmanaged.
fn usable_monitor(profile: Profile) -> Result<Arc<Profile>, MonitorProblem> {
    if profile.color_space != ColorSpace::Rgb {
        return Err(MonitorProblem::NotRgb { description: profile.description, space: profile.color_space });
    }
    let opts = TransformOptions { intent: DISPLAY_INTENT, bpc: true, ..TransformOptions::default() };
    cached(Builtin::Srgb.profile(), &profile, opts)
        .map_err(|e| MonitorProblem::CannotLink { description: profile.description.clone(), reason: e.to_string() })?;
    Ok(Arc::new(profile))
}

fn srgb_entry(status: MonitorStatus) -> MonitorEntry {
    MonitorEntry { profile: Arc::new(Builtin::Srgb.profile().clone()), status }
}

/// How one document profile shows on one monitor.
#[derive(Debug)]
pub struct DisplayTransform {
    /// The profile of the values the canvas texture holds (the document's composite profile).
    pub source: Arc<Profile>,
    /// The monitor profile.
    pub monitor: Arc<Profile>,
    /// Texture values to monitor values; `None` when that is the identity within half an 8-bit
    /// step on every lattice point (the neutral axis only for a grey source).
    pub transform: Option<Arc<Transform>>,
    /// Changes whenever any of the above changes, for the canvas's LUT and texture caches.
    pub key: u64,
}

impl DisplayTransform {
    /// The transform from `source` to `monitor` ([`DISPLAY_INTENT`], BPC), built through
    /// `craft_cms::cached`; `transform` is `None` when it is the identity.
    // pc: crates/engine/src/display_color.rs:413 `canvas_display_for` (the build part) and :151
    // `is_identity` (9³ lattice, half an 8-bit step; the 33 grey steps for a grey source).
    pub fn new(source: Arc<Profile>, monitor: Arc<Profile>) -> Result<DisplayTransform, CmsError> {
        let key = signature(&source, &monitor);
        let transform = if source.content_hash() == monitor.content_hash() {
            None
        } else {
            let opts = TransformOptions { intent: DISPLAY_INTENT, bpc: true, ..TransformOptions::default() };
            let t = cached(&source, &monitor, opts)?;
            (!is_identity(&t, source.color_space == ColorSpace::Gray)).then_some(t)
        };
        Ok(DisplayTransform { source, monitor, transform, key })
    }

    /// Values go to the screen unchanged.
    pub fn is_identity(&self) -> bool {
        self.transform.is_none()
    }

    /// `rgb` (0–1 each, in the source profile) as the monitor shows it.
    // pc: crates/engine/src/display_color.rs:38 `CanvasDisplay` (callers use `transform.eval`).
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let Some(t) = &self.transform else { return rgb };
        let mut out = [0.0f32; 16];
        match t.eval(&rgb, &mut out) {
            Ok(()) => [out[0], out[1], out[2]],
            Err(_) => rgb,
        }
    }

    /// RGBA8 pixels converted in place (alpha untouched); nothing happens for the identity.
    // new: the canvas's CPU path and the screen picker's loupe convert whole rows.
    pub fn apply_rgba8(&self, pixels: &mut [u8]) {
        let Some(t) = &self.transform else { return };
        // `convert_u8` reads and writes separate buffers: convert from a copy, a chunk at a time.
        let mut src = Vec::with_capacity(CHUNK_BYTES.min(pixels.len()));
        for chunk in pixels.chunks_mut(CHUNK_BYTES).map(|c| {
            let n = c.len() / 4 * 4;
            &mut c[..n]
        }) {
            src.clear();
            src.extend_from_slice(chunk);
            if t.convert_u8(&src, 4, chunk, 4, true).is_err() {
                chunk.copy_from_slice(&src);
            }
        }
    }
}

/// Why the monitor profile is what it is.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MonitorStatus {
    /// The display's own profile, as the platform reported it.
    Detected,
    /// The profile the app chose (Color Settings › Monitor Profile).
    Chosen,
    /// sRGB: the platform reports no profile for the display.
    NoProfile,
    /// sRGB: the profile could not be used, with the reason.
    Unusable(MonitorProblem),
}

/// Why a monitor profile cannot be used. The UI words it (`MonitorProblem::label`, with the
/// `ui` feature).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MonitorProblem {
    /// The profile describes another colour space than RGB.
    NotRgb {
        /// The profile's description text, for display.
        description: String,
        /// The colour space the profile declares.
        space: ColorSpace,
    },
    /// The profile cannot link to sRGB as a destination; `reason` is the colour engine's text.
    CannotLink {
        /// The profile's description text, for display.
        description: String,
        /// The colour engine's error text.
        reason: String,
    },
    /// The profile bytes do not parse; `reason` is the parser's text.
    Unreadable {
        /// The parser's error text.
        reason: String,
    },
}

/// One monitor's profile and how it was resolved.
#[derive(Clone, Debug)]
struct MonitorEntry {
    profile: Arc<Profile>,
    status: MonitorStatus,
}

/// The app's display caches: the monitor profile per display, and the [`DisplayTransform`] per
/// document profile and display. Shared by every window; `Sync`.
#[derive(Default)]
pub struct DisplayCaches {
    monitors: Mutex<HashMap<DisplayId, MonitorEntry>>,
    chosen: Mutex<Option<MonitorEntry>>,
    transforms: Mutex<HashMap<(u64, DisplayId), Arc<DisplayTransform>>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

const _: () = {
    const fn shared<T: Send + Sync>() {}
    shared::<DisplayCaches>();
};

impl DisplayCaches {
    /// Empty caches: no monitor profile and no transform yet.
    pub fn new() -> DisplayCaches {
        DisplayCaches::default()
    }

    /// Records the platform's reading of `display`'s profile (`icc` bytes, `None` when it has
    /// none). A reading that fails to parse keeps the previous good one. Drops the cached
    /// transforms of that display.
    // pc: crates/engine/src/display_color.rs:301 (`set_displays`: a failed re-read keeps the
    // displays read before) and the monitor cache at :268.
    pub fn set_monitor(&self, display: DisplayId, icc: Option<&[u8]>) {
        let entry = match icc {
            None => srgb_entry(MonitorStatus::NoProfile),
            Some(bytes) => match Profile::parse(bytes) {
                Ok(p) => match usable_monitor(p) {
                    Ok(profile) => MonitorEntry { profile, status: MonitorStatus::Detected },
                    Err(reason) => srgb_entry(MonitorStatus::Unusable(reason)),
                },
                Err(e) => {
                    if let std::collections::hash_map::Entry::Vacant(v) = lock(&self.monitors).entry(display) {
                        v.insert(srgb_entry(MonitorStatus::Unusable(MonitorProblem::Unreadable { reason: e.to_string() })));
                        lock(&self.transforms).clear();
                    }
                    return;
                }
            },
        };
        lock(&self.monitors).insert(display, entry);
        // The main window's display also stands in for unknown displays.
        lock(&self.transforms).clear();
    }

    /// The app's choice of monitor profile (Color Settings), or `None` to use each display's own.
    /// Drops every cached transform.
    // pc: crates/engine/src/display_color.rs:338 `resolved_monitor` (the `auto` / chosen branch).
    pub fn choose_monitor(&self, profile: Option<Arc<Profile>>) {
        let entry = profile.map(|p| match usable_monitor((*p).clone()) {
            Ok(profile) => MonitorEntry { profile, status: MonitorStatus::Chosen },
            Err(reason) => srgb_entry(MonitorStatus::Unusable(reason)),
        });
        *lock(&self.chosen) = entry;
        lock(&self.transforms).clear();
    }

    /// The monitor profile for `display`: the chosen one, else the display's own, else the
    /// primary display's when `display` is unknown, else sRGB.
    // pc: crates/engine/src/display_color.rs:284 `monitor_for`, :338 `resolved_monitor`.
    pub fn monitor_for(&self, display: DisplayId) -> Arc<Profile> {
        self.resolve(display).profile
    }

    /// What [`monitor_for`](Self::monitor_for) resolved to, and why.
    // pc: crates/engine/src/display_color.rs:294 `monitor_status_for`.
    pub fn monitor_status(&self, display: DisplayId) -> MonitorStatus {
        self.resolve(display).status
    }

    fn resolve(&self, display: DisplayId) -> MonitorEntry {
        if let Some(e) = lock(&self.chosen).clone() {
            return e;
        }
        let monitors = lock(&self.monitors);
        monitors.get(&display).or_else(|| monitors.get(&None)).cloned().unwrap_or_else(|| srgb_entry(MonitorStatus::NoProfile))
    }

    /// How a canvas in `source`'s profile shows on `display`, cached per source content and
    /// display.
    // pc: crates/engine/src/display_color.rs:413 `canvas_display_for`.
    pub fn canvas_display_for(&self, source: &Arc<Profile>, display: DisplayId) -> Result<Arc<DisplayTransform>, CmsError> {
        let monitor = self.monitor_for(display);
        let cache_key = (source.content_hash(), display);
        if let Some(d) = lock(&self.transforms).get(&cache_key)
            && d.monitor.content_hash() == monitor.content_hash()
        {
            return Ok(d.clone());
        }
        let d = Arc::new(DisplayTransform::new(source.clone(), monitor)?);
        let mut cache = lock(&self.transforms);
        if cache.len() >= MAX_TRANSFORMS {
            cache.clear();
        }
        cache.insert(cache_key, d.clone());
        Ok(d)
    }

    /// Changes whenever the transform of `source` on `display` changes (source, monitor, chosen
    /// profile): what a canvas compares to know its texture or LUT is stale.
    // pc: crates/engine/src/display_color.rs:453 `display_signature_for` (without Proof Colors,
    // Gamut Warning and the 32-bit preview: those are the app's and go into its own key).
    pub fn display_signature(&self, source: &Profile, display: DisplayId) -> u64 {
        signature(source, &self.monitor_for(display))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_ui::testing::trial;

    fn p3() -> Arc<Profile> {
        Arc::new(Builtin::DisplayP3.profile().clone())
    }

    fn srgb() -> Arc<Profile> {
        Arc::new(Builtin::Srgb.profile().clone())
    }

    /// Choosing another monitor profile changes the signature and drops the cached transform.
    #[trial]
    fn choosing_a_monitor_profile_changes_the_signature() {
        let caches = DisplayCaches::new();
        let source = srgb();
        let before = caches.display_signature(&source, None);
        let first = caches.canvas_display_for(&source, None).unwrap();
        assert!(Arc::ptr_eq(&first, &caches.canvas_display_for(&source, None).unwrap()));

        caches.choose_monitor(Some(p3()));
        assert_eq!(caches.monitor_status(None), MonitorStatus::Chosen);
        assert_ne!(caches.display_signature(&source, None), before);
        let second = caches.canvas_display_for(&source, None).unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(second.key, caches.display_signature(&source, None));

        caches.choose_monitor(None);
        assert_eq!(caches.display_signature(&source, None), before);
    }
}
