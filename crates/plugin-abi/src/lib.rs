#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! craft-abi 1 package types and the policy gate.
//!
//! - [`manifest`]: the `craft.manifest` section.
//! - [`ops`] and [`permissions`]: host call numbers, status codes, permission names.
//! - [`descriptor`]: the `craft_process` descriptor.
//! - [`mod@format`]: the request and response shapes of a `format` contribution.
//! - [`gate`]: the install-time policy check on a `.wasm` package.

pub mod descriptor;
pub mod format;
pub mod gate;
pub mod manifest;
pub mod ops;
pub mod permissions;
pub mod text;

pub use descriptor::{DescriptorError, ProcessDescriptor};
pub use gate::{HostPolicy, LegacyAbi, PolicyReport, Violation, check_package};
pub use manifest::{Manifest, ManifestError};
pub use ops::{Op, Status};
pub use permissions::{Permission, PermissionKind};
