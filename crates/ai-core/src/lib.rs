//! Shared data model for AI in the Craft apps: connections, models, capabilities, roles,
//! requests, jobs, consent and the launcher's shared store.
//!
//! This is the **interim contract** (task x1). It is small on purpose, has no I/O and no async
//! runtime, and compiles for wasm. Apps build their adapters against it and test with
//! [`mock::MockProvider`]. When the suite's AI core is chosen, it should adopt or replace this
//! model, and the swap should touch one module per app.
//!
//! # Map
//!
//! | Module | What it holds |
//! |---|---|
//! | [`ids`] | validated ids; `validated_string!` and `open_enum!` |
//! | [`secret`] | [`SecretString`] (not printable, not serializable) and `redact` |
//! | [`error`] | [`AiError`] with an actionable [`ErrorKind`] |
//! | [`history`] | the request log: [`HistoryRecord`] lines and how they merge into [`HistoryEntry`] rows |
//! | [`capability`] | [`Modality`], [`CapabilityId`] (`image.inpaint`), the open well-known list |
//! | [`payload`] | [`Payload`] (text, inline, handle), limits, base64 |
//! | [`model`] | [`ModelInfo`], [`CapabilitySpec`], [`InputRole`], [`Features`] |
//! | [`request`] | [`Request`] and its validation against a spec |
//! | [`job`] | [`JobState`], [`Event`], [`JobSnapshot`], [`Usage`], [`CostEstimate`] |
//! | [`endpoint`] | strict endpoint parsing and loopback detection |
//! | [`connection`] | [`Connection`], [`Auth`], [`Billing`], [`Locality`] |
//! | [`role`] | [`Role`], [`Binding`], [`resolve`](role::resolve), [`eligible`](role::eligible) |
//! | [`consent`] | [`Disclosure`], [`ConsentLedger`] |
//! | [`store`] | [`StoreFile`], the launcher's versioned shared store |
//! | [`picks`] | model presets: [`Pick`], the curated [`Catalog`], and [`picks::choose`] |
//! | [`spec`] | a provider described as data: types, checker and substitutions (task u2) |
//! | [`provider`] | the [`Provider`] trait |
//! | [`mock`], [`conformance`] | scripted provider and reusable behaviour checks |

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod capability;
pub mod conformance;
pub mod connection;
pub mod consent;
pub mod draft;
pub mod endpoint;
pub mod error;
pub mod history;
pub mod ids;
pub mod job;
pub mod mock;
pub mod model;
pub mod payload;
pub mod picks;
pub mod provider;
pub mod request;
pub mod role;
pub mod secret;
pub mod spec;
pub mod store;

pub use capability::{CapabilityId, Modality};
pub use connection::{Auth, Billing, Connection, Locality};
pub use consent::{ConsentLedger, Disclosure};
pub use draft::{ConnectionDraft, DraftError, Field, KeyChange, PRESETS, Prepared, Preset};
pub use error::{AiError, ErrorKind};
pub use history::{EndRecord, HistoryEntry, HistoryLog, HistoryRecord, Outcome, StartRecord};
pub use ids::{AppId, ConnectionId, JobId, ModelId, RoleId, SecretRef};
pub use job::{CostEstimate, Event, JobSnapshot, JobState, Output, Usage};
pub use model::{CapabilitySpec, Features, InputRole, ModelInfo, Pricing};
pub use payload::{Limits, MediaType, Payload, PayloadHandle, PayloadKind};
pub use picks::{Catalog, Pick, PickedBy};
pub use provider::Provider;
pub use request::Request;
pub use role::{Binding, Role};
pub use secret::SecretString;
pub use store::StoreFile;
