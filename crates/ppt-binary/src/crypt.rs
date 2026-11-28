//! Password-protected presentations ([MS-PPT] §2.3.7, [MS-OFFCRYPTO] §2.3.5).
//!
//! PowerPoint 2002 and later encrypt a `.ppt` with RC4 CryptoAPI. The `Current User` stream, the
//! `UserEditAtom`, the `PersistDirectoryAtom` and the `CryptSession10Container` (which holds the
//! encryption header) are stored as they are; every other persist object is encrypted on its own,
//! from its record header to its end, with the key of block number = its persist id. In the
//! `Pictures` stream each field of each record is encrypted on its own with the key of block 0:
//! the record header, each UID, a metafile's 34-byte header or a bitmap's tag byte, then the
//! picture data. (Saving a test deck with a password in PowerPoint 2016 and comparing it with the
//! same deck saved without one showed those fields.)
//!
//! [`decrypt`] checks the password against the header's verifier before anything is decrypted, so
//! a wrong password is reported and never yields a garbled presentation, and returns decrypted
//! copies of the two streams for the usual reader.

use std::collections::HashMap;

use craft_offcrypto::rc4::{Key, Rc4};

use crate::PptError;
use crate::doc::{self, rt};
use crate::record::{Rec, u16_at, u32_at};

/// The first and last record types of a BLIP ([MS-ODRAW] §2.2.23).
const BLIP_FIRST: u16 = 0xF018;
const BLIP_LAST: u16 = 0xF117;
/// The BLIP types whose data starts with a 34-byte metafile header: EMF, WMF and PICT.
const METAFILE_BLIPS: [u16; 3] = [0xF01A, 0xF01B, 0xF01C];
/// Bytes of a metafile BLIP's `OfficeArtMetafileHeader`.
const METAFILE_HEADER: usize = 34;
/// Bytes of an `OfficeArtFBSE` before its name ([MS-ODRAW] §2.2.32), field by field.
const FBSE_FIELDS: [usize; 11] = [1, 1, 16, 2, 4, 4, 4, 1, 1, 1, 1];
/// Offset of `cbName` in an `OfficeArtFBSE`.
const FBSE_NAME_LEN: usize = 33;

/// Decrypted copies of the `PowerPoint Document` and `Pictures` streams.
pub(crate) struct Decrypted {
    pub stream: Vec<u8>,
    pub pictures: Vec<u8>,
}

/// The decrypted streams, or `None` when the presentation isn't encrypted. `password` is only
/// needed, and only looked at, when it is.
pub(crate) fn decrypt(current_user: &[u8], stream: &[u8], pictures: &[u8], password: Option<&str>) -> Result<Option<Decrypted>, PptError> {
    let session = match session(current_user, stream) {
        Found::Plain => return Ok(None),
        // Without a password that is all there is to say; with one, the header is missing.
        Found::Marked if password.is_none() => return Err(PptError::Encrypted),
        Found::Marked => return Err(PptError::Damaged("the presentation is marked encrypted but has no encryption header".into())),
        Found::Session(s) => s,
    };
    let password = password.ok_or(PptError::Encrypted)?;
    let info = Rec::at(stream, session.at).map(|r| r.data).unwrap_or_default();
    // A presentation uses RC4 CryptoAPI only ([MS-PPT] §2.3.7). Plain RC4 (version 1.1) is
    // refused from its header, before the password is checked, so it isn't a wrong password.
    if info.get(..4) == Some(&[1, 0, 1, 0]) {
        return Err(PptError::UnsupportedEncryption("RC4 without CryptoAPI (version 1.1)".into()));
    }
    let (key, _) = Key::open(info, password).map_err(|e| match e {
        craft_offcrypto::Error::WrongPassword => PptError::WrongPassword,
        craft_offcrypto::Error::Unsupported(what) => PptError::UnsupportedEncryption(what),
        other => PptError::Damaged(other.to_string()),
    })?;
    Ok(Some(Decrypted { stream: decrypt_document(stream, &session, &key), pictures: decrypt_pictures(pictures, &key) }))
}

/// Where an encrypted presentation keeps what is stored as it is.
struct Session {
    /// The `CryptSession10Container`.
    at: usize,
    /// Persist ids by offset, from the edits' persist directories.
    persist: HashMap<u32, usize>,
    /// The edit and persist directory records, which are not encrypted either.
    plain: Vec<usize>,
}

/// What [`session`] found.
enum Found {
    /// Not encrypted, as far as decrypting goes. Other signs of encryption are the reader's.
    Plain,
    /// The `Current User` says encrypted, but no edit names a `CryptSession10Container`.
    Marked,
    Session(Session),
}

