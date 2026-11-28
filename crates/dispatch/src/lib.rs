//! Shared job schema and queue payloads for handing export work to encoderCraft over `craft-ipc`.
//!
//! Craft applications build a [`CraftJobSpec`] describing the media source (`File`,
//! `StitchedClips`, `Pipe`, or `ProjectReference`) and one or more [`OutputTarget`]s, and dispatch
//! it over `craft-ipc` using the [`queue`] method names and payload structures (`queue.submit`,
//! `queue.status`, `queue.list`, `queue.cancel`, `queue.subscribe`).
//!
//! Moved from `encoderCraft`'s `encodercraft-core` and `encodercraft-dispatch` crates
//! (`storytold/craft-libs#505`).

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod audio;
pub mod job;
pub mod metadata;
pub mod queue;
pub mod timecode;

pub use audio::{AudioChannelMapping, AudioTrackLayout, ChannelLayout};
pub use job::{
    AudioEncodeSettings, AudioStemMode, CaptionExportFormat, ClientInfo, ColorTag, ContainerType, CraftJobSpec, CropConfig, DeinterlaceMode,
    FlipMode, FrameRateMode, JobPriority, JobStatus, MediaStreamKind, NameOverlayConfig, OutputTarget, OverlayAnchor, PixelAspectRatioConform,
    PixelFormat, RateMode, Rotation, ScaleMode, ScalingFilter, SdrConformConfig, SourceDescriptor, SourceSelection, StartPolicy, StitchedClipSource,
    TimecodeOverlayConfig, VideoEncodeSettings, VideoLimiterClipLevel, VideoLimiterConfig, VideoTransform, WatermarkOverlayConfig,
};
pub use metadata::XmpMetadata;
pub use queue::{
    METHOD_QUEUE_CANCEL, METHOD_QUEUE_CLEAR_FINISHED, METHOD_QUEUE_LIST, METHOD_QUEUE_LOG, METHOD_QUEUE_PAUSE, METHOD_QUEUE_START,
    METHOD_QUEUE_STATUS, METHOD_QUEUE_SUBMIT, METHOD_QUEUE_SUBSCRIBE, QueueCancelParams, QueueEvent, QueueJobSummary, QueueListResult,
    QueuePauseParams, QueueStatusParams, QueueSubmitResult, QueueSubscribeParams,
};
pub use timecode::{MediaTimecode, Rational64};

/// Errors returned when validating or serializing a [`CraftJobSpec`] or queue payload.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DispatchError {
    /// A job specification or queue payload violates a schema constraint.
    #[error("invalid job specification: {0}")]
    InvalidJobSpec(String),
    /// JSON serialization or deserialization failed.
    #[error("JSON error: {0}")]
    Json(String),
    /// Underlying `craft-ipc` link error.
    #[cfg(all(feature = "ipc", not(target_arch = "wasm32")))]
    #[error("IPC link error: {0}")]
    Ipc(#[from] craft_ipc::LinkError),
}
