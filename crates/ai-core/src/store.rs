//! The shared store: the versioned document the launcher writes and every app reads.
//!
//! This is the contract between the launcher and the apps. It holds connections (with secret
//! *references*, never secrets), default bindings, and per-app override bindings. Rules:
//!
//! - **Versioned.** `schema_version` is checked first. A file from a newer schema is refused with
//!   a clear error so an old reader never rewrites a new file.
//! - **Tolerant.** Unknown fields are ignored, so a newer writer can add fields without breaking
//!   an older reader of the same major version.
//! - **Hostile-input safe.** Input is size-capped, every id and endpoint is validated, and a bad
//!   file is an error, not a panic.
//! - **No secrets.** There is no field that can hold one.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::connection::{Connection, ConnectionError};
use crate::ids::{AppId, ConnectionId, RoleId};
use crate::role::{Binding, ResolveError, Resolved, Role, resolve};

/// The schema this build reads and writes.
pub const SCHEMA_VERSION: u32 = 1;
/// Largest store file accepted, in bytes.
pub const MAX_STORE_BYTES: usize = 1024 * 1024;

/// One app's own settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSettings {
    /// Bindings that override the launcher defaults for this app's roles.
    #[serde(default)]
    pub overrides: Vec<Binding>,
}

/// The whole store.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreFile {
    pub schema_version: u32,
    #[serde(default)]
    pub connections: Vec<Connection>,
    /// Default bindings, set in the launcher, used by every app unless it overrides.
    #[serde(default)]
    pub defaults: Vec<Binding>,
    #[serde(default)]
    pub apps: BTreeMap<AppId, AppSettings>,
    /// User-declared tool contracts, shared across the suite. A declaration does not install an adapter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_tools: Vec<Role>,
}

/// Why a store file was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    TooLarge {
        len: usize,
        max: usize,
    },
    Syntax(String),
    /// Written by a newer version. Do not write to it.
    TooNew {
        found: u32,
        supported: u32,
    },
    BadVersion(u32),
    DuplicateConnection(ConnectionId),
    DuplicateTool(RoleId),
    TooManyTools(usize),
    InvalidTool(RoleId),
    Invalid {
        connection: ConnectionId,
        why: ConnectionError,
    },
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge { len, max } => {
                write!(f, "store file is {len} bytes, the limit is {max}")
            }
            Self::Syntax(e) => write!(f, "store file is not valid: {e}"),
            Self::TooNew { found, supported } => {
                write!(f, "store file is schema {found}, this version understands up to {supported}; update the app")
            }
            Self::BadVersion(v) => write!(f, "store file has schema version {v}"),
            Self::DuplicateConnection(id) => write!(f, "connection {id} appears twice"),
            Self::DuplicateTool(id) => write!(f, "tool {id} appears twice"),
            Self::TooManyTools(count) => write!(f, "{count} custom tools exceeds the limit of 64"),
            Self::InvalidTool(id) => write!(f, "tool {id} needs a short name and valid inputs"),
            Self::Invalid { connection, why } => write!(f, "connection {connection}: {why}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl Default for StoreFile {
    fn default() -> Self {
        Self { schema_version: SCHEMA_VERSION, connections: Vec::new(), defaults: Vec::new(), apps: BTreeMap::new(), custom_tools: Vec::new() }
    }
}

impl StoreFile {
    /// Parses and validates a store file.
    pub fn parse(text: &str) -> Result<Self, StoreError> {
        if text.len() > MAX_STORE_BYTES {
            return Err(StoreError::TooLarge { len: text.len(), max: MAX_STORE_BYTES });
        }
        // Read the version alone first, so a newer file is refused even if its other fields no
        // longer fit this build's types.
        #[derive(Deserialize)]
        struct Head {
            schema_version: u32,
        }
        let head: Head = serde_json::from_str(text).map_err(|e| StoreError::Syntax(e.to_string()))?;
        if head.schema_version == 0 {
            return Err(StoreError::BadVersion(0));
        }
        if head.schema_version > SCHEMA_VERSION {
            return Err(StoreError::TooNew { found: head.schema_version, supported: SCHEMA_VERSION });
        }
        let store: Self = serde_json::from_str(text).map_err(|e| StoreError::Syntax(e.to_string()))?;
        store.validate()?;
        Ok(store)
    }

