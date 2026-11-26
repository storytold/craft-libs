//! Requests: what an app asks a model to do.
//!
//! A request names a capability and carries typed inputs (prompt, source, mask, references)
//! plus free-form parameters. [`Request::validate`] checks it against a model's
//! [`CapabilitySpec`] before anything is sent, so "this model takes no mask" or "at most one
//! reference" is a clear [`AiError`] with kind `Unsupported`, never a mystery from the provider.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::capability::CapabilityId;
use crate::error::AiError;
use crate::model::{CapabilitySpec, InputRole};
use crate::payload::{Limits, Payload};

/// Largest accepted serialized size of `params`.
pub const MAX_PARAMS_BYTES: usize = 64 * 1024;

/// One typed input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    /// What the input is for.
    pub role: InputRole,
    /// The data.
    pub payload: Payload,
}

/// A unit of work for a model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// What the model is asked to do.
    pub capability: CapabilityId,
    /// The typed inputs, in the order given.
    #[serde(default)]
    pub inputs: Vec<Input>,
    /// Provider-neutral options such as `seed` or `size`. Capped by [`MAX_PARAMS_BYTES`].
    #[serde(default)]
    pub params: Map<String, Value>,
    /// How many results to return (variations).
    #[serde(default = "one")]
    pub outputs: u32,
    /// Ask for incremental results (text deltas).
    #[serde(default)]
    pub stream: bool,
}

fn one() -> u32 {
    1
}

impl Request {
    /// A request for `capability` with no inputs or params, one output, and no streaming.
    pub fn new(capability: CapabilityId) -> Self {
        Self { capability, inputs: Vec::new(), params: Map::new(), outputs: 1, stream: false }
    }

    /// Adds an input.
    pub fn with_input(mut self, role: InputRole, payload: Payload) -> Self {
        self.inputs.push(Input { role, payload });
        self
    }

    /// Sets a param, replacing any earlier value under `key`.
    pub fn with_param(mut self, key: &str, value: Value) -> Self {
        self.params.insert(key.to_owned(), value);
        self
    }

    /// Sets how many results to return.
    pub fn with_outputs(mut self, n: u32) -> Self {
        self.outputs = n;
        self
    }

    /// Asks for incremental results.
    pub fn streaming(mut self) -> Self {
        self.stream = true;
        self
    }

    /// How many inputs have this role.
    pub fn count(&self, role: &InputRole) -> usize {
        self.inputs.iter().filter(|i| &i.role == role).count()
    }

    /// Checks the request against what a model can do. The first problem found is returned.
    pub fn validate(&self, spec: &CapabilitySpec, limits: &Limits) -> Result<(), AiError> {
        if self.capability != spec.capability {
            return Err(AiError::invalid(format!("request is for {} but the model spec is for {}", self.capability, spec.capability)));
        }
        let mut counts: BTreeMap<&InputRole, u32> = BTreeMap::new();
        for input in &self.inputs {
            input.payload.check(limits)?;
            let n = counts.entry(&input.role).or_insert(0);
            *n = n.saturating_add(1);
        }
        for (role, n) in &counts {
            match spec.accepts.get(*role) {
                None => {
                    return Err(AiError::unsupported(format!("this model does not accept a '{role}' input")));
                }
                Some(max) if n > max => {
                    return Err(AiError::unsupported(format!("this model accepts at most {max} '{role}' input(s), got {n}")));
                }
                Some(_) => {}
            }
        }
        if let Some(missing) = spec.requires.iter().find(|r| !counts.contains_key(r)) {
            return Err(AiError::invalid(format!("a '{missing}' input is required")));
        }
        if self.outputs == 0 || self.outputs > spec.max_outputs {
            return Err(AiError::unsupported(format!("this model returns at most {} output(s); {} requested", spec.max_outputs, self.outputs)));
        }
        if self.stream && !spec.features.streaming {
            return Err(AiError::unsupported("this model cannot stream"));
        }
        let params_len = serde_json::to_vec(&self.params).map(|v| v.len()).unwrap_or(usize::MAX);
        if params_len > MAX_PARAMS_BYTES {
            return Err(AiError::invalid(format!("params are over {MAX_PARAMS_BYTES} bytes")));
        }
        Ok(())
    }
}
