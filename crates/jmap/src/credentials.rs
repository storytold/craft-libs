//! Credentials: HTTP Basic (RFC 7617) and Bearer tokens (RFC 6750).
//!
//! Credentials live in memory only. Their `Debug` form names the scheme and nothing else: no secret and no
//! user name, which is usually an e-mail address and so personal data. The memory holding a secret is
//! overwritten when it is dropped; this is best effort (the allocator, the operating system or an earlier
//! copy made by the caller are out of reach without `unsafe`), not a guarantee.

use std::fmt;

use crate::redact::Redactor;

/// The longest user name, password or token accepted, in octets.
pub const MAX_CREDENTIAL_LEN: usize = 8 * 1024;

/// A piece of text that must not be shown, and that is overwritten when dropped.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Secret(String);

impl Secret {
    pub(crate) fn new(text: String) -> Self {
        Self(text)
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        let mut bytes = std::mem::take(&mut self.0).into_bytes();
        bytes.iter_mut().for_each(|b| *b = 0);
        std::hint::black_box(&bytes);
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Why credentials were refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialsError {
    /// The user name is empty.
    EmptyUser,
    /// The token is empty.
    EmptyToken,
    /// The user name holds a colon, which Basic cannot carry (RFC 7617 §2).
    ColonInUser,
    /// A control character in the user name or password (RFC 7617 §2).
    ControlCharacter,
    /// The token holds a character RFC 6750 §2.1 does not allow.
    InvalidToken,
    /// Longer than [`MAX_CREDENTIAL_LEN`].
    TooLong,
}

impl fmt::Display for CredentialsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::EmptyUser => "the user name is empty; enter the user name of the mail account",
            Self::EmptyToken => "the access token is empty; sign in again to obtain one",
            Self::ColonInUser => {
                "the user name contains a colon (:), which password sign-in cannot carry; use a token instead, or the user name without it"
            }
            Self::ControlCharacter => "the user name or password contains an invisible control character, often from a paste; type it again",
            Self::InvalidToken => {
                "the access token contains a space, a comma or another character a token cannot hold; copy it again without extra characters"
            }
            Self::TooLong => "the user name, password or token is longer than 8 KiB, more than this program accepts; check what was pasted",
        })
    }
}

impl std::error::Error for CredentialsError {}

#[derive(Clone)]
enum Kind {
    Basic { user: Secret, password: Secret },
    Bearer { token: Secret },
}

/// Credentials for the JMAP server.
#[derive(Clone)]
pub struct Credentials {
    kind: Kind,
    authorization: Secret,
}

const BASE64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_char(index: u8) -> char {
    char::from(BASE64_ALPHABET.get(usize::from(index & 0x3F)).copied().unwrap_or(b'A'))
}

/// Base64 with padding (RFC 4648 §4), written into `out`.
fn base64_into(input: &[u8], out: &mut String) {
    for chunk in input.chunks(3) {
        let (a, b, c) = match *chunk {
            [a, b, c] => (a, Some(b), Some(c)),
            [a, b] => (a, Some(b), None),
            [a] => (a, None, None),
            _ => continue,
        };
        let b0 = b.unwrap_or(0);
        let c0 = c.unwrap_or(0);
        out.push(base64_char(a >> 2));
        out.push(base64_char(((a & 0x03) << 4) | (b0 >> 4)));
        out.push(if b.is_some() { base64_char(((b0 & 0x0F) << 2) | (c0 >> 6)) } else { '=' });
        out.push(if c.is_some() { base64_char(c0 & 0x3F) } else { '=' });
    }
}

/// The part an octet may play in a bearer token (RFC 6750 §2.1, `b64token`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TokenOctet {
    /// A letter, a digit, or one of `-` `.` `_` `~` `+` `/`: allowed anywhere before the padding.
    Body,
    /// `=`: allowed only at the end, any number of times.
    Padding,
    /// Anything else, spaces, commas, quotes, controls and non-ASCII octets among them.
    Refused,
}

/// Sorts one octet into its [`TokenOctet`] part.
const fn token_octet(octet: u8) -> TokenOctet {
    match octet {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' => TokenOctet::Body,
        b'-' | b'.' | b'_' | b'~' | b'+' | b'/' => TokenOctet::Body,
        b'=' => TokenOctet::Padding,
        _ => TokenOctet::Refused,
    }
}

