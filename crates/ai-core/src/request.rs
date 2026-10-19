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
    pub role: InputRole,
    pub payload: Payload,
}

/// A unit of work for a model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub capability: CapabilityId,
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
    pub fn new(capability: CapabilityId) -> Self {
        Self { capability, inputs: Vec::new(), params: Map::new(), outputs: 1, stream: false }
    }

    pub fn with_input(mut self, role: InputRole, payload: Payload) -> Self {
        self.inputs.push(Input { role, payload });
        self
    }

    pub fn with_param(mut self, key: &str, value: Value) -> Self {
        self.params.insert(key.to_owned(), value);
        self
    }

    pub fn with_outputs(mut self, n: u32) -> Self {
        self.outputs = n;
        self
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use crate::model::Features;

    fn cap(s: &str) -> CapabilityId {
        CapabilityId::parse(s).unwrap()
    }

    fn png() -> Payload {
        Payload::inline("image/png", vec![1, 2, 3]).unwrap()
    }

    fn inpaint_spec() -> CapabilitySpec {
        CapabilitySpec::new(cap("image.inpaint"))
            .accepting(InputRole::Prompt, 1)
            .accepting(InputRole::Source, 1)
            .accepting(InputRole::Mask, 1)
            .accepting(InputRole::Reference, 1)
            .requiring(InputRole::Source)
            .requiring(InputRole::Mask)
            .max_outputs(3)
    }

    fn good() -> Request {
        Request::new(cap("image.inpaint")).with_input(InputRole::Source, png()).with_input(InputRole::Mask, png())
    }

    #[test]
    fn a_well_formed_request_passes() {
        assert!(good().with_input(InputRole::Prompt, Payload::text("a cat")).validate(&inpaint_spec(), &Limits::default()).is_ok());
    }

    #[test]
    fn unsupported_input_is_a_clear_error() {
        let spec = CapabilitySpec::new(cap("image.generate")).accepting(InputRole::Prompt, 1);
        let req = Request::new(cap("image.generate")).with_input(InputRole::Mask, png());
        let err = req.validate(&spec, &Limits::default()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Unsupported);
        assert!(err.message.contains("mask"));
    }

    #[test]
    fn too_many_references_are_counted_separately_from_masks() {
        let req = good().with_input(InputRole::Reference, png()).with_input(InputRole::Reference, png());
        let err = req.validate(&inpaint_spec(), &Limits::default()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Unsupported);
        assert!(err.message.contains("reference"));
    }

    #[test]
    fn missing_required_input_is_invalid() {
        let req = Request::new(cap("image.inpaint")).with_input(InputRole::Source, png());
        let err = req.validate(&inpaint_spec(), &Limits::default()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidRequest);
        assert!(err.message.contains("mask"));
    }

    #[test]
    fn output_count_is_bounded() {
        assert!(good().with_outputs(3).validate(&inpaint_spec(), &Limits::default()).is_ok());
        assert!(good().with_outputs(4).validate(&inpaint_spec(), &Limits::default()).is_err());
        assert!(good().with_outputs(0).validate(&inpaint_spec(), &Limits::default()).is_err());
    }

    #[test]
    fn streaming_needs_the_feature() {
        let spec = CapabilitySpec::new(cap("text.complete")).accepting(InputRole::Prompt, 1);
        let req = Request::new(cap("text.complete")).with_input(InputRole::Prompt, Payload::text("hi")).streaming();
        assert_eq!(req.validate(&spec, &Limits::default()).unwrap_err().kind, ErrorKind::Unsupported);
        let spec = spec.with_features(Features { streaming: true, ..Features::default() });
        assert!(req.validate(&spec, &Limits::default()).is_ok());
    }

    #[test]
    fn capability_mismatch_is_invalid() {
        let req = Request::new(cap("image.generate"));
        assert_eq!(req.validate(&inpaint_spec(), &Limits::default()).unwrap_err().kind, ErrorKind::InvalidRequest);
    }

    #[test]
    fn oversized_inline_input_is_refused() {
        let limits = Limits { max_inline_bytes: 2, max_text_bytes: 10 };
        assert!(good().validate(&inpaint_spec(), &limits).is_err());
    }

    #[test]
    fn oversized_params_are_refused() {
        let big = Value::String("x".repeat(MAX_PARAMS_BYTES + 1));
        assert!(good().with_param("blob", big).validate(&inpaint_spec(), &Limits::default()).is_err());
    }

    #[test]
    fn request_round_trips_and_defaults_apply() {
        let req = good().with_param("seed", Value::from(7)).with_outputs(2);
        let back: Request = serde_json::from_str(&serde_json::to_string(&req).unwrap()).unwrap();
        assert_eq!(req, back);
        let minimal: Request = serde_json::from_str(r#"{"capability":"image.generate"}"#).unwrap();
        assert_eq!(minimal.outputs, 1);
        assert!(!minimal.stream);
    }
}
