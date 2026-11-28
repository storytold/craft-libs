//! A declarative route: a small data file that describes how to call a provider whose API is not
//! OpenAI-shaped (design note `docs/ai-declarative-adapter.md`).
//!
//! A spec runs no code. It names the calls to make, where the key goes and where in each reply to
//! find the result. This module holds the types and the pure checker, so it compiles for wasm and
//! a launcher can validate a spec as it is pasted. The runner that makes the calls lives in the
//! native crate `craft-ai-declarative`.
//!
//! Invariants, all enforced by [`Spec::parse`]:
//! - the file is at most [`MAX_SPEC_BYTES`], and every list and string has a bound;
//! - unknown fields are refused, never ignored;
//! - every path is relative to `base` and cannot name another host;
//! - the only substitutions are `{prompt}`, `{model}` and `{job_id}`, and each is allowed only
//!   where it is safe (see [`template`]);
//! - the key is never part of a spec.
//!
//! Version 1 leaves out, on purpose: uploads, streaming, signing, webhooks, request parameters
//! and a second input kind next to the prompt. A spec that needs one is refused with its name.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::capability::CapabilityId;
use crate::ids::ModelId;
use crate::model::{CapabilitySpec, InputRole, ModelInfo};

mod check;
pub mod template;

pub use check::SpecError;

/// The only spec version this build reads.
pub const SPEC_VERSION: u32 = 1;
/// Largest spec file accepted.
pub const MAX_SPEC_BYTES: usize = 32 * 1024;
/// Most models one spec may offer.
pub const MAX_MODELS: usize = 16;
/// Largest stored template (a body, serialized).
pub const MAX_TEMPLATE_BYTES: usize = 4 * 1024;
/// Most text one rendered template may hold, summed over its strings with every substitution made.
/// A slot may repeat, so this, not [`MAX_TEMPLATE_BYTES`], bounds what a prompt can grow to.
pub const MAX_RENDERED_BYTES: usize = 4 * 1024 * 1024;
/// Deepest JSON pointer.
pub const MAX_POINTER_DEPTH: usize = 8;
/// Most entries in a status table.
pub const MAX_STATES: usize = 16;
/// Longest path or query value.
pub const MAX_PATH_BYTES: usize = 256;

/// How the key is sent. The key itself comes from the connection's secret reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpecAuth {
    /// `Authorization: Bearer <key>`.
    Bearer,
    /// The key is the whole value of this header.
    Header {
        /// The header name. It must pass the same checks as a connection's key header.
        name: String,
    },
    /// No key.
    None,
}

/// HTTP methods a call may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    #[allow(missing_docs)]
    Get,
    #[allow(missing_docs)]
    Post,
    #[allow(missing_docs)]
    Delete,
}

/// One call. `path` is joined to `base`; `query` and `body` may use the allowed substitutions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    /// HTTP method.
    pub method: Method,
    /// Path joined to the spec's `base`. Starts with `/` and cannot name another host.
    pub path: String,
    /// Query parameters. Values may use the substitutions allowed for the call.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub query: BTreeMap<String, String>,
    /// JSON body template. Substitutions apply inside its string values only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
}

/// What a result is, and how to read it from the reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractAs {
    /// Base64 text that decodes to media bytes (needs `media_type`).
    Base64,
    /// An https link, returned as a handle and never fetched here (needs `media_type`).
    Url,
    /// Plain text.
    Text,
}

/// Where in a reply to find a result, and what kind of data it is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extract {
    /// A JSON pointer (RFC 6901) into the reply.
    pub at: String,
    /// How to read the value at `at`. Written `as` in the spec file.
    #[serde(rename = "as")]
    pub kind: ExtractAs,
    /// Media type of the result, such as `image/png`. Required for `base64` and `url`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

/// The four states a job can be in, as a status word maps onto them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateWord {
    /// The provider has accepted the job but not started it.
    Queued,
    /// The job is in progress.
    Running,
    /// The job finished and the result is ready.
    Succeeded,
    /// The job failed.
    Failed,
}

/// The call that starts a queued job.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submit {
    /// The call to make.
    pub call: Call,
    /// Pointer to the job id in the reply.
    pub job_id_at: String,
}

