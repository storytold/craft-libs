//! Turning what a person types into the JMAP session address to fetch (requirement 1.6, RFC 8620 §2.2).
//!
//! A person may type `example.com`, `example.com/jmap/session`, `example.com:8443`, `https://example.com` or a
//! full address. The result defaults to `https`, and to [`WELL_KNOWN_PATH`] when only a host was given. Plain
//! `http` is kept only when it was typed and the host is loopback (requirement 1.2); it is never chosen by
//! this function, and `http` typed for a remote host is refused rather than silently upgraded, so that the
//! person sees that what they typed cannot be used.

use crate::address::{Address, AddressError, MAX_ADDRESS_LEN};

/// The well-known path of the JMAP session resource (RFC 8620 §2.2, registered by §9.2).
pub const WELL_KNOWN_PATH: &str = "/.well-known/jmap";

/// Whether `text` begins with `scheme://`, the scheme being well formed (RFC 3986 §3.1).
fn has_explicit_scheme(text: &str) -> bool {
    match text.split_once("://") {
        Some((scheme, _)) => {
            let mut bytes = scheme.bytes();
            matches!(bytes.next(), Some(b) if b.is_ascii_alphabetic()) && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
        }
        None => false,
    }
}

/// Whether `text` starts with `http:` or `https:` without the `//` that must follow (`https:example.com`).
fn has_scheme_without_slashes(text: &str) -> bool {
    let lower = text.get(..6).unwrap_or(text).to_ascii_lowercase();
    (lower.starts_with("http:") || lower.starts_with("https:")) && !text.contains("://")
}

/// Turns a session address typed by a person into the address to fetch.
///
/// Surrounding white space is ignored; anything else that is not a valid address is refused with the same
/// errors as [`Address::parse`]. The result always satisfies the transport rule.
pub fn session_address(input: &str) -> Result<Address, AddressError> {
    let trimmed = input.trim_matches(|c: char| c.is_ascii_whitespace());
    if trimmed.is_empty() {
        return Err(AddressError::Empty);
    }
    if trimmed.len() > MAX_ADDRESS_LEN {
        return Err(AddressError::TooLong);
    }
    if has_scheme_without_slashes(trimmed) {
        return Err(AddressError::MissingAuthority);
    }
    let address = if has_explicit_scheme(trimmed) {
        Address::parse(trimmed)?
    } else {
        let mut with_scheme = String::with_capacity(trimmed.len().saturating_add(8));
        with_scheme.push_str("https://");
        with_scheme.push_str(trimmed);
        Address::parse(&with_scheme)?
    };
    let address = if address.path() == "/" && address.query().is_none() { address.with_path(WELL_KNOWN_PATH)? } else { address };
    address.require_secure_transport()?;
    Ok(address)
}

#[cfg(test)]
mod tests;
