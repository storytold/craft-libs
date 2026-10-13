//! The administrator's password, stored as an iterated, salted SHA-512 hash and never in clear.
//!
//! The construction is the one ECMA-376 (Office Open XML) uses for password protection, which
//! GridCraft already verifies for workbooks: `H₀ = SHA-512(salt ‖ password)`, then
//! `Hᵢ₊₁ = SHA-512(i as u32 LE ‖ Hᵢ)` for each iteration. It needs only `sha2`, already in every
//! Craft app's dependency tree. The default 210,000 iterations is OWASP's current figure for
//! PBKDF2-HMAC-SHA512. It is not memory-hard (as Argon2 would be); the hash lives in a file only
//! the service account can read, so getting it already means having that account.
//!
//! Stored form: `craft-sha512$<iterations>$<salt hex>$<hash hex>`.

use crate::Error;
use sha2::Digest;
use std::fmt;

/// Iterations for a new password.
pub const DEFAULT_ITERATIONS: u32 = 210_000;
/// Fewest iterations accepted from a configuration file.
pub const MIN_ITERATIONS: u32 = 1_000;
/// Most iterations accepted: more would only make every sign-in hang.
pub const MAX_ITERATIONS: u32 = 10_000_000;
/// Salt length for a new password, and the shortest accepted.
pub const SALT_LEN: usize = 16;
const MAX_SALT_LEN: usize = 64;
/// Longest password hashed; longer input is refused rather than hashed.
pub const MAX_PASSWORD_BYTES: usize = 1024;
const PREFIX: &str = "craft-sha512";

#[derive(Clone, PartialEq, Eq)]
pub struct PasswordHash {
    iterations: u32,
    salt: Vec<u8>,
    hash: [u8; 64],
}

impl PasswordHash {
    /// Hashes a new password with [`DEFAULT_ITERATIONS`]. `salt` must be at least [`SALT_LEN`]
    /// random bytes (the server takes them from the operating system).
    pub fn new(password: &str, salt: &[u8]) -> Result<PasswordHash, Error> {
        Self::with_iterations(password, salt, DEFAULT_ITERATIONS)
    }

    pub fn with_iterations(password: &str, salt: &[u8], iterations: u32) -> Result<PasswordHash, Error> {
        if password.is_empty() {
            return Err(Error::Config("the administrator password can't be empty".into()));
        }
        if password.len() > MAX_PASSWORD_BYTES {
            return Err(Error::Config(format!("the administrator password is longer than {MAX_PASSWORD_BYTES} bytes")));
        }
        check(iterations, salt)?;
        Ok(PasswordHash { iterations, salt: salt.to_vec(), hash: derive(password, salt, iterations) })
    }

    /// Reads the stored form.
    pub fn parse(text: &str) -> Result<PasswordHash, Error> {
        let damaged = || Error::Config("the stored administrator password hash is damaged".into());
        let mut parts = text.trim().split('$');
        if parts.next() != Some(PREFIX) {
            return Err(Error::Config(format!("the administrator password hash isn't a {PREFIX} hash")));
        }
        let iterations: u32 = parts.next().and_then(|n| n.parse().ok()).ok_or_else(damaged)?;
        let salt = parts.next().and_then(from_hex).ok_or_else(damaged)?;
        let hash: [u8; 64] = parts.next().and_then(from_hex).and_then(|h| h.try_into().ok()).ok_or_else(damaged)?;
        if parts.next().is_some() {
            return Err(damaged());
        }
        check(iterations, &salt)?;
        Ok(PasswordHash { iterations, salt, hash })
    }

    /// Whether `password` is the one hashed. Compares in constant time.
    pub fn verify(&self, password: &str) -> bool {
        if password.len() > MAX_PASSWORD_BYTES {
            return false;
        }
        constant_time_eq(&derive(password, &self.salt, self.iterations), &self.hash)
    }

    pub fn iterations(&self) -> u32 {
        self.iterations
    }
}

fn check(iterations: u32, salt: &[u8]) -> Result<(), Error> {
    if !(MIN_ITERATIONS..=MAX_ITERATIONS).contains(&iterations) {
        return Err(Error::Config(format!("the password hash asks for {iterations} iterations; {MIN_ITERATIONS} to {MAX_ITERATIONS} are allowed")));
    }
    if !(SALT_LEN..=MAX_SALT_LEN).contains(&salt.len()) {
        return Err(Error::Config(format!("a password salt must be {SALT_LEN} to {MAX_SALT_LEN} bytes")));
    }
    Ok(())
}

