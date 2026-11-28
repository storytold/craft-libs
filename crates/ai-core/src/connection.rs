//! Connections: places work can be sent, and how to get in.
//!
//! A [`Connection`] is saved once (in the launcher) and used by every app. It records where it is
//! (`endpoint`), whether data stays on this machine (`locality`), how it authenticates (`auth`),
//! how it bills (`billing`) and which models it offers. Secrets are never stored here: `auth`
//! holds a [`SecretRef`], a locator into a keychain or encrypted file.
//!
//! [`Connection::validate`] enforces the rules consent depends on, chiefly that a connection
//! cannot be labelled `Local` unless it really is loopback or in-process, so a mislabel cannot
//! skip the consent prompt.

use serde::{Deserialize, Serialize};

use crate::endpoint::Endpoint;
use crate::ids::{ConnectionId, IdError, SecretRef, validate_id};
use crate::model::ModelInfo;

/// Does data leave this machine?
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Locality {
    /// Loopback server or in-process model: nothing leaves the machine.
    Local,
    /// The model runs on another machine. Consent applies.
    Remote,
}

/// How a connection proves who is calling.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Auth {
    /// No credentials (a local model, an open local server).
    None,
    /// Authenticates with a stored key, sent in a header (see `Connection::key_header`).
    ApiKey {
        /// Locator of the key.
        secret: SecretRef,
    },
    /// An official subscription route (for example the Codex app-server). `route` names it.
    Subscription {
        /// Name of the subscription route.
        route: String,
        /// Locator of the credential for the route.
        secret: SecretRef,
    },
}

impl Auth {
    /// The secret locator this authentication uses, or `None` when it needs no credentials.
    pub fn secret_ref(&self) -> Option<&SecretRef> {
        match self {
            Self::None => None,
            Self::ApiKey { secret } | Self::Subscription { secret, .. } => Some(secret),
        }
    }
}

/// Who pays, which decides what a failure like "quota" means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Billing {
    /// No charge.
    Free,
    /// Pay per use, billed separately.
    Metered,
    /// Covered by an existing subscription's allowance.
    SubscriptionAllowance,
    /// The cost is not known.
    Unknown,
}

/// A saved connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    #[allow(missing_docs)]
    pub id: ConnectionId,
    /// Name shown to the user.
    pub label: String,
    /// Which adapter talks to it, such as `http`, `openai`, `fal`, `comfyui`, `onnx`.
    pub provider: String,
    /// Server address, such as `https://host:port`. `None` for an in-process or route-only connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// Whether data stays on this machine. `validate` checks it against `endpoint`.
    pub locality: Locality,
    /// How the connection authenticates.
    pub auth: Auth,
    /// Who pays for use.
    pub billing: Billing,
    /// Models this connection offers. Ids must be unique.
    #[serde(default)]
    pub models: Vec<ModelInfo>,
    /// When false, the connection is kept but not offered for use. Defaults to true.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Header that carries the key, for a server that does not use `Authorization: Bearer`.
    /// `None` is the bearer default. The key is sent as the header's whole value. See
    /// [`check_key_header`] for the names allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_header: Option<String>,
}

fn yes() -> bool {
    true
}

/// Longest header name accepted for a key.
pub const MAX_KEY_HEADER_BYTES: usize = 64;

/// Names a key must never be sent under: they steer the transport itself, or name a different
/// credential, or are what the default already does.
const RESERVED_HEADERS: &[&str] = &[
    "authorization",
    "host",
    "content-length",
    "content-type",
    "content-encoding",
    "transfer-encoding",
    "connection",
    "cookie",
    "set-cookie",
    "upgrade",
    "te",
    "trailer",
    "expect",
    "user-agent",
    "accept",
];

/// Whether `name` may carry a key. A short HTTP token (letters, digits and `-_.`), not one of the
/// transport's own headers and not a proxy header. Leave the name empty (`None`) for
/// `Authorization: Bearer`.
pub fn check_key_header(name: &str) -> Result<(), &'static str> {
    if name.is_empty() || name.len() > MAX_KEY_HEADER_BYTES {
        return Err("a header name is 1 to 64 characters");
    }
    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')) {
        return Err("a header name uses only letters, digits, - _ and .");
    }
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("proxy-") || lower.starts_with("sec-") {
        return Err("that header belongs to the network, not to a key");
    }
    if RESERVED_HEADERS.contains(&lower.as_str()) {
        return Err(if lower == "authorization" {
            "leave the name empty to send the key as Authorization: Bearer"
        } else {
            "that header cannot carry a key"
        });
    }
    Ok(())
}

