//! craft-ai — AI provider abstraction for the Craft suite.
//!
//! See `README.md` for the design: BYO API provider (OpenAI-compatible) +
//! one-click local models with capability detection.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod error;
mod provider;
mod registry;
mod translate;

#[cfg(not(target_arch = "wasm32"))]
pub mod fetch;
#[cfg(not(target_arch = "wasm32"))]
#[cfg(feature = "local")]
pub mod local;
#[cfg(not(target_arch = "wasm32"))]
pub mod probe;

pub use error::AiError;
pub use provider::{GenOpts, OpenAiCompat, Provider, ProviderInfo};
pub use registry::{Arch, MODELS, ModelEntry, Quality, by_id, fits, suggest};
pub use translate::{PdfGrammar, Segment, TranslateGrammar, is_protected_token, segment, translate_segments};
