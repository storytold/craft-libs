#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Hosts a Craft app in a native iPadOS or Android view.
//!
//! The native shell (`SwiftUI` with `CraftKit`, Compose with `craftkit`) owns the app, its windows,
//! its panels and the frame clock. This crate owns egui and wgpu, draws the app into the shell's
//! view, and bridges everything else:
//! - **Files:** the app asks for files to open or bytes to save through [`Platform`]; the shell
//!   answers with the system document picker and exporter. Files the system hands over (Files,
//!   Share, drag and drop) arrive through [`CraftApp::open_file`]. A file the shell may write
//!   back to comes with a [`Location`]; saving to its [`PickedFile::document_path`] writes the
//!   original in place, without a sheet, and [`CraftApp::written`] hears how it went.
//! - **Commands:** native controls send an [`Action`]; the app maps it to its own commands.
//! - **Panel state:** the app describes what native panels show as a [`Snapshot`]; the shell gets
//!   it again whenever it changes.
//!
//! An app implements [`CraftApp`] and calls [`export!`] once.

// Only the native shells (iOS, Android) drive the host; elsewhere just the unit tests build it.
#![cfg_attr(not(any(target_os = "ios", target_os = "android")), expect(dead_code, reason = "the host is driven by the iOS and Android shells only"))]

#[cfg(any(target_os = "android", test))]
mod driver;
pub mod host;
pub mod input;
#[cfg(any(target_os = "android", test))]
mod keycodes;
pub mod platform;
mod pointer;
mod session;
pub mod snapshot;

#[cfg(target_os = "android")]
#[doc(hidden)]
pub mod android;
#[cfg(target_os = "ios")]
#[doc(hidden)]
pub mod ios;

pub use eframe;
pub use egui;
pub use host::{HostError, Touch, ViewSize};
pub use input::{Input, Power, Stylus, Thermal};
pub use platform::{Location, Pasted, PickReply, PickedFile, Platform, SaveRoute, WriteOutcome, with_file_names};
pub use snapshot::{DocumentInfo, Item, Snapshot};

/// A Craft app as the native shells host it.
///
/// The frame itself is [`eframe::App`]'s: `raw_input_hook`, then `logic` and `ui` in one pass.
pub trait CraftApp: eframe::App + 'static {
    /// Build the app on `ctx`. `wgpu` is the shell's GPU when it came up; `platform` is how the
    /// app reaches the shell (file pickers, exports): keep it.
    fn create(ctx: &egui::Context, wgpu: Option<eframe::egui_wgpu::RenderState>, platform: Platform) -> Self
    where
        Self: Sized;

    /// How to set up wgpu (adapter limits, backends). The default is egui-wgpu's.
    #[must_use]
    fn wgpu_configuration() -> eframe::egui_wgpu::WgpuConfiguration
    where
        Self: Sized,
    {
        eframe::egui_wgpu::WgpuConfiguration::default()
    }

    /// A file the system handed over: opened from Files, shared to the app, or dropped on it.
    /// With [`PickedFile::document_path`] it can be saved back where it came from.
    fn open_file(&mut self, ctx: &egui::Context, file: PickedFile);

    /// A write back to a document's own file ([`Platform::save`]) ended. Mark the document
    /// saved only now, and only at the revision that was written; on `Err` the shell has shown
    /// the reason and offered the exporter instead.
    fn written(&mut self, ctx: &egui::Context, outcome: &WriteOutcome) {
        let _ = ctx;
        if let Err(e) = &outcome.result {
            log::warn!("craft-mobile: couldn't write {}: {e}", outcome.path);
        }
    }

    /// A native control was used. Runs inside the frame, before `logic`.
    ///
    /// # Errors
    /// Why the action could not run (the shell may show it).
    fn action(&mut self, ctx: &egui::Context, action: &Action) -> Result<(), String>;

    /// What the native panels show. Called after every frame; the shell hears of it when it
    /// changes, so keep it cheap and deterministic.
    fn snapshot(&self) -> Snapshot;

    /// The app is leaving the screen and the OS may terminate it without warning: start writing
    /// recovery data for unsaved work now, on workers, and return at once. The shells call this
    /// on the thread the OS watches (iPadOS ends apps whose main thread hangs; Android's `onStop`
    /// waits for it): report the writes through [`busy`](Self::busy) until they are on disk,
    /// and the shells keep the app running in the background meanwhile.
    fn suspend(&mut self, ctx: &egui::Context) {
        let _ = ctx;
    }

    /// The OS is short of memory: drop caches and anything else that can be rebuilt, but never
    /// the user's work or undo history.
    fn trim_memory(&mut self, ctx: &egui::Context) {
        let _ = ctx;
    }

    /// Whether the app is doing work the user started that must finish even if the app leaves
    /// the screen: encoding a save or an export on a worker, or writing the recovery data
    /// [`suspend`](Self::suspend) started, say. Checked after every frame and after `suspend`; while
    /// it is `true` the shells keep the app running in the background (an iPadOS background task,
    /// an Android short foreground service) and keep running frames, without drawing, so the app
    /// can finish and hand over the result. Keep it cheap, and `false` whenever nothing the user
    /// is waiting for is running: the OS gives this time sparingly.
    fn busy(&self) -> bool {
        false
    }

    /// The device's thermal state or power saving changed; also called once as the app starts.
    /// Lower what costs the most per frame (preview quality, GPU budgets) while
    /// [`Power::constrained`]; the shells already lower the frame rate ([`Power::max_frame_rate`]).
    fn power(&mut self, ctx: &egui::Context, power: Power) {
        let _ = (ctx, power);
    }

    /// A stylus sample (pressure, tilt), before the same contact reaches egui as a touch.
    /// Painting apps feed their brush engine from here.
    fn stylus(&mut self, sample: &Stylus) {
        let _ = sample;
    }
}