/// Reads a bearer token in one pass: at least one body octet, then nothing but padding once padding began.
fn is_bearer_token(token: &str) -> bool {
    let mut body_octets = 0usize;
    let mut padding_started = false;
    for octet in token.bytes() {
        match (token_octet(octet), padding_started) {
            (TokenOctet::Body, false) => body_octets = body_octets.saturating_add(1),
            (TokenOctet::Padding, _) => padding_started = true,
            (TokenOctet::Body, true) | (TokenOctet::Refused, _) => return false,
        }
    }
    body_octets > 0
}

impl Credentials {
    /// Basic credentials (RFC 7617 §2): `user:password` in UTF-8, base64-encoded.
    ///
    /// The user name may not be empty or hold a colon; neither part may hold a control character. The text is
    /// sent as given: Unicode normalisation (RFC 7617 §2.1 mentions NFC) is left to the caller.
    pub fn basic(user: &str, password: &str) -> Result<Self, CredentialsError> {
        if user.is_empty() {
            return Err(CredentialsError::EmptyUser);
        }
        if user.len() > MAX_CREDENTIAL_LEN || password.len() > MAX_CREDENTIAL_LEN {
            return Err(CredentialsError::TooLong);
        }
        if user.contains(':') {
            return Err(CredentialsError::ColonInUser);
        }
        if user.chars().chain(password.chars()).any(char::is_control) {
            return Err(CredentialsError::ControlCharacter);
        }
        let mut pair = Vec::with_capacity(user.len().saturating_add(password.len()).saturating_add(1));
        pair.extend_from_slice(user.as_bytes());
        pair.push(b':');
        pair.extend_from_slice(password.as_bytes());
        let encoded_len = pair.len().div_ceil(3).saturating_mul(4);
        let mut header = String::with_capacity(encoded_len.saturating_add(6));
        header.push_str("Basic ");
        base64_into(&pair, &mut header);
        pair.iter_mut().for_each(|b| *b = 0);
        std::hint::black_box(&pair);
        Ok(Self {
            kind: Kind::Basic { user: Secret::new(user.to_owned()), password: Secret::new(password.to_owned()) },
            authorization: Secret::new(header),
        })
    }

    /// Bearer credentials (RFC 6750 §2.1). The token must match `b64token`.
    pub fn bearer(token: &str) -> Result<Self, CredentialsError> {
        if token.is_empty() {
            return Err(CredentialsError::EmptyToken);
        }
        if token.len() > MAX_CREDENTIAL_LEN {
            return Err(CredentialsError::TooLong);
        }
        if !is_bearer_token(token) {
            return Err(CredentialsError::InvalidToken);
        }
        let mut header = String::with_capacity(token.len().saturating_add(7));
        header.push_str("Bearer ");
        header.push_str(token);
        Ok(Self { kind: Kind::Bearer { token: Secret::new(token.to_owned()) }, authorization: Secret::new(header) })
    }

    /// The authentication scheme's name: `Basic` or `Bearer`.
    pub fn scheme_name(&self) -> &'static str {
        match self.kind {
            Kind::Basic { .. } => "Basic",
            Kind::Bearer { .. } => "Bearer",
        }
    }

    /// The value of the `Authorization` header field.
    pub(crate) fn authorization(&self) -> &Secret {
        &self.authorization
    }

    /// Teaches a redactor every secret these credentials hold.
    pub fn register_secrets(&self, redactor: &mut Redactor) {
        match &self.kind {
            Kind::Basic { user, password } => {
                redactor.add_personal(user.expose());
                redactor.add(password.expose());
            }
            Kind::Bearer { token } => redactor.add(token.expose()),
        }
        let header = self.authorization.expose();
        let value = header.split_once(' ').map_or(header, |(_, value)| value);
        redactor.add(value);
        if let Kind::Basic { .. } = self.kind {
            // The base64 forms a server might echo: without padding, and in the URL-safe alphabet (RFC 4648 §5).
            let unpadded = value.trim_end_matches('=');
            redactor.add(unpadded);
            let url_safe = Secret::new(unpadded.replace('+', "-").replace('/', "_"));
            redactor.add(url_safe.expose());
        }
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Credentials({}, <redacted>)", self.scheme_name())
    }
}

#[cfg(test)]
mod tests;
