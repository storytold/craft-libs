//! Lower the declared memory maximum of a core module.
//!
//! A browser has no hook on `memory.grow`. The one cap it enforces is the maximum a module
//! declares for its memory. The browser engines therefore rewrite that maximum to the budget before
//! they compile. A grow past the new maximum returns -1 to the guest, as the Wasm spec says.
//!
//! The rewrite touches the memory section only. No other section holds an offset into it, so the
//! rest of the module stays byte for byte as it was. The gate allows exactly one memory, and no
//! flag but "has maximum" (`0x01`) or "no maximum" (`0x00`) can reach this code.

use std::fmt;
use std::ops::Range;

const MEMORY_SECTION: u8 = 5;
const FLAG_HAS_MAX: u8 = 0x01;
/// Wasm page size in bytes.
pub const WASM_PAGE: u64 = 65_536;

/// The declared limits of the one memory of a module, in 64 KiB pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryLimits {
    pub min_pages: u32,
    pub max_pages: Option<u32>,
}

/// Why a module's memory section could not be read or rewritten.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatchError {
    /// The bytes end inside a header, a section or a number.
    Truncated,
    /// The module declares a count or flag the gate never lets through (shared, 64 bit, many memories).
    Unsupported(&'static str),
    /// A requested maximum is below the declared minimum.
    BelowMinimum { min_pages: u32, max_pages: u32 },
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PatchError::Truncated => f.write_str("the module ends inside its memory section"),
            PatchError::Unsupported(what) => write!(f, "unsupported memory declaration: {what}"),
            PatchError::BelowMinimum { min_pages, max_pages } => write!(f, "maximum {max_pages} pages is below the {min_pages} page minimum"),
        }
    }
}

impl std::error::Error for PatchError {}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> Result<u8, PatchError> {
        let b = *self.bytes.get(self.pos).ok_or(PatchError::Truncated)?;
        self.pos += 1;
        Ok(b)
    }

    /// An unsigned LEB128 number of at most 32 bits.
    fn leb_u32(&mut self) -> Result<u32, PatchError> {
        let mut value = 0u64;
        for i in 0..5 {
            let b = self.u8()?;
            value |= u64::from(b & 0x7F) << (7 * i);
            if b & 0x80 == 0 {
                return u32::try_from(value).map_err(|_| PatchError::Unsupported("number wider than 32 bits"));
            }
        }
        Err(PatchError::Unsupported("number wider than 32 bits"))
    }
}

fn push_leb_u32(out: &mut Vec<u8>, mut n: u32) {
    loop {
        let b = (n & 0x7F) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

/// Where the memory section sits: its whole extent (header included) and its payload.
struct Located {
    whole: Range<usize>,
    payload: Range<usize>,
}

fn locate(wasm: &[u8]) -> Result<Option<Located>, PatchError> {
    let mut r = Reader { bytes: wasm, pos: 8 };
    if wasm.len() < 8 {
        return Err(PatchError::Truncated);
    }
    while r.pos < wasm.len() {
        let start = r.pos;
        let id = r.u8()?;
        let size = r.leb_u32()? as usize;
        let payload_start = r.pos;
        let end = payload_start.checked_add(size).filter(|e| *e <= wasm.len()).ok_or(PatchError::Truncated)?;
        if id == MEMORY_SECTION {
            return Ok(Some(Located { whole: start..end, payload: payload_start..end }));
        }
        r.pos = end;
    }
    Ok(None)
}

fn parse_limits(payload: &[u8]) -> Result<MemoryLimits, PatchError> {
    let mut r = Reader { bytes: payload, pos: 0 };
    if r.leb_u32()? != 1 {
        return Err(PatchError::Unsupported("the module does not declare exactly one memory"));
    }
    let flags = r.u8()?;
    let min_pages = r.leb_u32()?;
    let max_pages = match flags {
        0x00 => None,
        FLAG_HAS_MAX => Some(r.leb_u32()?),
        _ => return Err(PatchError::Unsupported("shared, 64 bit or custom page size memory")),
    };
    Ok(MemoryLimits { min_pages, max_pages })
}

/// The declared limits of the module's memory, or `None` when it declares none (it imports one).
pub fn memory_limits(wasm: &[u8]) -> Result<Option<MemoryLimits>, PatchError> {
    match locate(wasm)? {
        None => Ok(None),
        Some(at) => parse_limits(wasm.get(at.payload).ok_or(PatchError::Truncated)?).map(Some),
    }
}

/// The module with its memory maximum lowered to `max_pages`. A maximum that is already lower stays.
/// Fails with [`PatchError::BelowMinimum`] when `max_pages` is under the declared minimum.
pub fn with_memory_max(wasm: &[u8], max_pages: u32) -> Result<Vec<u8>, PatchError> {
    let at = locate(wasm)?.ok_or(PatchError::Unsupported("the module declares no memory"))?;
    let limits = parse_limits(wasm.get(at.payload.clone()).ok_or(PatchError::Truncated)?)?;
    if max_pages < limits.min_pages {
        return Err(PatchError::BelowMinimum { min_pages: limits.min_pages, max_pages });
    }
    let max = limits.max_pages.map_or(max_pages, |m| m.min(max_pages));
    let mut payload = vec![1, FLAG_HAS_MAX];
    push_leb_u32(&mut payload, limits.min_pages);
    push_leb_u32(&mut payload, max);
    let mut out = Vec::with_capacity(wasm.len() + 8);
    out.extend_from_slice(wasm.get(..at.whole.start).ok_or(PatchError::Truncated)?);
    out.push(MEMORY_SECTION);
    push_leb_u32(&mut out, u32::try_from(payload.len()).map_err(|_| PatchError::Unsupported("section too large"))?);
    out.extend_from_slice(&payload);
    out.extend_from_slice(wasm.get(at.whole.end..).ok_or(PatchError::Truncated)?);
    Ok(out)
}

/// Pages that fit in `bytes`, rounded down. Saturates at `u32::MAX`.
pub fn pages_in(bytes: u64) -> u32 {
    u32::try_from(bytes / WASM_PAGE).unwrap_or(u32::MAX)
}
