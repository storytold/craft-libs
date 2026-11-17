#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Craft plug-ins: the manifest and the WebAssembly sandbox a plug-in runs in, and the widget
//! descriptions a plug-in hands back. The app decides what a plug-in may do (its permissions,
//! which commands it may call, one undo step per run, panel or widget callback, where settings are
//! kept); this crate only runs modules safely: a metered interpreter with a memory cap, no WASI and
//! a handful of host functions.

pub mod bundle;
mod color;
pub mod manifest;
mod sandbox;
pub mod widget;

pub use manifest::{API_VERSION, Access, MANIFEST_FILE, Manifest, Panel, Param, ParamKind, Permissions, PluginCommand};
pub use sandbox::{
    ENTRY, Host, IMPORT_MODULE, IMPORT_MODULES, Instance, LEGACY_ENTRY, LEGACY_IMPORT_MODULE, Limits, PluginError, RunOutput, UiChange, check_module,
    run,
};
pub use widget::{MenuItem, MenuKind, WidgetHover, WidgetKind, WidgetNode, WidgetRender};

#[cfg(test)]
mod abi_names_tests;
#[cfg(test)]
mod bundle_tests;
#[cfg(test)]
mod save_file_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod v2_tests;
#[cfg(test)]
mod widget_tests;
