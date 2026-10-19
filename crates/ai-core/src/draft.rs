//! The form behind "add a connection": editable text in, a checked [`Connection`] out.
//!
//! The launcher's connections screen and the small fallback form inside an app both edit a
//! [`ConnectionDraft`], so the rules live once and the two screens cannot disagree:
//!
//! - **Where it goes decides whether it leaves the computer.** The locality is derived from the
//!   address (loopback means local), never chosen, so a remote service cannot be mislabelled
//!   local to skip the consent prompt.
//! - **A key is checked before it is kept.** One line of visible ASCII, no spaces. A pasted
//!   newline would otherwise reach an HTTP header.
//! - **Editing keeps what was not touched.** Leaving the key box empty keeps the saved key; the
//!   models saved with a connection are dropped when the address changes, because they belong to
//!   the old one. Models found by a test at the new address are kept, and a test that answers
//!   after the address moved on is dropped rather than credited to the address it finds.
//! - **Messages are for people.** Every [`DraftError`] names the field and says what to do, and
//!   never contains the key.
//!
//! Nothing here does I/O, so it compiles for wasm.

use std::fmt;

use crate::connection::{Auth, Billing, Connection, Locality};
use crate::endpoint::Endpoint;
use crate::ids::{ConnectionId, SecretRef, validate_id};
use crate::model::{CapabilitySpec, InputRole, ModelInfo};
use crate::secret::SecretString;
use crate::{CapabilityId, ModelId};

/// Longest label, in characters.
pub const MAX_LABEL_CHARS: usize = 80;
/// Longest address, in bytes.
pub const MAX_ENDPOINT_BYTES: usize = 2048;
/// Longest key, in bytes.
pub const MAX_KEY_BYTES: usize = 4096;

/// A starting point for the form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preset {
    /// Stable name, for tests and analytics-free bookkeeping.
    pub key: &'static str,
    /// What the button says.
    pub name: &'static str,
    pub provider: &'static str,
    /// Empty for "type your own".
    pub endpoint: &'static str,
    pub billing: Billing,
    pub needs_key: bool,
    /// One line about it, shown under the name.
    pub note: &'static str,
}

/// The services most people connect, in the order a form lists them.
pub const PRESETS: &[Preset] = &[
    Preset {
        key: "openai",
        name: "OpenAI",
        provider: "openai",
        endpoint: "https://api.openai.com/v1",
        billing: Billing::Metered,
        needs_key: true,
        note: "Your own OpenAI API key. Billed by OpenAI.",
    },
    Preset {
        key: "openrouter",
        name: "OpenRouter",
        provider: "openrouter",
        endpoint: "https://openrouter.ai/api/v1",
        billing: Billing::Metered,
        needs_key: true,
        note: "One key for many hosted models.",
    },
    Preset {
        key: "bfl",
        name: "Black Forest Labs",
        provider: "bfl",
        endpoint: "https://api.bfl.ai/v1",
        billing: Billing::Metered,
        needs_key: true,
        note: "FLUX 3 image and video generation. Billed by Black Forest Labs.",
    },
    Preset {
        key: "seedance",
        name: "Seedance (BytePlus ModelArk)",
        provider: "seedance",
        endpoint: "https://ark.ap-southeast.bytepluses.com/api/v3",
        billing: Billing::Metered,
        needs_key: true,
        note: "Seedance 2.0 text-to-video with your BytePlus ModelArk API key.",
    },
    Preset {
        key: "vertex-express",
        name: "Vertex AI (express mode)",
        provider: "vertex-express",
        endpoint: "https://aiplatform.googleapis.com/v1",
        billing: Billing::Metered,
        needs_key: true,
        note: "Gemini image generation. API key only; full Vertex uses different credentials. Key cannot be tested without a paid request.",
    },
    Preset {
        key: "artcraft-omni",
        name: "ArtCraft Omni API key",
        provider: "artcraft-omni",
        endpoint: "https://api.storyteller.ai",
        billing: Billing::Unknown,
        needs_key: true,
        note: "Staff-enabled API key for ArtCraft image and video generation. Subscription allowance is unverified; no free key test is documented.",
    },
    Preset {
        key: "ollama",
        name: "Ollama",
        provider: "ollama",
        endpoint: "http://127.0.0.1:11434/v1",
        billing: Billing::Free,
        needs_key: false,
        note: "Models running on this computer. Nothing leaves it.",
    },
    Preset {
        key: "lmstudio",
        name: "LM Studio",
        provider: "lmstudio",
        endpoint: "http://127.0.0.1:1234/v1",
        billing: Billing::Free,
        needs_key: false,
        note: "Models running on this computer. Nothing leaves it.",
    },
    Preset {
        key: "llamacpp",
        name: "llama.cpp",
        provider: "llamacpp",
        endpoint: "http://127.0.0.1:8080/v1",
        billing: Billing::Free,
        needs_key: false,
        note: "A llama.cpp server on this computer.",
    },
    Preset {
        key: "comfyui",
        name: "ComfyUI (local)",
        provider: "comfyui",
        endpoint: "http://127.0.0.1:8188",
        billing: Billing::Free,
        needs_key: false,
        note: "Local workflows on this computer. Import workflow JSON files first; each becomes a model.",
    },
    Preset {
        key: "custom",
        name: "Other (OpenAI compatible)",
        provider: "openai-compatible",
        endpoint: "",
        billing: Billing::Unknown,
        needs_key: true,
        note: "Any server that speaks the OpenAI API.",
    },
];

