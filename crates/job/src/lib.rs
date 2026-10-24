//! The vocabulary that printing and scanning (and any other device-job crate) would otherwise
//! each define for themselves: a native error code, namespaced backend ids, the retry-safety
//! rules with the one function that turns them into advice, and plain data records a UI can show
//! the same way for both: [`JobView`] and [`DeviceView`].
//!
//! A leaf crate: `std` only, no I/O, no threads, no dependencies, builds for
//! `wasm32-unknown-unknown`. It has no lifecycle object or state machine, no error kinds, no
//! attempt tracking, no reconciliation engine and no UI; those belong to the domain crates. See
//! the crate README.
//!
//! | Module | Contents |
//! |---|---|
//! | [`native`] | [`NativeCode`], [`HRESULT_DOMAIN`] |
//! | [`id`] | [`BackendName`], [`JobId`], [`DeviceId`], [`ParseIdError`], [`namespaced_id`], [`split_id`] |
//! | [`retry`] | [`RetrySafety`], [`RetryPolicy`], [`RetryFacts`], [`RetryAdvice`], [`advise`], [`Jitter`] |
//! | [`job`] | [`JobKind`], [`JobState`], [`Progress`], [`ExpectedTotals`] |
//! | [`document`] | [`DocumentRef`], [`JobDocuments`] |
//! | [`time`] | [`Timestamp`], [`JobTimes`] |
//! | [`tag`] | [`Tag`], [`TagSupport`], [`TagPlacement`], [`Fingerprint`] |
//! | [`view`] | [`JobView`], [`ErrorSummary`] |
//! | [`device`] | [`DeviceView`], [`Indicator`], [`IndicatorKey`], [`Tri`], [`Severity`], [`DeviceState`], [`DeviceConnection`], [`CapabilitiesSummary`], [`PaperPathView`], [`PaperSource`], [`Supply`] |
//!
//! ```
//! use craft_job::{advise, RetryAdvice, RetryFacts, RetryPolicy, RetrySafety};
//!
//! let policy = RetryPolicy::default();
//! // A refused connection: transient, and nothing was delivered.
//! let facts = RetryFacts::new(true, RetrySafety::Safe).with_attempts(1);
//! assert!(matches!(advise(&facts, &policy), RetryAdvice::RetryAfter(_) | RetryAdvice::RetryNow));
//! // The reply to a submit never came: look at the device before anything is sent again.
//! let facts = RetryFacts::new(true, RetrySafety::OutcomeUnknown).with_attempts(1);
//! assert_eq!(advise(&facts, &policy), RetryAdvice::CheckDeviceFirst);
//! ```

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod device;
pub mod document;
pub mod id;
pub mod job;
pub mod native;
pub mod retry;
pub mod tag;
pub mod time;
pub mod view;

#[cfg(feature = "serde")]
mod wire;

pub use device::{
    CapabilitiesSummary, DeviceConnection, DeviceState, DeviceView, Indicator, IndicatorKey, PaperPathView, PaperSource, Severity, Supply, Tri,
};
pub use document::{DocumentRef, JobDocuments};
pub use id::{BackendName, DeviceId, JobId, ParseIdError, ParseIdErrorKind, namespaced_id, split_id};
pub use job::{ExpectedTotals, JobKind, JobState, Progress};
pub use native::{HRESULT_DOMAIN, NativeCode};
pub use retry::{Jitter, RetryAdvice, RetryFacts, RetryPolicy, RetrySafety, advise};
pub use tag::{Fingerprint, Tag, TagError, TagPlacement, TagSupport};
pub use time::{JobTimes, Timestamp};
pub use view::{ErrorSummary, JobView};
