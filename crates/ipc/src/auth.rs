//! Session tokens and the peer user check.
//!
//! On Unix the peer check is the effective uid from `interprocess`. A missing
//! credential is a refusal. On Windows, `interprocess` 2.4.4 `PeerCreds` only
//! exposes a pid. The pipe SDDL is the user check there. A pid is not a user.

use interprocess::local_socket::Stream;
#[cfg(unix)]
use interprocess::local_socket::traits::StreamCommon;

#[cfg(unix)]
use crate::discovery::current_euid;
use crate::error::LinkError;

const HEX: &[u8; 16] = b"0123456789abcdef";

pub fn encode_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

pub fn is_lower_hex(text: &str) -> bool {
    text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// 32 random bytes as 64 lowercase hex characters.
pub fn generate_token() -> Result<String, LinkError> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(LinkError::io)?;
    Ok(encode_hex(&bytes))
}

/// UUID version 4. The instance record stores this string.
pub fn random_uuid() -> Result<String, LinkError> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(LinkError::io)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = encode_hex(&bytes);
    Ok(format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32]))
}

/// Compare two tokens without stopping at the first different byte.
/// Length is public (always 64). A different length is not equal.
pub fn tokens_equal(expected: &str, presented: &str) -> bool {
    if expected.len() != 64 || presented.len() != 64 {
        return false;
    }
    let mut diff = 0u8;
    for (left, right) in expected.bytes().zip(presented.bytes()) {
        diff |= left ^ right;
    }
    diff == 0
}

/// `Ok` when this Unix peer is the current effective user.
/// Missing credentials are `Err(Auth)`, not a pass.
///
/// Windows returns `Ok`. The named-pipe SDDL admits only the owner and SYSTEM.
/// This function cannot see a Windows user id through interprocess 2.4.4.
pub fn ensure_peer_user(stream: &Stream) -> Result<(), LinkError> {
    #[cfg(unix)]
    {
        let creds = stream.peer_creds().map_err(|_| LinkError::Auth)?;
        let Some(peer) = creds.euid() else {
            return Err(LinkError::Auth);
        };
        let me = current_euid().map_err(|_| LinkError::Auth)?;
        if peer == me { Ok(()) } else { Err(LinkError::Auth) }
    }
    #[cfg(not(unix))]
    {
        let _ = stream;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_64_lower_hex_and_compare_is_exact() {
        let token = generate_token().unwrap();
        assert_eq!(token.len(), 64);
        assert!(is_lower_hex(&token));
        assert!(tokens_equal(&token, &token));
        let mut other = token.clone();
        other.replace_range(0..1, if token.starts_with('a') { "b" } else { "a" });
        assert!(!tokens_equal(&token, &other));
        assert!(!tokens_equal(&token, "abcd"));
        assert!(!tokens_equal(&token, &token.to_uppercase()));
    }

    #[test]
    fn uuid_v4_shape() {
        let id = random_uuid().unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(id.as_bytes()[14], b'4');
        let variant = id.as_bytes()[19];
        assert!(matches!(variant, b'8' | b'9' | b'a' | b'b'));
    }
}