fn preset_models(preset: &Preset) -> Vec<ModelInfo> {
    if preset.key == "artcraft-omni" {
        return [("nano_banana", "Nano Banana (unverified)", "image.generate"), ("seedance_2p0", "Seedance 2.0 (unverified)", "video.generate")]
            .into_iter()
            .filter_map(|(id, label, capability)| {
                Some(
                    ModelInfo::new(ModelId::new(id).ok()?, label).with(
                        CapabilitySpec::new(CapabilityId::parse(capability).ok()?).accepting(InputRole::Prompt, 1).requiring(InputRole::Prompt),
                    ),
                )
            })
            .collect();
    }
    // Vertex express does not offer a known free key probe. Seed the documented model as
    // unverified so it can be bound without pretending the Test button has reached Google.
    if preset.key != "vertex-express" {
        return Vec::new();
    }
    match (ModelId::new("gemini-2.5-flash-image"), CapabilityId::parse("image.generate")) {
        (Ok(id), Ok(cap)) => vec![
            ModelInfo::new(id, "Gemini 2.5 Flash Image (unverified)")
                .with(CapabilitySpec::new(cap).accepting(InputRole::Prompt, 1).requiring(InputRole::Prompt)),
        ],
        _ => Vec::new(),
    }
}

/// Which box a problem belongs to, so a form can mark it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Label,
    Provider,
    Address,
    Key,
    /// The name of the header that carries the key.
    KeyHeader,
}

/// Why a draft was refused. The message is safe to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftError {
    pub field: Field,
    pub message: String,
}

impl DraftError {
    fn new(field: Field, message: impl Into<String>) -> Self {
        Self { field, message: message.into() }
    }
}

impl fmt::Display for DraftError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DraftError {}

/// What to do with the stored key.
#[derive(Clone, Debug)]
pub enum KeyChange {
    /// Leave whatever is saved.
    Keep,
    Set(SecretString),
    /// The connection no longer uses a key; remove the saved one.
    Clear,
}

/// A checked connection and what to do with its key.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub connection: Connection,
    pub key: KeyChange,
}

