//! Speech recognition shared by the Craft apps.
//!
//! The crate has no platform or model dependency by default:
//!
//! - [`provider`]: the [`Recognizer`] protocol a speech engine implements (`start`, `feed`,
//!   `finish`, `poll`, `status`), partial and final [`Segment`]s with bounded text, and a
//!   deterministic [`FakeRecognizer`] for hosts without a microphone and for tests.
//! - [`audio`]: bounded microphone buffers, a channel downmix and linear resampling to the
//!   16 kHz mono speech models take. Sample rates, channel counts and lengths are validated
//!   before anything is converted or allocated.
//! - [`manifest`]: a model manifest (id, revision, files, sizes, SHA-256, licence and source
//!   URL), verification that refuses a wrong size or digest, the directory states a model can
//!   be in, and the [`whisper_tiny`] manifest. Model weights are never shipped.
//! - [`consent`]: the disclosure shown before a download and the state machine behind an
//!   explicit accept. [`Consent::permits`] is false until the user accepted *that* manifest.
//! - [`download`]: the verified download loop. It refuses to run without consent, writes every
//!   file beside its final name, checks size and SHA-256, then renames it; a caller-supplied
//!   [`Transport`] keeps tests off the network.
//!
//! Optional features:
//!
//! - `candle` adds `WhisperEngine` and `WhisperRecognizer`: Whisper inference through
//!   [candle](https://github.com/huggingface/candle) on the CPU, following the model of
//!   Radford et al., *Robust Speech Recognition via Large-Scale Weak Supervision*
//!   ([arXiv:2212.04356](https://arxiv.org/abs/2212.04356)). The capture device and the
//!   worker thread stay with the app.
//! - `download` adds `HttpTransport`, the HTTPS transport for model files. The manifest only
//!   accepts `https://` URLs on the model host it names.
//!
//! Every function treats its input as hostile (NaN samples, zero or huge sample rates, damaged
//! files, hand-edited manifests) and returns an error instead of panicking. Audio, line lengths
//! and model files are bounded.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod audio;
pub mod consent;
pub mod download;
pub mod manifest;
pub mod provider;

#[cfg(feature = "candle")]
mod candle_backend;
#[cfg(feature = "candle")]
pub use candle_backend::{WhisperEngine, WhisperRecognizer};

pub use audio::{
    AudioBuffer, AudioError, Limits, MAX_AUDIO_BYTES, MAX_AUDIO_SECONDS, MAX_SAMPLE_RATE, MAX_SAMPLES, TARGET_RATE, downmix, resample_linear,
};
pub use consent::{Acceptance, Consent, Decision, Disclosure, FileDisclosure, fingerprint};
#[cfg(all(feature = "download", not(target_arch = "wasm32")))]
pub use download::HttpTransport;
pub use download::{DownloadError, Transport, TransportError, download_with};
pub use manifest::{
    MAX_FIELD_CHARS, MAX_FILE_BYTES, MAX_FILES, ModelFile, ModelManifest, ModelState, ModelStatus, SIDECAR, VerifyError, WHISPER_TINY_ID,
    WHISPER_TINY_REVISION, check_dir, check_dir_files, check_dir_until, model_url_allowed, sha256_file, sha256_file_until, sha256_hex, verify_file,
    verify_file_until, whisper_tiny, write_sidecar,
};
pub use provider::{AsrError, FakeRecognizer, MAX_SEGMENT_CHARS, Recognizer, Segment, State, Status};