/// Why a connection was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionError {
    /// The provider name is not a valid id.
    Provider(IdError),
    /// The endpoint cannot be parsed.
    Endpoint(IdError),
    /// Labelled `Local` but the endpoint is not loopback.
    LocalButNotLoopback,
    /// Labelled `Remote` but the endpoint is loopback (harmless, but a sign of a wrong setup).
    RemoteButLoopback,
    /// No endpoint, labelled `Local`, yet it uses credentials. Such a route sends data away.
    LocalWithoutEndpointNeedsNoAuth,
    /// The label is empty or only whitespace.
    EmptyLabel,
    /// Two models share this id.
    DuplicateModel(String),
    /// A key header name that is not allowed, or a header named for a connection with no key.
    KeyHeader(&'static str),
}

impl std::fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provider(e) => write!(f, "provider name: {e}"),
            Self::Endpoint(e) => write!(f, "endpoint: {e}"),
            Self::LocalButNotLoopback => f.write_str("marked local, but the endpoint is not a loopback address"),
            Self::RemoteButLoopback => f.write_str("marked remote, but the endpoint is a loopback address"),
            Self::LocalWithoutEndpointNeedsNoAuth => f.write_str("a connection with no endpoint can only be local if it needs no credentials"),
            Self::EmptyLabel => f.write_str("label is empty"),
            Self::DuplicateModel(m) => write!(f, "model {m} is listed twice"),
            Self::KeyHeader(why) => write!(f, "key header: {why}"),
        }
    }
}

impl std::error::Error for ConnectionError {}

impl Connection {
    /// The parsed endpoint, if any.
    pub fn parsed_endpoint(&self) -> Option<Result<Endpoint, IdError>> {
        self.endpoint.as_deref().map(Endpoint::parse)
    }

    /// Checks the rules that consent and safety rely on.
    pub fn validate(&self) -> Result<(), ConnectionError> {
        if self.label.trim().is_empty() {
            return Err(ConnectionError::EmptyLabel);
        }
        validate_id(&self.provider).map_err(ConnectionError::Provider)?;
        match self.parsed_endpoint() {
            Some(parsed) => {
                let loopback = parsed.map_err(ConnectionError::Endpoint)?.is_loopback();
                match (self.locality, loopback) {
                    (Locality::Local, false) => return Err(ConnectionError::LocalButNotLoopback),
                    (Locality::Remote, true) => return Err(ConnectionError::RemoteButLoopback),
                    _ => {}
                }
            }
            None => {
                if self.locality == Locality::Local && self.auth != Auth::None {
                    return Err(ConnectionError::LocalWithoutEndpointNeedsNoAuth);
                }
            }
        }
        if let Some(name) = &self.key_header {
            check_key_header(name).map_err(ConnectionError::KeyHeader)?;
            if self.auth.secret_ref().is_none() {
                return Err(ConnectionError::KeyHeader("a header name needs a connection that uses a key"));
            }
        }
        let mut seen = std::collections::HashSet::with_capacity(self.models.len());
        for m in &self.models {
            if !seen.insert(&m.id) {
                return Err(ConnectionError::DuplicateModel(m.id.to_string()));
            }
        }
        Ok(())
    }

    /// The model with this id, if the connection lists it.
    pub fn model(&self, id: &crate::ids::ModelId) -> Option<&ModelInfo> {
        self.models.iter().find(|m| &m.id == id)
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    pub fn remote(id: &str) -> Connection {
        Connection {
            id: ConnectionId::new(id).unwrap(),
            label: "Remote".into(),
            provider: "http".into(),
            endpoint: Some("https://api.example.com".into()),
            locality: Locality::Remote,
            auth: Auth::ApiKey { secret: SecretRef::new(format!("secret-{id}")).unwrap() },
            billing: Billing::Metered,
            models: Vec::new(),
            enabled: true,
            key_header: None,
        }
    }
}