/// The form's state.
#[derive(Clone)]
pub struct ConnectionDraft {
    /// `Some` when editing an existing connection.
    pub editing: Option<ConnectionId>,
    pub label: String,
    pub provider: String,
    pub endpoint: String,
    pub billing: Billing,
    pub uses_key: bool,
    /// Text typed into the key box. Empty while editing means "keep the saved key". It is a plain
    /// string only because a text box needs one; [`Debug`] never prints it and [`prepare`]
    /// (Self::prepare) turns it into a [`SecretString`].
    pub key: String,
    /// Header that carries the key; empty sends it as `Authorization: Bearer`.
    pub key_header: String,
    pub enabled: bool,
    /// Models carried onto the saved connection. Assigning here declares them by hand and records
    /// no address, so they survive any address change; models a test found go through
    /// [`set_models`](Self::set_models), which records where they came from.
    pub models: Vec<ModelInfo>,
    /// Editing: the saved key's locator, and whether a key was saved at all.
    secret: Option<SecretRef>,
    /// The address `models` were loaded, preset or last set for.
    models_for: Option<String>,
}

impl Default for ConnectionDraft {
    fn default() -> Self {
        Self {
            editing: None,
            label: String::new(),
            provider: "openai-compatible".to_owned(),
            endpoint: String::new(),
            billing: Billing::Unknown,
            uses_key: true,
            key: String::new(),
            key_header: String::new(),
            enabled: true,
            models: Vec::new(),
            secret: None,
            models_for: None,
        }
    }
}

impl fmt::Debug for ConnectionDraft {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionDraft")
            .field("editing", &self.editing)
            .field("label", &self.label)
            .field("provider", &self.provider)
            .field("endpoint", &self.endpoint)
            .field("uses_key", &self.uses_key)
            .field("key_header", &self.key_header)
            .field("key", &format_args!("<{} chars>", self.key.chars().count()))
            .finish_non_exhaustive()
    }
}

impl ConnectionDraft {
    /// A new connection started from a preset.
    pub fn from_preset(preset: &Preset) -> Self {
        let models = preset_models(preset);
        Self {
            label: preset.name.to_owned(),
            provider: preset.provider.to_owned(),
            endpoint: preset.endpoint.to_owned(),
            billing: preset.billing,
            uses_key: preset.needs_key,
            models_for: (!models.is_empty()).then(|| preset.endpoint.trim().to_owned()),
            models,
            ..Self::default()
        }
    }

    /// Edits a saved connection. The key box starts empty.
    pub fn from_connection(c: &Connection) -> Self {
        Self {
            editing: Some(c.id.clone()),
            label: c.label.clone(),
            provider: c.provider.clone(),
            endpoint: c.endpoint.clone().unwrap_or_default(),
            billing: c.billing,
            uses_key: c.auth.secret_ref().is_some(),
            key: String::new(),
            key_header: c.key_header.clone().unwrap_or_default(),
            enabled: c.enabled,
            models: c.models.clone(),
            secret: c.auth.secret_ref().cloned(),
            models_for: c.endpoint.as_deref().map(|e| e.trim().to_owned()),
        }
    }

    /// Replaces `models` with what a test found, where `address` is the value the `endpoint`
    /// field held when that test was started — not the URL the test called, and not whatever the
    /// field holds now.
    ///
    /// A test is not instant and the address stays editable while it runs, so its answer can
    /// arrive after the address has changed, or after a later test at the new address has already
    /// answered. Recording it then would file one endpoint's models under another, which is what
    /// [`models_fit`](Self::models_fit) exists to prevent. An answer is therefore taken only while
    /// the field still holds the address it was asked of, and a late one is dropped. Returns
    /// whether the models were taken, so a caller can say "this result is out of date".
    ///
    /// A caller with the answer already in hand passes the address it used just the same; it
    /// cannot have moved on, so the call always takes.
    #[must_use]
    pub fn set_models(&mut self, address: &str, models: Vec<ModelInfo>) -> bool {
        if address.trim() != self.endpoint.trim() {
            return false;
        }
        self.models = models;
        self.models_for = Some(self.endpoint.trim().to_owned());
        true
    }

    /// Known model provenance must match the current address, including after repeated edits.
    /// A new custom draft may also have manually declared models without a prior address.
    fn models_fit(&self, address: &str) -> bool {
        self.models_for.as_deref().is_none_or(|origin| origin == address)
    }

    /// Where this would send data, as far as the address can tell. `None` while it is not a usable
    /// address, so a form can show the badge live.
    pub fn locality(&self) -> Option<Locality> {
        let endpoint = Endpoint::parse(self.endpoint.trim()).ok()?;
        Some(if endpoint.is_loopback() { Locality::Local } else { Locality::Remote })
    }

