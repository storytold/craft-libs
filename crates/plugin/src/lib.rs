#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! One WebAssembly plug-in runtime for the Craft apps (RFC-0046).
//!
//! - [`sandbox`](crate::Instance): modules run in a metered interpreter (wasmi, no JIT) with a
//!   memory cap, no WASI and a handful of host functions, imported from `craft` (FigCraft's older
//!   `figcraft` works too). The app implements [`Host`].
//! - [`manifest`]: `craft-plugin.json` (and FigCraft's `figcraft-plugin.json`): id, kind, the apps
//!   it targets, commands and their parameters, permissions, panels. [`Manifest::target`] says
//!   whether an app ([`AppInfo`], with an API range per kind) can run it.
//! - [`permissions`]: the allowlist, the refused list every app shares and the app's own.
//! - [`bundle`]: the `.craftplugin` file, its SHA-256 and version comparison.
//! - [`source`]: where an installed plug-in came from ([`Source`]) and its record, on every target.
//! - [`reviewed`]: a bundle checked for the app and held until the person agrees ([`Reviewed`]),
//!   plain data a worker thread hands to the UI.
//! - [`store`] (native builds): the per-user `Plug-ins` folder. Apps install and list through
//!   `Store::install_for` and `Store::list_for`, which check the target for them.
//! - [`catalog`] and [`github`]: catalogs (`index.json`) and installing from a repository's
//!   releases, as pure functions; [`fetch`] runs them over the app's HTTP client.
//! - [`widget`]: FigCraft's widget descriptions, which share the ABI.
//!
//! What stays in each app: plug-in kinds that touch its own model, its refused commands, running a
//! plug-in as one undo step and rolling back on failure, and its menus and windows.

pub mod bundle;
pub mod catalog;
mod color;
pub mod fetch;
pub mod github;
pub mod manifest;
pub mod permissions;
pub mod reviewed;
pub mod sandbox;
pub mod source;
#[cfg(not(target_arch = "wasm32"))]
pub mod store;
mod target;
pub mod widget;

pub use bundle::{BUNDLE_EXTENSION, Bundle, MAX_BUNDLE_BYTES, compare_versions, newer, parse_version, sha256_hex};
pub use color::normalize_hex;
pub use fetch::Fetch;
pub use manifest::{
    API_VERSION, Access, AppTarget, KIND_COMMAND, LEGACY_MANIFEST_FILE, MANIFEST_FILE, MANIFEST_FILES, Manifest, Panel, Param, ParamKind,
    Permissions, PluginCommand,
};
pub use permissions::{ALWAYS_DENIED, Policy, check_call, check_read_only};
pub use reviewed::Reviewed;
pub use sandbox::{
    ENTRY, Host, IMPORT_MODULE, IMPORT_MODULES, Instance, LEGACY_ENTRY, LEGACY_IMPORT_MODULE, Limits, PluginError, RunOutput, UiChange, check_module,
    run,
};
pub use source::{Source, SourceRecord};
pub use target::{AppInfo, KindApi, Target};
pub use widget::{MenuItem, MenuKind, WidgetHover, WidgetKind, WidgetNode, WidgetRender};

#[cfg(test)]
mod abi_names_tests;
#[cfg(test)]
mod bundle_tests;
#[cfg(test)]
mod catalog_tests;
#[cfg(test)]
mod fetch_tests;
#[cfg(test)]
mod github_tests;
#[cfg(test)]
mod manifest_tests;
#[cfg(test)]
mod permissions_tests;
#[cfg(test)]
mod reviewed_tests;
#[cfg(test)]
mod save_file_tests;
#[cfg(test)]
mod source_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod store_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod v2_tests;
#[cfg(test)]
mod widget_tests;
