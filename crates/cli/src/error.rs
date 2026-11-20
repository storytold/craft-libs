//! One error type for the CLI. The message is what the developer reads, so each constructor
//! says what failed and, where it can, what to do.

use std::fmt;

#[derive(Debug)]
pub struct CliError(pub String);

pub type Result<T> = std::result::Result<T, CliError>;

impl CliError {
    pub fn new(message: impl Into<String>) -> CliError {
        CliError(message.into())
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CliError {}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError(e.to_string())
    }
}

impl From<serde_json::Error> for CliError {
    fn from(e: serde_json::Error) -> Self {
        CliError(format!("JSON: {e}"))
    }
}

/// `io::Error` with the path it concerned.
pub fn io_at(path: &std::path::Path, e: std::io::Error) -> CliError {
    CliError(format!("{}: {e}", path.display()))
}
