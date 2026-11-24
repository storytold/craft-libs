//! Ids and key locators for new connections: readable, unique among the saved ones.

use super::{DraftError, Field};
use crate::connection::Connection;
use crate::ids::{ConnectionId, SecretRef};

/// A lowercase, dash-separated name made from a label.
fn slug(label: &str) -> String {
    let mut out = String::new();
    for c in label.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 40 {
            break;
        }
    }
    let out = out.trim_end_matches('-');
    if out.is_empty() { "connection".to_owned() } else { out.to_owned() }
}

/// The locator for a new connection's key: its id plus `.key`, so it is recognisable in a keychain
/// listing and two connections can never share a key. (Ids are unique, so this is too, unless an
/// older file used the same text for something else; then it gets a number.)
pub(super) fn new_secret_ref(id: &ConnectionId, existing: &[Connection]) -> Result<SecretRef, DraftError> {
    let used = |s: &str| existing.iter().any(|c| c.auth.secret_ref().is_some_and(|r| r.as_str() == s));
    let mut candidate = format!("{id}.key");
    let mut n = 2u32;
    while used(&candidate) {
        candidate = format!("{id}.key{n}");
        n = n.saturating_add(1);
    }
    SecretRef::new(candidate).map_err(|_| DraftError::new(Field::Label, "That name cannot be used for the saved key."))
}

/// An id derived from the label that no saved connection uses.
pub(super) fn new_id(label: &str, existing: &[Connection]) -> Result<ConnectionId, DraftError> {
    let base = slug(label);
    let taken = |s: &str| existing.iter().any(|c| c.id.as_str() == s);
    let mut candidate = base.clone();
    let mut n = 2u32;
    while taken(&candidate) {
        candidate = format!("{base}-{n}");
        n = n.saturating_add(1);
    }
    ConnectionId::new(candidate).map_err(|_| DraftError::new(Field::Label, "That name cannot be turned into an id."))
}
