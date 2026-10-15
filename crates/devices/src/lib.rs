//! Device tools shared by the Craft apps. UI-free and platform-free: an app registers its platform
//! backends into `craft-scan` / `craft-print` and mounts these tools into its own CLI or tool list.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod scan_jobs;