/// The encryption session of an encrypted presentation, whose newest edit names a
/// `CryptSession10Container` (PowerPoint 2002 marked such files with the usual `Current User`
/// token, later versions with the encrypted one).
fn session(current_user: &[u8], stream: &[u8]) -> Found {
    let cu = Rec::at(current_user, 0).filter(|r| r.kind == rt::CURRENT_USER);
    let token = cu.and_then(|r| r.u32(4));
    // An encrypted stream can't be scanned for its edit: the records before it are encrypted.
    let marked = if token == Some(doc::ENCRYPTED_TOKEN) { Found::Marked } else { Found::Plain };
    let Some(edit_at) = cu.and_then(|r| r.u32(8)).map(|v| v as usize) else { return marked };
    let Some(edit) = Rec::at(stream, edit_at).filter(|r| r.kind == rt::USER_EDIT) else { return marked };
    let (persist, plain) = doc::persist_directories(stream, edit_at);
    let crypt_ref = edit.u32(28).filter(|_| edit.data.len() >= 32);
    let at = crypt_ref.and_then(|id| persist.get(&id).copied()).filter(|&o| Rec::at(stream, o).is_some_and(|r| r.kind == rt::CRYPT_SESSION));
    match at {
        Some(at) => Found::Session(Session { at, persist, plain }),
        None => marked,
    }
}

/// The document stream with every persist object decrypted. Each object is decrypted from its
/// header to its end, but no further than the next object or plain record: real objects don't
/// overlap, and clipping keeps hostile ones from making the work more than the stream's length.
fn decrypt_document(stream: &[u8], session: &Session, key: &Key) -> Vec<u8> {
    let mut out = stream.to_vec();
    // Each offset once, under its lowest id.
    let mut objects: Vec<(usize, u32)> = session.persist.iter().map(|(&id, &at)| (at, id)).collect();
    objects.sort_unstable();
    objects.dedup_by_key(|o| o.0);
    let mut bounds: Vec<usize> = objects.iter().map(|o| o.0).chain(session.plain.iter().copied()).chain([session.at, stream.len()]).collect();
    bounds.sort_unstable();
    bounds.dedup();
    let mut plain = session.plain.clone();
    plain.sort_unstable();
    for &(at, id) in &objects {
        if at == session.at || plain.binary_search(&at).is_ok() || at >= stream.len() {
            continue;
        }
        let next = bounds.get(bounds.partition_point(|&b| b <= at)).copied().unwrap_or(stream.len());
        let Some(object) = out.get_mut(at..next) else { continue };
        // The header says how long the object is; the keystream runs on into its body.
        let mut rc4 = key.cipher(id);
        let (header, body) = object.split_at_mut(object.len().min(8));
        rc4.apply(header);
        let len = u32_at(header, 4).map_or(0, |l| l as usize);
        let end = len.min(body.len());
        if let Some(b) = body.get_mut(..end) {
            rc4.apply(b);
        }
    }
    out
}

/// The `Pictures` stream with each record decrypted field by field. It ends at the first record
/// that is all zeros (the stream's padding, which is stored as it is) or doesn't fit.
fn decrypt_pictures(pictures: &[u8], key: &Key) -> Vec<u8> {
    let mut out = pictures.to_vec();
    let k0 = key.block_key(0);
    let mut at = 0;
    while at + 8 <= out.len() {
        if out.get(at..at + 8).is_some_and(|h| h.iter().all(|&b| b == 0)) {
            break;
        }
        match decrypt_record(&mut out, at, &k0, true) {
            Some(end) if end > at => at = end,
            _ => break,
        }
    }
    out
}

/// Decrypt `len` bytes from `from` (fewer at `end`) as one field, with a fresh block-0 cipher;
/// the offset after them.
fn field(out: &mut [u8], from: usize, len: usize, end: usize, k0: &[u8]) -> usize {
    let to = from.saturating_add(len).min(end).min(out.len()).max(from);
    if let Some(b) = out.get_mut(from..to) {
        Rc4::new(k0).apply(b);
    }
    to
}

/// Decrypt one Pictures-stream record at `at` field by field; the offset after it. `nested` is
/// whether an FBSE may hold its BLIP, one level down.
fn decrypt_record(out: &mut [u8], at: usize, k0: &[u8], nested: bool) -> Option<usize> {
    let body = field(out, at, 8, out.len(), k0);
    if body < at + 8 {
        return None;
    }
    let vi = u16_at(out, at)?;
    let kind = u16_at(out, at + 2)?;
    let end = body.saturating_add(u32_at(out, at + 4)? as usize).min(out.len());
    let mut pos = body;
    match kind {
        BLIP_FIRST..=BLIP_LAST => {
            // A second UID when the instance is the odd one of its pair.
            let uids = if (vi >> 4) & 1 == 1 { 2 } else { 1 };
            for _ in 0..uids {
                pos = field(out, pos, 16, end, k0);
            }
            pos = field(out, pos, if METAFILE_BLIPS.contains(&kind) { METAFILE_HEADER } else { 1 }, end, k0);
        }
        rt::FBSE => {
            for len in FBSE_FIELDS {
                pos = field(out, pos, len, end, k0);
            }
            let name = out.get(body + FBSE_NAME_LEN).copied().map_or(0, usize::from);
            pos = field(out, pos, name, end, k0);
            if nested
                && pos + 8 <= end
                && let Some(within) = out.get_mut(..end)
            {
                pos = decrypt_record(within, pos, k0, false).unwrap_or(end);
            }
        }
        _ => {}
    }
    // The rest of the record is one field.
    field(out, pos, end.saturating_sub(pos), end, k0);
    Some(end)
}