/// What a native control asks of the app.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    /// Open documents (the app's own File › Open, which asks the shell for a picker).
    Open,
    /// Save the active document.
    Save,
    /// Undo the last step.
    Undo,
    /// Redo the last undone step.
    Redo,
    /// Make this item (layer, track, page) the active one.
    Select {
        /// The item, by the id its [`Item`] carries.
        id: u64,
    },
    /// Show or hide this item.
    SetVisible {
        /// The item, by the id its [`Item`] carries.
        id: u64,
        /// Shown (`true`) or hidden.
        visible: bool,
    },
    /// Anything else: one of the app's own commands by id.
    Command {
        /// The command id, as the app's menus use it.
        id: String,
        /// Its parameters; `null` runs it as its menu item does (dialogs included).
        #[serde(default)]
        params: serde_json::Value,
    },
}

/// Export an app's entry point for the native shells: `craft_start` for iOS, and
/// `dev.craft.kit.CraftNative.start` for Android. Use it once, in the app's mobile crate, with
/// its [`CraftApp`] type.
///
/// Everything else the shells call lives in this crate; only the start is per app, because it
/// builds the app's own type.
#[macro_export]
macro_rules! export {
    ($app:ty) => {
        /// Start the app drawing into `layer`, a `CAMetalLayer` that must outlive `craft_stop`.
        ///
        /// # Safety
        /// `layer` must be a valid `CAMetalLayer` that stays alive until `craft_stop`.
        #[cfg(target_os = "ios")]
        // `allow`, not `expect`: whether `unsafe_code` fires inside an external macro's
        // expansion depends on the compiler's external-macro rules.
        #[allow(unsafe_code, reason = "the one exported symbol per app; the unsafe work is in craft-mobile")]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn craft_start(layer: *mut ::core::ffi::c_void, width_px: u32, height_px: u32, scale: f32) -> bool {
            // SAFETY: the shell's contract (CraftKit) is this function's contract.
            unsafe { $crate::ios::start::<$app>(layer, width_px, height_px, scale) }
        }

        /// `CraftNative.start(surface, widthPx, heightPx, scale)`: start the app drawing into
        /// `surface`, or move the running app there. Returns at once; the render thread does the work.
        #[cfg(target_os = "android")]
        // `allow`, not `expect`: as for `craft_start`.
        #[allow(unsafe_code, reason = "the one exported symbol per app; the unsafe work is in craft-mobile")]
        #[allow(non_snake_case, reason = "JNI names the symbol after the Kotlin class")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_craft_kit_CraftNative_start<'local>(
            env: $crate::android::jni::JNIEnv<'local>,
            _class: $crate::android::jni::objects::JClass<'local>,
            surface: $crate::android::jni::objects::JObject<'local>,
            width_px: $crate::android::jni::sys::jint,
            height_px: $crate::android::jni::sys::jint,
            scale: $crate::android::jni::sys::jfloat,
        ) -> $crate::android::jni::sys::jboolean {
            $crate::android::start::<$app>(&env, &surface, width_px, height_px, scale)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::Action;

    #[test]
    fn actions_arrive_as_tagged_json() {
        let parse = |s: &str| serde_json::from_str::<Action>(s).ok();
        assert_eq!(parse(r#"{"action":"undo"}"#), Some(Action::Undo));
        assert_eq!(parse(r#"{"action":"set_visible","id":7,"visible":false}"#), Some(Action::SetVisible { id: 7, visible: false }));
        assert_eq!(
            parse(r#"{"action":"command","id":"image.flip"}"#),
            Some(Action::Command { id: "image.flip".into(), params: serde_json::Value::Null })
        );
        assert_eq!(parse(r#"{"action":"format_disk"}"#), None, "an unknown action is rejected, not guessed");
    }
}
