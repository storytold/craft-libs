//! The OS's own interface face, for `craft_theme::InterfaceFont::System`.
//!
//! It is read from the installed fonts at run time: fontique's `system-ui` family (San Francisco
//! on macOS, Segoe UI on Windows, fontconfig's `system-ui` match on Linux, which falls back to
//! `sans-serif`), preferring Segoe UI Variable where Windows 11 has it.
//!
//! The face comes from the index of installed fonts the system font provider shares, which is
//! built on a background thread (see `index.rs`). The UI thread doesn't wait for it: a stack
//! built before the face resolves holds [`SYSTEM_UI_REGULAR`] and [`SYSTEM_UI_SEMIBOLD`] without
//! data, draws with the app's faces meanwhile, and is installed again with the face's data at the
//! start of the first pass after it resolves.

use std::task::Poll;

use egui::{Context, FontData};

use crate::runtime::egui::fonts as egui_fonts;

/// The optical size (`opsz`) the system face is set to, in points: the body type size. It holds
/// for text drawn from a bare font id, such as egui's own labels; the formats of type styles set
/// their own size instead. A face without an `opsz` axis ignores it; one whose axis starts above
/// it takes the axis minimum.
pub const SYSTEM_UI_OPTICAL_SIZE: f32 = 13.0;

/// The egui name of the system face's regular face, in the proportional family.
pub const SYSTEM_UI_REGULAR: &str = "system-ui";
/// The egui name of the system face's semibold face, in the semibold family.
pub const SYSTEM_UI_SEMIBOLD: &str = "system-ui:semibold";

/// The OS's interface face at regular and semibold weight.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SystemUiFace {
    /// The family's name, e.g. `DejaVu Sans` or `System Font`.
    pub family: String,
    /// The regular (400) face.
    pub regular: FontData,
    /// The semibold face: a variable face's `wght` 600 instance, else the family's lightest static
    /// face from 600 up (Bold when it has no SemiBold), else the regular face. Never emboldened.
    pub semibold: FontData,
    /// The weight [`Self::semibold`] draws at: 400 when it is the regular face.
    pub semibold_weight: u16,
}

impl SystemUiFace {
    /// A face from its parts.
    #[cfg(all(feature = "system_fonts", not(target_arch = "wasm32")))]
    pub(super) fn new(family: String, regular: FontData, semibold: FontData, semibold_weight: u16) -> Self {
        Self { family, regular, semibold, semibold_weight }
    }
}

#[cfg(all(feature = "system_fonts", not(target_arch = "wasm32")))]
static FACE: std::sync::OnceLock<Option<SystemUiFace>> = std::sync::OnceLock::new();
/// Contexts to repaint when the face resolves.
#[cfg(all(feature = "system_fonts", not(target_arch = "wasm32")))]
static WAITING: std::sync::Mutex<Vec<Context>> = std::sync::Mutex::new(Vec::new());

/// The OS's interface face, or `None` on the web, without the `system_fonts` feature, or when no
/// installed family resolves. Resolved once per process. Before the index of installed fonts is
/// built, this waits for it, so call it from checks, tests or a background thread; the UI thread
/// asks [`poll_system_ui_face`].
#[cfg(all(feature = "system_fonts", not(target_arch = "wasm32")))]
pub fn system_ui_face() -> Option<&'static SystemUiFace> {
    FACE.get_or_init(super::fontique::resolve_system_ui_face).as_ref()
}

/// The OS's interface face, or `None` on the web, without the `system_fonts` feature, or when no
/// installed family resolves.
#[cfg(not(all(feature = "system_fonts", not(target_arch = "wasm32"))))]
pub fn system_ui_face() -> Option<&'static SystemUiFace> {
    None
}

/// [`system_ui_face`] without waiting: pending while the index of installed fonts is built, in
/// the background from the first call on. `ctx` repaints when the face resolves, so an app can
/// offer the interface-font setting only where it takes effect, from the frame it is known.
pub fn poll_system_ui_face(ctx: &Context) -> Poll<Option<&'static SystemUiFace>> {
    if let Some(face) = resolved() {
        return Poll::Ready(face);
    }
    wait_for(ctx);
    resolved().map_or(Poll::Pending, Poll::Ready)
}

/// The face if it has resolved, without waiting or starting the index.
pub(crate) fn resolved() -> Option<Option<&'static SystemUiFace>> {
    #[cfg(all(feature = "system_fonts", not(target_arch = "wasm32")))]
    return FACE.get().map(Option::as_ref);
    #[cfg(not(all(feature = "system_fonts", not(target_arch = "wasm32"))))]
    Some(None)
}

/// Repaints `ctx` when the face resolves: now if it has, else from the index's built hook. Starts the index.
pub(crate) fn wake_on_resolve(ctx: &Context) {
    if poll_system_ui_face(ctx).is_ready() {
        egui_fonts::wake(ctx);
    }
}

#[cfg(all(feature = "system_fonts", not(target_arch = "wasm32")))]
fn wait_for(ctx: &Context) {
    super::fontique::start_building();
    let mut waiting = WAITING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // Checked under the lock that `index_built` takes after setting the face, so a context either
    // sees the face here or is repainted there.
    if FACE.get().is_none() && !waiting.contains(ctx) {
        waiting.push(ctx.clone());
    }
}

#[cfg(not(all(feature = "system_fonts", not(target_arch = "wasm32"))))]
fn wait_for(_ctx: &Context) {}

/// The index's built hook, on the thread that built it: resolves the face, then repaints the
/// contexts waiting for it.
#[cfg(all(feature = "system_fonts", not(target_arch = "wasm32")))]
pub(super) fn index_built() {
    system_ui_face();
    let waiting = std::mem::take(&mut *WAITING.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
    for ctx in waiting {
        egui_fonts::wake(&ctx);
    }
}

/// Sets `data`'s variation coordinates to `coords`, each clamped to its axis's range, plus the
/// optical size if the face has that axis. Axes the face lacks are skipped, and so is a tag that
/// comes again.
#[cfg(all(feature = "system_fonts", not(target_arch = "wasm32")))]
pub(super) fn instance(mut data: FontData, coords: impl IntoIterator<Item = ([u8; 4], f32)>) -> FontData {
    let axes = data.variation_axes();
    let opsz = (*b"opsz", SYSTEM_UI_OPTICAL_SIZE);
    for (tag, value) in coords.into_iter().chain([opsz]) {
        let tag_value = egui::epaint::text::Tag::new(&tag);
        if data.tweak.coords.as_ref().iter().any(|(t, _)| *t == tag_value) {
            continue;
        }
        if let Some(axis) = axes.iter().find(|axis| axis.tag == tag_value) {
            data.tweak.coords.push(tag, value.clamp(axis.range.min, axis.range.max));
        }
    }
    data
}
