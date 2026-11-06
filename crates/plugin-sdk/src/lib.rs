#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Guest SDK for craft-abi 1 plugins.
//!
//! A plugin is an inline module of handler functions plus a `craft.toml`:
//!
//! ```ignore
//! use craft_plugin_sdk::prelude::*;
//!
//! #[craft_plugin]
//! mod plugin {
//!     use craft_plugin_sdk::prelude::*;
//!
//!     fn say(call: CommandCall) -> Result<Value, String> {
//!         host::info("hello");
//!         Ok(json!({ "said": call.params }))
//!     }
//! }
//! ```
//!
//! - [`craft_plugin`] reads `craft.toml`, checks it, embeds it, and exports the five ABI functions.
//! - [`host`], [`cmd`], [`state`], [`doc`], [`link`], [`clock`], [`random`] wrap `craft_host.call`.
//! - [`ui`] builds panel widget trees.
//! - [`entry`] holds the request and response types handlers use.
//! - [`testing`] (native only) installs a fake host so handlers run in a plain `cargo test`.
//!
//! Plugin code needs no `unsafe`. The glue that does is in [`backend`] and [`rt`], each behind an
//! explicit `#[allow(unsafe_code)]`.

pub mod backend;
pub mod entry;
pub mod error;
pub mod host;
pub mod rt;
#[cfg(not(target_arch = "wasm32"))]
pub mod testing;
pub mod ui;

pub use craft_plugin_abi as abi;
pub use craft_plugin_sdk_macros::craft_plugin;
pub use serde;
pub use serde_json::{self, Value, json};

pub use entry::{
    Band, CallInfo, CommandCall, CommandReturn, ControlFn, DataFn, FormatCall, FormatOp, FormatOutput, FormatReturn, HookCall, HookVerdict,
    ImageReturn, PanelEvent, PanelReturn,
};
pub use error::HostError;
pub use host::{OpError, clock, cmd, doc, link, random, state};

// Paths the generated code names, so a plugin crate needs no `use` for them.
#[doc(hidden)]
pub use std::format;
#[doc(hidden)]
pub use std::string::String;
#[doc(hidden)]
pub use std::vec::Vec;

/// One import for plugin code.
pub mod prelude {
    pub use crate::ui::{self, Widget};
    pub use crate::{
        Band, CommandCall, FormatCall, FormatOp, FormatOutput, HookCall, HookVerdict, HostError, PanelEvent, Value, clock, cmd, craft_plugin, doc,
        host, json, link, random, state,
    };
}