    /// Checks every connection and custom tool before anything is written.
    pub fn validate(&self) -> Result<(), StoreError> {
        if self.custom_tools.len() > 64 {
            return Err(StoreError::TooManyTools(self.custom_tools.len()));
        }
        for (i, tool) in self.custom_tools.iter().enumerate() {
            if self.custom_tools.iter().skip(i + 1).any(|other| other.id == tool.id) {
                return Err(StoreError::DuplicateTool(tool.id.clone()));
            }
            if tool.label.trim().is_empty()
                || tool.label.len() > 120
                || tool.label.chars().any(char::is_control)
                || tool.inputs.is_empty()
                || tool.inputs.len() > 8
                || tool.inputs.iter().enumerate().any(|(i, input)| tool.inputs.iter().skip(i + 1).any(|other| other == input))
            {
                return Err(StoreError::InvalidTool(tool.id.clone()));
            }
        }
        for (i, c) in self.connections.iter().enumerate() {
            if self.connections.iter().skip(i + 1).any(|o| o.id == c.id) {
                return Err(StoreError::DuplicateConnection(c.id.clone()));
            }
            c.validate().map_err(|why| StoreError::Invalid { connection: c.id.clone(), why })?;
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<String, StoreError> {
        serde_json::to_string_pretty(self).map_err(|e| StoreError::Syntax(e.to_string()))
    }

    /// A custom id is one suite-wide contract. An app may localise its label, but cannot change
    /// the capability or input set while using the same id.
    pub fn check_tool(&self, role: &Role) -> Result<(), ResolveError> {
        if let Some(saved) = self.custom_tools.iter().find(|tool| tool.id == role.id)
            && (saved.capability != role.capability
                || saved.inputs.len() != role.inputs.len()
                || !saved.inputs.iter().all(|input| role.inputs.contains(input)))
        {
            return Err(ResolveError::ToolMismatch(role.id.clone()));
        }
        Ok(())
    }

    /// Resolves an app's role through its overrides, then the launcher defaults.
    pub fn resolve(&self, app: &AppId, role: &Role) -> Result<Resolved<'_>, ResolveError> {
        self.check_tool(role)?;
        let empty: &[Binding] = &[];
        let overrides = self.apps.get(app).map_or(empty, |a| a.overrides.as_slice());
        resolve(role, &[overrides, self.defaults.as_slice()], &self.connections)
    }

    /// Bindings that name a connection that no longer exists, for the launcher to clean up.
    pub fn dangling_bindings(&self) -> Vec<&Binding> {
        let known = |b: &&Binding| self.connections.iter().any(|c| c.id == b.connection);
        let overrides = self.apps.values().flat_map(|a| a.overrides.iter());
        self.defaults.iter().chain(overrides).filter(|b| !known(b)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilityId;
    use crate::connection::fixtures::remote;
    use crate::ids::{ModelId, RoleId};
    use crate::model::{CapabilitySpec, InputRole, ModelInfo};

    fn cap() -> CapabilityId {
        CapabilityId::parse("text.complete").unwrap()
    }

    fn store() -> StoreFile {
        let mut c = remote("a");
        c.models = vec![ModelInfo::new(ModelId::new("m").unwrap(), "M").with(CapabilitySpec::new(cap()).accepting(InputRole::Prompt, 1))];
        let binding = Binding { role: RoleId::new("rewrite").unwrap(), connection: c.id.clone(), model: ModelId::new("m").unwrap() };
        StoreFile { connections: vec![c], defaults: vec![binding], ..StoreFile::default() }
    }

    fn role() -> Role {
        Role { id: RoleId::new("rewrite").unwrap(), label: "Rewrite".into(), capability: cap(), inputs: vec![InputRole::Prompt] }
    }

    #[test]
    fn round_trips() {
        let s = store();
        assert_eq!(StoreFile::parse(&s.to_json().unwrap()).unwrap(), s);
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let mut v: serde_json::Value = serde_json::from_str(&store().to_json().unwrap()).unwrap();
        v["added_in_a_later_release"] = serde_json::json!({"x": 1});
        v["connections"][0]["new_flag"] = serde_json::json!(true);
        assert!(StoreFile::parse(&v.to_string()).is_ok());
    }

    #[test]
    fn a_newer_schema_is_refused_even_if_its_shape_changed() {
        let text = r#"{"schema_version": 2, "connections": "now a different shape"}"#;
        assert_eq!(StoreFile::parse(text), Err(StoreError::TooNew { found: 2, supported: SCHEMA_VERSION }));
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        for text in
            ["", "not json", "[]", "{}", r#"{"schema_version": 0}"#, r#"{"schema_version": -1}"#, r#"{"schema_version": 1, "connections": [1]}"#]
        {
            assert!(StoreFile::parse(text).is_err(), "{text:?}");
        }
        let big = format!("{{\"schema_version\":1,\"pad\":\"{}\"}}", "x".repeat(MAX_STORE_BYTES));
        assert!(matches!(StoreFile::parse(&big), Err(StoreError::TooLarge { .. })));
    }

    #[test]
    fn a_mislabelled_connection_is_refused_at_load() {
        let mut s = store();
        s.connections[0].locality = crate::connection::Locality::Local;
        let err = StoreFile::parse(&s.to_json().unwrap()).unwrap_err();
        assert!(matches!(err, StoreError::Invalid { .. }));
    }

    #[test]
    fn duplicate_connections_are_refused() {
        let mut s = store();
        s.connections.push(s.connections[0].clone());
        assert!(matches!(s.validate(), Err(StoreError::DuplicateConnection(_))));
    }

    #[test]
    fn resolve_uses_default_then_app_override() {
        let mut s = store();
        let app = AppId::new("wordcraft").unwrap();
        assert_eq!(s.resolve(&app, &role()).unwrap().connection.id.as_str(), "a");
        let mut other = remote("b");
        other.models = s.connections[0].models.clone();
        s.connections.push(other);
        let over = Binding { role: RoleId::new("rewrite").unwrap(), connection: ConnectionId::new("b").unwrap(), model: ModelId::new("m").unwrap() };
        s.apps.insert(app.clone(), AppSettings { overrides: vec![over] });
        assert_eq!(s.resolve(&app, &role()).unwrap().connection.id.as_str(), "b");
        assert_eq!(s.resolve(&AppId::new("photocraft").unwrap(), &role()).unwrap().connection.id.as_str(), "a");
    }

    #[test]
    fn dangling_bindings_are_reported() {
        let mut s = store();
        s.connections.clear();
        assert_eq!(s.dangling_bindings().len(), 1);
    }

    #[test]
    fn truncated_and_mutated_files_never_panic() {
        let text = store().to_json().unwrap();
        for end in 0..=text.len() {
            if text.is_char_boundary(end) {
                let _ = StoreFile::parse(&text[..end]);
            }
        }
        let bytes = text.as_bytes();
        for i in (0..bytes.len()).step_by(7) {
            let mut copy = bytes.to_vec();
            copy[i] = copy[i].wrapping_add(1);
            if let Ok(s) = String::from_utf8(copy) {
                let _ = StoreFile::parse(&s);
            }
        }
    }

    #[test]
    fn serialized_store_contains_no_secret_values() {
        let json = store().to_json().unwrap();
        assert!(json.contains("secret-a"), "the reference is present");
        for needle in ["sk-", "\"api_key\":", "\"token\":", "\"password\":", "\"key\":"] {
            assert!(!json.contains(needle), "{needle}");
        }
    }
}