fn derive(password: &str, salt: &[u8], iterations: u32) -> [u8; 64] {
    let mut hasher = sha2::Sha512::new();
    hasher.update(salt);
    hasher.update(password.as_bytes());
    let mut h: [u8; 64] = hasher.finalize().into();
    for i in 0..iterations {
        let mut hasher = sha2::Sha512::new();
        hasher.update(i.to_le_bytes());
        hasher.update(h);
        h = hasher.finalize().into();
    }
    h
}

/// Byte-wise equality whose running time doesn't depend on where the inputs differ.
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    out
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || text.len() > 2 * 128 {
        return None;
    }
    let digit = |c: u8| (c as char).to_digit(16).and_then(|d| u8::try_from(d).ok());
    text.as_bytes().chunks(2).map(|pair| Some(digit(*pair.first()?)? << 4 | digit(*pair.get(1)?)?)).collect()
}

impl fmt::Display for PasswordHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{PREFIX}${}${}${}", self.iterations, to_hex(&self.salt), to_hex(&self.hash))
    }
}

/// Never prints the hash or the salt.
impl fmt::Debug for PasswordHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PasswordHash({PREFIX}, {} iterations)", self.iterations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SALT: &[u8; 16] = b"0123456789abcdef";

    #[test]
    fn the_right_password_verifies_and_others_do_not() {
        let h = PasswordHash::with_iterations("correct horse", SALT, MIN_ITERATIONS).unwrap();
        assert!(h.verify("correct horse"));
        for wrong in ["", "correct hors", "correct horse ", "Correct horse", "correct horsf", &"x".repeat(MAX_PASSWORD_BYTES + 1)] {
            assert!(!h.verify(wrong), "{wrong:?}");
        }
    }

    #[test]
    fn the_stored_form_round_trips_and_is_salted() {
        let h = PasswordHash::with_iterations("pässwörd", SALT, 2_000).unwrap();
        let text = h.to_string();
        assert!(text.starts_with("craft-sha512$2000$30313233343536373839616263646566$"), "{text}");
        assert!(!text.contains("pässwörd"));
        let back = PasswordHash::parse(&text).unwrap();
        assert_eq!(back, h);
        assert!(back.verify("pässwörd"));
        let other_salt = PasswordHash::with_iterations("pässwörd", b"fedcba9876543210", 2_000).unwrap();
        assert_ne!(other_salt.to_string(), text);
        assert!(!format!("{h:?}").contains("3031"));
    }

    #[test]
    fn the_construction_matches_its_definition() {
        // H0 = SHA-512(salt || password); H(i+1) = SHA-512(LE32(i) || Hi): one iteration by hand.
        let h0: [u8; 64] = sha2::Sha512::digest([SALT.as_slice(), b"pw"].concat()).into();
        let h1: [u8; 64] = sha2::Sha512::digest([0u32.to_le_bytes().as_slice(), &h0].concat()).into();
        assert_eq!(derive("pw", SALT, 1), h1);
    }

    #[test]
    fn damaged_or_dangerous_stored_hashes_are_refused() {
        let good = PasswordHash::with_iterations("pw", SALT, MIN_ITERATIONS).unwrap().to_string();
        let hash_hex = good.rsplit('$').next().unwrap().to_string();
        for bad in [
            String::new(),
            "pw".into(),
            good.replacen("craft-sha512", "sha512", 1),
            format!("craft-sha512$999${}${hash_hex}", to_hex(SALT)),
            format!("craft-sha512$4294967295${}${hash_hex}", to_hex(SALT)),
            format!("craft-sha512$1000$0011${hash_hex}"),
            format!("craft-sha512$1000${}${}", to_hex(SALT), &hash_hex[2..]),
            format!("craft-sha512$1000${}${hash_hex}$extra", to_hex(SALT)),
            format!("craft-sha512$1000${}$zz{}", to_hex(SALT), &hash_hex[2..]),
            format!("craft-sha512$-5${}${hash_hex}", to_hex(SALT)),
        ] {
            assert!(PasswordHash::parse(&bad).is_err(), "{bad:?}");
        }
        assert!(PasswordHash::new("", SALT).is_err());
        assert!(PasswordHash::new("pw", b"short").is_err());
    }

    #[test]
    fn constant_time_eq_is_plain_equality() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }
}
