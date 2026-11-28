#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]
//! Dalvik DEX inspection and bytecode decoding. Never executes bytecode.
mod bytes;
/// Checked byte reader accepted by the signed-LEB compatibility helper.
pub use bytes::Reader;
pub mod dex;
pub mod dex_ops;
pub use dex::*;
#[cfg(test)]
mod test_support;
/// Parser admission failures; damaged substructures retain the original partial-report behavior.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DexError {
    #[error("the file ends early ({0})")]
    Truncated(String),
    #[error("the file is malformed ({0})")]
    Malformed(String),
    #[error("not supported: {0}")]
    Unsupported(String),
}
use DexError as FormatError;

#[cfg(test)]
#[path = "regressions/array_widths.rs"]
mod array_widths;

#[cfg(test)]
#[path = "regressions/capped_table_boundaries.rs"]
mod capped_table_boundaries;

#[cfg(test)]
#[path = "regressions/invoke_registers.rs"]
mod invoke_registers;

#[cfg(test)]
#[path = "regressions/payload_budgets.rs"]
mod payload_budgets;

#[cfg(test)]
#[path = "regressions/signed_leb128.rs"]
mod signed_leb128;
