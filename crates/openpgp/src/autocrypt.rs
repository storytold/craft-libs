//! The `Autocrypt:` header (Autocrypt Level 1, §2.1): a sender's Public key carried in every
//! message, for the address it came from. Read, never acted on here: the caller keeps the key
//! for that address, and nothing is sent.

use crate::keys::{Budget, MAX_CHECKS_PER_CALL, MAX_KEY_BYTES, PublicKey};
use crate::{Error, mime};

/// How long a header value may be: a key for Autocrypt is meant to be small (Level 1 §2.1
/// recommends one signing and one encryption key, a few KB), and the cap leaves room.
pub(crate) const MAX_HEADER: usize = 64 * 1024;

/// An `Autocrypt:` header that reads: whose it is, what they prefer, and their key.
#[derive(Clone, Debug)]
pub struct Autocrypt {
    /// The address it is for (`addr`), lower case.
    pub address: String,
    /// `prefer-encrypt=mutual`: the sender wants encrypted replies when both sides can.
    pub prefer_encrypt_mutual: bool,
    /// The key (`keydata`), its self-signatures checked.
    pub key: PublicKey,
}

/// Reads one `Autocrypt:` header value. [`Error::Unreadable`] when it lacks `addr` or `keydata`,
/// has an attribute it doesn't know that doesn't start with `_` (which Level 1 says makes the
/// whole header one to ignore), or carries no key that checks; [`Error::TooLarge`] when it is
/// longer than a key for Autocrypt needs.
pub fn parse_autocrypt(value: &str) -> Result<Autocrypt, Error> {
    attributes(value)?.read(&mut Budget::new(MAX_CHECKS_PER_CALL))
}

/// A header's attributes, read before its key is: the key is the costly part.
pub(crate) struct Attributes<'a> {
    pub(crate) address: String,
    mutual: bool,
    keydata: &'a str,
}

/// The attributes of an `Autocrypt:` header value, as [`parse_autocrypt`] reads them.
pub(crate) fn attributes(value: &str) -> Result<Attributes<'_>, Error> {
    if value.len() > MAX_HEADER {
        return Err(Error::TooLarge);
    }
    let (mut address, mut keydata, mut mutual) = (None, None, false);
    for attribute in value.split(';') {
        let attribute = attribute.trim();
        if attribute.is_empty() {
            continue;
        }
        let (name, data) = attribute.split_once('=').ok_or(Error::Unreadable)?;
        match name.trim() {
            "addr" if address.is_none() => address = Some(mime::address(data.trim())),
            "keydata" if keydata.is_none() => keydata = Some(data),
            "prefer-encrypt" => mutual = data.trim() == "mutual",
            name if name.starts_with('_') => {}
            _ => return Err(Error::Unreadable),
        }
    }
    let address = address.filter(|a| !a.is_empty()).ok_or(Error::Unreadable)?;
    Ok(Attributes { address, mutual, keydata: keydata.ok_or(Error::Unreadable)? })
}

impl Attributes<'_> {
    /// The header with its key read.
    pub(crate) fn read(self, total: &mut Budget) -> Result<Autocrypt, Error> {
        let bytes = mime::base64(self.keydata.as_bytes());
        if bytes.len() > MAX_KEY_BYTES {
            return Err(Error::TooLarge);
        }
        // Level 1 §2.1: the keydata is one Transferable Public Key.
        let key = PublicKey::binary(&bytes, 1, total).into_iter().next().ok_or(Error::Unreadable)?;
        Ok(Autocrypt { address: self.address, prefer_encrypt_mutual: self.mutual, key })
    }
}
