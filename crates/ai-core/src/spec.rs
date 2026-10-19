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
/// Largest rendered or stored template (a body, serialized).
pub const MAX_TEMPLATE_BYTES: usize = 4 * 1024;
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
    Header { name: String },
    /// No key.
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    Get,
    Post,
    Delete,
}

/// One call. `path` is joined to `base`; `query` and `body` may use the allowed substitutions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub method: Method,
    pub path: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub query: BTreeMap<String, String>,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extract {
    /// A JSON pointer (RFC 6901) into the reply.
    pub at: String,
    #[serde(rename = "as")]
    pub kind: ExtractAs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

/// The four states a job can be in, as a status word maps onto them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateWord {
    Queued,
    Running,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submit {
    pub call: Call,
    /// Pointer to the job id in the reply.
    pub job_id_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Poll {
    pub call: Call,
    pub status_at: String,
    pub states: BTreeMap<String, StateWord>,
    pub result: Extract,
}

// A spec is loaded once and read many times; boxing the queue variant would only add noise.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Flow {
    /// One call whose reply holds the result.
    RequestResponse { call: Call, result: Extract },
    /// Submit, then ask for the result until it is ready.
    Queue {
        submit: Submit,
        poll: Poll,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancel: Option<Call>,
    },
}

/// A model the spec offers. Nothing is offered that is not declared here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecModel {
    pub id: String,
    pub label: String,
    pub capability: String,
    /// Input role to the most of it the model takes.
    pub accepts: BTreeMap<String, u32>,
    #[serde(default)]
    pub requires: Vec<String>,
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
    pub spec_version: u32,
    pub id: String,
    pub label: String,
    /// `https://host[/prefix]`, or `http://` for a loopback host.
    pub base: String,
    #[serde(default = "bearer")]
    pub auth: SpecAuth,
    /// A free, read-only request for the Test button.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<Call>,
    pub models: Vec<SpecModel>,
    pub flow: Flow,
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
