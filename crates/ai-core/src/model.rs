//! Models and what each one can do.
//!
//! A provider offers models. A model advertises [`CapabilitySpec`]s: for each capability, which
//! inputs it accepts and how many of each (so "takes a mask" and "takes at most one reference"
//! are data, not guesses), how many outputs it can return, and whether it can stream, call tools
//! or return structured output. Requests are validated against the spec before anything is sent.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::capability::CapabilityId;
use crate::ids::{ModelId, open_enum};

open_enum!(
    /// What an input is for. Edits need typed inputs: a mask is not just another image.
    InputRole {
        Prompt => "prompt",
        System => "system",
        Source => "source",
        Mask => "mask",
        Reference => "reference",
        Context => "context",
    }
);

/// Optional abilities beyond the basic request and result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Features {
    /// The model can stream text deltas as it generates.
    #[serde(default)]
    pub streaming: bool,
    /// The model can call tools.
    #[serde(default)]
    pub tools: bool,
    /// The model can return JSON that follows a schema.
    #[serde(default)]
    pub structured_output: bool,
}

/// What one model can do for one capability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySpec {
    /// The capability this spec describes.
    pub capability: CapabilityId,
    /// Inputs the model accepts, with the most of each. An input role not listed is unsupported.
    pub accepts: BTreeMap<InputRole, u32>,
    /// Inputs that must be present.
    #[serde(default)]
    pub requires: Vec<InputRole>,
    /// Most outputs one request can return (variations count here).
    #[serde(default = "one")]
    pub max_outputs: u32,
    /// Optional abilities the model has for this capability.
    #[serde(default)]
    pub features: Features,
}

fn one() -> u32 {
    1
}

impl CapabilitySpec {
    /// A spec for `capability` that accepts no inputs, requires none and returns one output; add
    /// inputs with [`accepting`](Self::accepting).
    pub fn new(capability: CapabilityId) -> Self {
        Self { capability, accepts: BTreeMap::new(), requires: Vec::new(), max_outputs: 1, features: Features::default() }
    }

    /// Accepts up to `max` inputs of `role`.
    pub fn accepting(mut self, role: InputRole, max: u32) -> Self {
        self.accepts.insert(role, max);
        self
    }

    /// Requires an input of `role`. Adding the same role twice has no effect.
    pub fn requiring(mut self, role: InputRole) -> Self {
        if !self.requires.contains(&role) {
            self.requires.push(role);
        }
        self
    }

    /// Sets the most outputs one request can return.
    pub fn max_outputs(mut self, n: u32) -> Self {
        self.max_outputs = n;
        self
    }

    /// Sets the optional abilities.
    pub fn with_features(mut self, features: Features) -> Self {
        self.features = features;
        self
    }
}

/// What a provider says a model costs, in millionths of a US dollar. Every field is optional:
/// providers report some, one or none, and a missing price is never filled in by guessing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pricing {
    /// For one million input tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_micros_per_mtok: Option<u64>,
    /// For one million output tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_micros_per_mtok: Option<u64>,
    /// For one image (or one generated clip, for video).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_image_micros: Option<u64>,
}

/// One model as offered by one connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    #[allow(missing_docs)]
    pub id: ModelId,
    /// Name shown to the user.
    pub label: String,
    /// What the model can do, one spec per capability.
    #[serde(default)]
    pub capabilities: Vec<CapabilitySpec>,
    /// Reported by the provider where it can be (OpenRouter does). Absent in stores written before
    /// this field existed, which still load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<Pricing>,
    /// When the model was released, in Unix seconds, where the provider reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub released: Option<u64>,
    /// Labels from the curated catalog, such as `recommended`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Where the weights are on this computer, for a model an app runs itself (a native adapter
    /// reads this file). Absent for hosted models and for ones a server such as ComfyUI keeps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_file: Option<String>,
}

impl ModelInfo {
    /// Builds a model with no capabilities. Add them with [`ModelInfo::with`].
    pub fn new(id: ModelId, label: impl Into<String>) -> Self {
        Self { id, label: label.into(), capabilities: Vec::new(), pricing: None, released: None, tags: Vec::new(), local_file: None }
    }

    /// Adds a capability spec.
    pub fn with(mut self, spec: CapabilitySpec) -> Self {
        self.capabilities.push(spec);
        self
    }

    /// The spec for `capability`, if the model supports it.
    pub fn spec_for(&self, capability: &CapabilityId) -> Option<&CapabilitySpec> {
        self.capabilities.iter().find(|s| &s.capability == capability)
    }
}
