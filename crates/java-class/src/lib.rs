#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]
//! Java class-file inspection and JVM bytecode decoding. Never executes bytecode.
//! Retains source item, attribute and annotation-depth limits and partial reports.
mod bytes;
mod descriptor;
pub mod java_class;
pub mod java_ops;
pub use java_class::*;
#[cfg(test)]
mod test_support;
/// File admission errors; damaged optional structures retain partial-report warnings.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JavaClassError {
    #[error("the file ends early ({0})")]
    Truncated(String),
    #[error("the file is malformed ({0})")]
    Malformed(String),
    #[error("not supported: {0}")]
    Unsupported(String),
}
use JavaClassError as FormatError;

#[cfg(test)]
#[path="regressions/java_array_operands.rs"]
mod java_array_operands;