    /// Checks everything and builds the connection. `existing` is every saved connection, so new
    /// ids stay unique and an edit finds its original.
    pub fn prepare(&self, existing: &[Connection]) -> Result<Prepared, DraftError> {
        let label = self.label.trim();
        if label.is_empty() {
            return Err(DraftError::new(Field::Label, "Give the connection a name."));
        }
        if label.chars().count() > MAX_LABEL_CHARS || label.chars().any(char::is_control) {
            return Err(DraftError::new(Field::Label, format!("The name must be one line of at most {MAX_LABEL_CHARS} characters.")));
        }
        let provider = self.provider.trim();
        validate_id(provider).map_err(|e| DraftError::new(Field::Provider, format!("The service type is not valid ({e}).")))?;
        let address = self.endpoint.trim();
        if address.is_empty() {
            return Err(DraftError::new(Field::Address, "Enter the address of the server."));
        }
        if address.len() > MAX_ENDPOINT_BYTES {
            return Err(DraftError::new(Field::Address, "That address is too long."));
        }
        let parsed = Endpoint::parse(address).map_err(|e| {
            DraftError::new(Field::Address, format!("That address cannot be used ({e}). Use https://, or http:// only for this computer."))
        })?;
        let locality = if parsed.is_loopback() { Locality::Local } else { Locality::Remote };

        let typed = self.typed_key()?;
        let key_header = self.checked_key_header()?;

        let id = match &self.editing {
            Some(id) => {
                if !existing.iter().any(|c| &c.id == id) {
                    return Err(DraftError::new(Field::Label, "This connection was removed elsewhere. Close the form and start again."));
                }
                id.clone()
            }
            None => new_id(label, existing)?,
        };
        let (auth, key) = self.auth_and_key(typed, &id, existing)?;
        let connection = Connection {
            id,
            label: label.to_owned(),
            provider: provider.to_owned(),
            endpoint: Some(address.to_owned()),
            locality,
            auth,
            billing: self.billing,
            models: if self.models_fit(address) { self.models.clone() } else { Vec::new() },
            enabled: self.enabled,
            key_header,
        };
        connection.validate().map_err(|e| DraftError::new(Field::Address, format!("{e}.")))?;
        Ok(Prepared { connection, key })
    }

    /// The typed key, checked, or `None` if the box is empty.
    fn typed_key(&self) -> Result<Option<SecretString>, DraftError> {
        let text = self.key.trim();
        if text.is_empty() {
            return Ok(None);
        }
        check_key(text).map(|()| Some(SecretString::new(text)))
    }

    fn auth_and_key(&self, typed: Option<SecretString>, id: &ConnectionId, existing: &[Connection]) -> Result<(Auth, KeyChange), DraftError> {
        if !self.uses_key {
            let change = if self.secret.is_some() { KeyChange::Clear } else { KeyChange::Keep };
            return Ok((Auth::None, change));
        }
        let secret = match &self.secret {
            Some(s) => s.clone(),
            None => new_secret_ref(id, existing)?,
        };
        match typed {
            Some(key) => Ok((Auth::ApiKey { secret }, KeyChange::Set(key))),
            None if self.secret.is_some() => Ok((Auth::ApiKey { secret }, KeyChange::Keep)),
            None => Err(DraftError::new(Field::Key, "Paste the key. If this server needs none, turn off \"Uses a key\".")),
        }
    }
}

/// One line of visible ASCII with no spaces: what an HTTP bearer token can be.
pub fn check_key(text: &str) -> Result<(), DraftError> {
    if text.len() > MAX_KEY_BYTES {
        return Err(DraftError::new(Field::Key, "That key is far too long to be a key."));
    }
    if !text.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err(DraftError::new(Field::Key, "A key is one line of letters, digits and symbols with no spaces. Copy it again."));
    }
    Ok(())
}

use ids::{new_id, new_secret_ref};

mod custom;
mod ids;

#[cfg(test)]
mod tests;