/// The repeated call that checks a queued job and reads its result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Poll {
    /// The call to make. It may use `{job_id}`.
    pub call: Call,
    /// JSON pointer to the status word in the reply.
    pub status_at: String,
    /// Maps each status word the provider uses to one of the four states. A word not listed is an error.
    pub states: BTreeMap<String, StateWord>,
    /// Where to read the result once the job has succeeded.
    pub result: Extract,
}

// A spec is loaded once and read many times; boxing the queue variant would only add noise.
/// How a request becomes a result: one call, or a submit and poll queue.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Flow {
    /// One call whose reply holds the result.
    RequestResponse {
        /// The one call to make.
        call: Call,
        /// Where the result is in the reply.
        result: Extract,
    },
    /// Submit, then ask for the result until it is ready.
    Queue {
        /// The call that starts the job.
        submit: Submit,
        /// The call that checks it.
        poll: Poll,
        /// Optional call that cancels the job.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancel: Option<Call>,
    },
}

/// A model the spec offers. Nothing is offered that is not declared here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecModel {
    /// Model id, as the provider names it. Must be a valid [`ModelId`].
    pub id: String,
    /// Name shown to the user.
    pub label: String,
    /// The capability the model offers, as `<modality>.<task>`.
    pub capability: String,
    /// Input role to the most of it the model takes.
    pub accepts: BTreeMap<String, u32>,
    /// Input roles that must be sent. Every model must require `prompt`.
    #[serde(default)]
    pub requires: Vec<String>,
    /// Most outputs one request can return. Defaults to 1.
    #[serde(default = "one")]
    pub max_outputs: u32,
}

fn one() -> u32 {
    1
}

/// Bounds on one job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecLimits {
    /// Most status checks for one job.
    #[serde(default = "default_polls")]
    pub max_polls: u32,
    /// Most seconds from submit to a final state.
    #[serde(default = "default_seconds")]
    pub max_seconds: u32,
}

fn default_polls() -> u32 {
    600
}

fn default_seconds() -> u32 {
    600
}

impl Default for SpecLimits {
    fn default() -> Self {
        Self { max_polls: default_polls(), max_seconds: default_seconds() }
    }
}

/// A checked spec. Build one only through [`Spec::parse`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    /// Must equal [`SPEC_VERSION`].
    pub spec_version: u32,
    /// Id for the connection this spec makes. Must be a valid [`ConnectionId`](crate::ids::ConnectionId).
    pub id: String,
    /// Name shown to the user, 1 to 120 plain characters.
    pub label: String,
    /// `https://host[/prefix]`, or `http://` for a loopback host.
    pub base: String,
    /// How the key is sent. Defaults to a bearer token.
    #[serde(default = "bearer")]
    pub auth: SpecAuth,
    /// A free, read-only request for the Test button.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<Call>,
    /// The models the spec offers, at most [`MAX_MODELS`].
    pub models: Vec<SpecModel>,
    /// How a request becomes a result.
    pub flow: Flow,
    /// Bounds on one job.
    #[serde(default)]
    pub limits: SpecLimits,
}

fn bearer() -> SpecAuth {
    SpecAuth::Bearer
}

impl Spec {
    /// Reads and checks a spec. Nothing is returned that has not passed every rule.
    pub fn parse(text: &str) -> Result<Self, SpecError> {
        check::parse(text)
    }

    /// The models the spec offers, as the rest of the layer sees them.
    pub fn model_infos(&self) -> Vec<ModelInfo> {
        self.models
            .iter()
            .filter_map(|m| {
                let mut capability = CapabilitySpec::new(CapabilityId::parse(&m.capability).ok()?).max_outputs(m.max_outputs);
                for (role, count) in &m.accepts {
                    capability = capability.accepting(InputRole::parse(role).ok()?, *count);
                }
                for role in &m.requires {
                    capability = capability.requiring(InputRole::parse(role).ok()?);
                }
                Some(ModelInfo::new(ModelId::new(&m.id).ok()?, &m.label).with(capability))
            })
            .collect()
    }
}
