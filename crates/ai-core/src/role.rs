//! Roles and bindings: how an app's features reach a model.
//!
//! An app declares [`Role`]s ("Select Subject", "Generative Fill", "Rewrite"), each needing one
//! capability and some inputs. The user binds a role to a connection and model with a
//! [`Binding`]. Bindings come in layers: an app override beats the launcher default.
//!
//! [`resolve`] picks the first layer that mentions the role and then checks it end to end. If
//! that binding is broken it is an error. It never quietly falls through to a different
//! connection, because that could send data somewhere the user did not choose.
//! [`eligible`] lists every model that could fill a role, for the model picker.

use serde::{Deserialize, Serialize};

use crate::capability::CapabilityId;
use crate::connection::Connection;
use crate::ids::{ConnectionId, ModelId, RoleId};
use crate::model::{CapabilitySpec, InputRole, ModelInfo};

/// A feature an app offers that needs a model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    #[allow(missing_docs)]
    pub id: RoleId,
    /// Name shown to the user, such as "Generative Fill".
    pub label: String,
    /// The capability a model must offer to fill the role.
    pub capability: CapabilityId,
    /// Inputs this feature will send. A model must accept them and require no others.
    #[serde(default)]
    pub inputs: Vec<InputRole>,
}

/// The user's choice for one role.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    /// The role being bound.
    pub role: RoleId,
    /// The chosen connection.
    pub connection: ConnectionId,
    /// The chosen model, offered by that connection.
    pub model: ModelId,
}

/// A fully checked choice.
#[derive(Clone, Copy, Debug)]
pub struct Resolved<'a> {
    /// The connection to send the job to.
    pub connection: &'a Connection,
    /// The model to run.
    pub model: &'a ModelInfo,
    /// The model's spec for the role's capability.
    pub spec: &'a CapabilitySpec,
}

/// Why a role could not be resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// No layer has a binding for the role.
    Unbound(RoleId),
    /// The bound connection is not in the list.
    ConnectionMissing(ConnectionId),
    /// The bound connection is switched off.
    ConnectionDisabled(ConnectionId),
    /// The bound connection does not offer the bound model.
    ModelMissing {
        /// The bound connection.
        connection: ConnectionId,
        /// The bound model.
        model: ModelId,
    },
    /// The model has no spec for the role's capability.
    CapabilityMismatch {
        /// The bound model.
        model: ModelId,
        /// The capability the role needs.
        capability: CapabilityId,
    },
    /// The role sends an input the model does not accept.
    InputUnsupported {
        /// The bound model.
        model: ModelId,
        /// The input the model rejects.
        input: InputRole,
    },
    /// The model requires an input the role does not send.
    InputRequired {
        /// The bound model.
        model: ModelId,
        /// The input the role is missing.
        input: InputRole,
    },
    /// A tool role differs from the shared definition saved for its id.
    ToolMismatch(RoleId),
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unbound(r) => write!(f, "no model is chosen for '{r}'"),
            Self::ConnectionMissing(c) => write!(f, "connection {c} no longer exists"),
            Self::ConnectionDisabled(c) => write!(f, "connection {c} is switched off"),
            Self::ModelMissing { connection, model } => {
                write!(f, "connection {connection} does not offer model {model}")
            }
            Self::CapabilityMismatch { model, capability } => {
                write!(f, "model {model} cannot do {capability}")
            }
            Self::InputUnsupported { model, input } => {
                write!(f, "model {model} does not accept a '{input}' input")
            }
            Self::InputRequired { model, input } => {
                write!(f, "model {model} requires a '{input}' input that this tool does not provide")
            }
            Self::ToolMismatch(id) => write!(f, "tool {id} does not match its shared definition"),
        }
    }
}

impl std::error::Error for ResolveError {}

/// Resolves `role`. `layers` runs most specific first (app override, then launcher default).
pub fn resolve<'a>(role: &Role, layers: &[&[Binding]], connections: &'a [Connection]) -> Result<Resolved<'a>, ResolveError> {
    let binding = layers.iter().find_map(|layer| layer.iter().find(|b| b.role == role.id)).ok_or_else(|| ResolveError::Unbound(role.id.clone()))?;
    let connection =
        connections.iter().find(|c| c.id == binding.connection).ok_or_else(|| ResolveError::ConnectionMissing(binding.connection.clone()))?;
    if !connection.enabled {
        return Err(ResolveError::ConnectionDisabled(connection.id.clone()));
    }
    let model = connection
        .model(&binding.model)
        .ok_or_else(|| ResolveError::ModelMissing { connection: connection.id.clone(), model: binding.model.clone() })?;
    let spec = model
        .spec_for(&role.capability)
        .ok_or_else(|| ResolveError::CapabilityMismatch { model: model.id.clone(), capability: role.capability.clone() })?;
    if let Some(input) = role.inputs.iter().find(|i| !spec.accepts.contains_key(i)) {
        return Err(ResolveError::InputUnsupported { model: model.id.clone(), input: input.clone() });
    }
    if let Some(input) = spec.requires.iter().find(|i| !role.inputs.contains(i)) {
        return Err(ResolveError::InputRequired { model: model.id.clone(), input: input.clone() });
    }
    Ok(Resolved { connection, model, spec })
}

/// What `model` can do for `role`: its spec for the role's capability, if it takes every input the
/// role sends and the role supplies every input the model requires. `None` means the model
/// cannot fill the role.
pub fn fits<'a>(role: &Role, model: &'a ModelInfo) -> Option<&'a CapabilitySpec> {
    let spec = model.spec_for(&role.capability)?;
    (role.inputs.iter().all(|i| spec.accepts.contains_key(i)) && spec.requires.iter().all(|i| role.inputs.contains(i))).then_some(spec)
}

/// Every enabled connection's models that could fill `role`. This is the model picker's list.
pub fn eligible<'a>(role: &Role, connections: &'a [Connection]) -> Vec<Resolved<'a>> {
    let mut out = Vec::new();
    for connection in connections.iter().filter(|c| c.enabled) {
        for model in &connection.models {
            if let Some(spec) = fits(role, model) {
                out.push(Resolved { connection, model, spec });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::fixtures::remote;

    fn cap(s: &str) -> CapabilityId {
        CapabilityId::parse(s).unwrap()
    }

    fn role() -> Role {
        Role {
            id: RoleId::new("generative-fill").unwrap(),
            label: "Generative Fill".into(),
            capability: cap("image.inpaint"),
            inputs: vec![InputRole::Source, InputRole::Mask],
        }
    }

    fn conn(id: &str, with_mask: bool) -> Connection {
        let mut spec = CapabilitySpec::new(cap("image.inpaint")).accepting(InputRole::Source, 1);
        if with_mask {
            spec = spec.accepting(InputRole::Mask, 1);
        }
        let mut c = remote(id);
        c.models = vec![ModelInfo::new(ModelId::new("m").unwrap(), "M").with(spec)];
        c
    }

    fn bind(conn: &str) -> Binding {
        Binding { role: RoleId::new("generative-fill").unwrap(), connection: ConnectionId::new(conn).unwrap(), model: ModelId::new("m").unwrap() }
    }

    #[test]
    fn a_broken_override_is_an_error_and_never_falls_through() {
        let conns = [conn("a", true)];
        let (over, def) = ([bind("gone")], [bind("a")]);
        let err = resolve(&role(), &[&over, &def], &conns).unwrap_err();
        assert!(matches!(err, ResolveError::ConnectionMissing(_)));
    }
}
