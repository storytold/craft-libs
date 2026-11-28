//! Autosave and crash recovery: when to autosave ([`Autosave`]), where the copies go
//! ([`recovery_dir`]), and the folder of copies ([`RecoveryStore`]).
//!
//! The app serializes its documents; this module decides when, writes the bytes with a small
//! JSON sidecar, removes a copy when its document is saved or closed, and lists the copies an
//! earlier run left behind. The recover-on-launch question is
//! `craft_ui::dialog::RecoveryDialog` (or none: PhotoCraft reopens them without asking).

mod recovery;
mod timer;

pub use recovery::*;
pub use timer::*;

/// Moving a legacy recovery folder into the new one, without ever replacing anything.
#[cfg(not(target_arch = "wasm32"))]
mod migrate;
