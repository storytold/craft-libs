#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Small, reusable egui components for the Craft frontends.
//!
//! Applications own their assets, themes, layouts, commands and document semantics. Components
//! accept focused style values and return egui responses; no application engine is required.

pub mod backdrop;
pub mod buttons;
pub mod capture;
pub mod combo;
pub mod direction;
pub mod docking;
mod focus;
pub mod fonts;
pub mod icons;
pub mod ime;
pub mod layout;
pub mod localized;
pub mod panes;
pub mod printing;
pub mod shapes;
pub mod tabs;
#[cfg(all(feature = "screen-testing", not(target_arch = "wasm32")))]
pub mod testing;
pub mod text;
pub mod theme;

pub use combo::ComboBox;

/// Lets one unit test at a time hold a wgpu renderer. Each renderer creates its own Vulkan instance
/// and device. When one test uses its device while a test on another thread creates or destroys
/// one, the Vulkan loader now and then crashes the test binary with SIGSEGV. Take the lock before
/// building the harness (`let _gpu = crate::gpu_test_lock();`), so it is released after the harness
/// is dropped. A test that failed while holding it does not block the others.
#[cfg(test)]
pub(crate) fn gpu_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
