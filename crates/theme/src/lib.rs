#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! The Craft suite's theme contract: theme identity, 13 core colour roles per light and dark
//! variant, derived interaction states, contrast checks, the built-in themes and resolution of a
//! selection against what the OS reports.
//!
//! Pure data and functions: no egui, no I/O, no threads. Theme files, the suite folder and change
//! watching belong to `craft-theme-store`; `craft_ui::theme` turns colours into egui ones.
//!
//! ```
//! use craft_theme::{Mode, Resolution, Selection, ThemeId, Variant, lookup_builtin, resolve};
//!
//! let sel = Selection { theme: ThemeId::parse("craft.graphite").unwrap(), mode: Mode::System };
//! // The OS says dark; the app falls back to light when the OS says nothing.
//! match resolve(&sel, Some(Variant::Dark), Variant::Light, &lookup_builtin) {
//!     Resolution::Theme(r) => assert_eq!(r.roles.text.to_hex(), "#ececef"),
//!     Resolution::AppDefault { .. } => unreachable!(),
//! }
//! ```

mod builtins;
mod color;
pub mod contrast;
mod derive;
mod id;
mod resolve;
mod roles;
mod theme;

pub use builtins::{builtin, builtins, lookup_builtin};
pub use color::Rgba;
pub use derive::{Derived, derive};
pub use id::{APP_DEFAULT, IdError, RESERVED_PREFIX, SUITE_DEFAULT, ThemeId};
pub use resolve::{AppChoice, AppDefaults, Fallback, Resolution, Resolved, Selection, resolve};
pub use roles::{CoreRoles, Mode, Role, Variant};
pub use theme::{DefinitionError, MissingRole, Origin, RoleMap, SuiteTheme};
