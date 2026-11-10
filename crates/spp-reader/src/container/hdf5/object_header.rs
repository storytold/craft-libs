//! Version 1 object headers: the list of raw messages describing one group or dataset.
//!
//! Messages may continue in further blocks (continuation message 0x0010); those are followed here, so
//! callers get one flat list. Decoding message bodies is [`super::message`]'s job.

use super::error::Hdf5Error;
use super::sizes::Address;
use super::storage::Storage;
use crate::container::bytes::ByteReader;
use std::collections::HashSet;

pub(crate) const CONTINUATION: u16 = 0x0010;

/// Prefix of a v1 header: version, reserved, message count, reference count, header size, padding.
const PREFIX_LEN: u64 = 16;

/// More continuation blocks than this means a corrupt (or hostile) file.
const MAX_BLOCKS: usize = 4096;

/// Message flag: the body is a reference to a shared message stored elsewhere.
const FLAG_SHARED: u8 = 0x02;

/// One message as stored: its type, flags and undecoded body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawMessage {
    pub kind: u16,
    pub flags: u8,
    pub body: Vec<u8>,
}

impl RawMessage {
    pub(crate) fn is_shared(&self) -> bool {
        self.flags & FLAG_SHARED != 0
    }
}

/// Reads every message of the object header at `addr`, following continuation blocks.
pub(crate) fn read_messages(storage: &Storage, addr: Address) -> Result<Vec<RawMessage>, Hdf5Error> {
    let prefix = storage.read(addr, PREFIX_LEN, "object header")?;
    let mut r = ByteReader::new(&prefix);
    let version = r.u8()?;
    if version != 1 {
        // Version 2 headers start with "OHDR", whose first byte is 'O'.
        let what = if prefix.starts_with(b"OHDR") { "object header version 2".into() } else { format!("object header version {version}") };
        return Err(Hdf5Error::Unsupported(what));
    }
    r.skip(1 + 2 + 4)?; // reserved, message count (blocks are read to their end instead), reference count
    let first_len = u64::from(r.u32_le()?);

    let mut messages = Vec::new();
    let mut blocks = vec![(addr.saturating_add(PREFIX_LEN), first_len)];
    let mut seen = HashSet::new();
    while let Some((block_addr, block_len)) = blocks.pop() {
        if !seen.insert(block_addr) || seen.len() > MAX_BLOCKS {
            return Err(Hdf5Error::Corrupt(format!("object header at {addr:#x} has looping continuations")));
        }
        let block = storage.read(block_addr, block_len, "object header block")?;
        let mut found = parse_block(&block)?;
        // Continuations found in this block are read after it, in order (pushed in reverse).
        for message in found.iter().rev().filter(|m| m.kind == CONTINUATION) {
            blocks.push(parse_continuation(storage, &message.body)?);
        }
        found.retain(|m| m.kind != CONTINUATION);
        messages.append(&mut found);
    }
    Ok(messages)
}

/// Splits one header block into messages: type (2), size (2), flags (1), reserved (3), body.
fn parse_block(block: &[u8]) -> Result<Vec<RawMessage>, Hdf5Error> {
    let mut r = ByteReader::new(block);
    let mut out = Vec::new();
    while r.remaining() >= 8 {
        let kind = r.u16_le()?;
        let size = usize::from(r.u16_le()?);
        let flags = r.u8()?;
        r.skip(3)?;
        let body = r.bytes(size)?.to_vec();
        if kind != 0 {
            out.push(RawMessage { kind, flags, body });
        }
    }
    Ok(out)
}

/// Body of a continuation message: address and length of the next block.
fn parse_continuation(storage: &Storage, body: &[u8]) -> Result<(Address, u64), Hdf5Error> {
    let mut r = ByteReader::new(body);
    let sizes = storage.sizes();
    let addr = sizes.required_address(&mut r, "object header continuation")?;
    Ok((addr, sizes.length(&mut r)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::hdf5::sizes::Sizes;
    use crate::container::hdf5::source::MemorySource;

    fn message(kind: u16, body: &[u8]) -> Vec<u8> {
        let mut out = kind.to_le_bytes().to_vec();
        out.extend((body.len() as u16).to_le_bytes());
        out.extend([0; 4]);
        out.extend(body);
        out
    }

    fn header(count: u16, messages: &[u8]) -> Vec<u8> {
        let mut out = vec![1, 0];
        out.extend(count.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        out.extend((messages.len() as u32).to_le_bytes());
        out.extend([0; 4]);
        out.extend(messages);
        out
    }

    fn storage(file: Vec<u8>) -> Storage {
        Storage::new(Box::new(MemorySource(file)), 0, Sizes::new(8, 8).unwrap())
    }

    #[test]
    fn follows_continuation_blocks() {
        let mut cont = 0x100u64.to_le_bytes().to_vec();
        cont.extend(16u64.to_le_bytes());
        let mut file = header(3, &[message(0x11, &[1; 8]), message(CONTINUATION, &cont)].concat());
        file.resize(0x100, 0);
        file.extend(message(0x0c, &[2; 8]));
        let messages = read_messages(&storage(file), 0).unwrap();
        let kinds: Vec<u16> = messages.iter().map(|m| m.kind).collect();
        assert_eq!(kinds, vec![0x11, 0x0c]);
        assert_eq!(messages[1].body, vec![2; 8]);
    }

    #[test]
    fn a_self_referencing_continuation_is_corrupt() {
        let mut cont = 16u64.to_le_bytes().to_vec();
        cont.extend(24u64.to_le_bytes());
        let file = header(9, &message(CONTINUATION, &cont));
        assert!(matches!(read_messages(&storage(file), 0), Err(Hdf5Error::Corrupt(_))));
    }

    #[test]
    fn version_2_headers_are_unsupported() {
        let file = b"OHDR\x02\0\0\0\0\0\0\0\0\0\0\0".to_vec();
        assert!(matches!(read_messages(&storage(file), 0), Err(Hdf5Error::Unsupported(_))));
    }

    #[test]
    fn an_overlong_message_is_corrupt() {
        let mut block = message(0x11, &[0; 8]);
        block[2] = 200;
        assert!(matches!(read_messages(&storage(header(1, &block)), 0), Err(Hdf5Error::Corrupt(_))));
    }
}
