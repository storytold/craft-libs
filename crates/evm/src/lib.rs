//! EVM bytecode: the public function selectors a contract's dispatcher compares against, where
//! each one jumps, whether it refuses payments, fallback/receive presence, and the compiler
//! metadata (CBOR) appended to the code. Nothing is executed; this is a linear sweep that
//! respects PUSH data plus pattern matching over the instruction stream.

use crate::FormatError;
use serde::Serialize;
use serde_json::{Map, Value};

/// Largest bytecode we analyse (16 MiB; deployed contracts are at most 24 KiB).
pub const MAX_CODE: usize = 16 << 20;
const MAX_FUNCTIONS: usize = 4096;
const LISTING_CAP: usize = 400;

/// How input bytes are encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// Hex when the bytes are all hex digits (with an optional 0x), raw otherwise.
    Auto,
    Raw,
    Hex,
}

/// Turn input bytes into bytecode. Hex is strict: optional `0x`, even length, only hex digits;
/// surrounding whitespace is ignored.
pub fn decode_input(input: &[u8], enc: Encoding) -> Result<(Vec<u8>, &'static str), FormatError> {
    let trimmed = input.trim_ascii();
    let body = trimmed.strip_prefix(b"0x").or_else(|| trimmed.strip_prefix(b"0X")).unwrap_or(trimmed);
    let looks_hex = !body.is_empty() && body.iter().all(u8::is_ascii_hexdigit);
    let hex = match enc {
        Encoding::Hex => true,
        Encoding::Raw => false,
        Encoding::Auto => looks_hex,
    };
    if !hex {
        if input.len() > MAX_CODE {
            return Err(FormatError::TooLarge(format!("{} bytes; the limit is {MAX_CODE}", input.len())));
        }
        return Ok((input.to_vec(), "raw"));
    }
    if body.is_empty() {
        return Err(FormatError::Malformed("no hex digits".into()));
    }
    if let Some(p) = body.iter().position(|b| !b.is_ascii_hexdigit()) {
        return Err(FormatError::Malformed(format!("not a hex digit at position {p}")));
    }
    if !body.len().is_multiple_of(2) {
        return Err(FormatError::Malformed(format!("odd number of hex digits ({})", body.len())));
    }
    if body.len() / 2 > MAX_CODE {
        return Err(FormatError::TooLarge(format!("{} bytes; the limit is {MAX_CODE}", body.len() / 2)));
    }
    let nib = |c: u8| -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => c.wrapping_sub(b'A').wrapping_add(10) & 0x0f,
        }
    };
    Ok((body.chunks(2).map(|p| (nib(p.first().copied().unwrap_or(b'0')) << 4) | nib(p.get(1).copied().unwrap_or(b'0'))).collect(), "hex"))
}

#[derive(Debug, Clone, Copy)]
struct Ins {
    off: usize,
    op: u8,
    /// Immediate (PUSH data) start and length.
    imm: (usize, usize),
}

fn push_len(op: u8) -> usize {
    if (0x60..=0x7f).contains(&op) { usize::from(op - 0x5f) } else { 0 }
}

/// The mnemonic of an opcode (`UNKNOWN_0x…` for unassigned ones).
pub fn mnemonic(op: u8) -> String {
    let s = match op {
        0x00 => "STOP",
        0x01 => "ADD",
        0x02 => "MUL",
        0x03 => "SUB",
        0x04 => "DIV",
        0x05 => "SDIV",
        0x06 => "MOD",
        0x07 => "SMOD",
        0x08 => "ADDMOD",
        0x09 => "MULMOD",
        0x0a => "EXP",
        0x0b => "SIGNEXTEND",
        0x10 => "LT",
        0x11 => "GT",
        0x12 => "SLT",
        0x13 => "SGT",
        0x14 => "EQ",
        0x15 => "ISZERO",
        0x16 => "AND",
        0x17 => "OR",
        0x18 => "XOR",
        0x19 => "NOT",
        0x1a => "BYTE",
        0x1b => "SHL",
        0x1c => "SHR",
        0x1d => "SAR",
        0x20 => "KECCAK256",
        0x30 => "ADDRESS",
        0x31 => "BALANCE",
        0x32 => "ORIGIN",
        0x33 => "CALLER",
        0x34 => "CALLVALUE",
        0x35 => "CALLDATALOAD",
        0x36 => "CALLDATASIZE",
        0x37 => "CALLDATACOPY",
        0x38 => "CODESIZE",
        0x39 => "CODECOPY",
        0x3a => "GASPRICE",
        0x3b => "EXTCODESIZE",
        0x3c => "EXTCODECOPY",
        0x3d => "RETURNDATASIZE",
        0x3e => "RETURNDATACOPY",
        0x3f => "EXTCODEHASH",
        0x40 => "BLOCKHASH",
        0x41 => "COINBASE",
        0x42 => "TIMESTAMP",
        0x43 => "NUMBER",
        0x44 => "PREVRANDAO",
        0x45 => "GASLIMIT",
        0x46 => "CHAINID",
        0x47 => "SELFBALANCE",
        0x48 => "BASEFEE",
        0x49 => "BLOBHASH",
        0x4a => "BLOBBASEFEE",
        0x50 => "POP",
        0x51 => "MLOAD",
        0x52 => "MSTORE",
        0x53 => "MSTORE8",
        0x54 => "SLOAD",
        0x55 => "SSTORE",
        0x56 => "JUMP",
        0x57 => "JUMPI",
        0x58 => "PC",
        0x59 => "MSIZE",
        0x5a => "GAS",
        0x5b => "JUMPDEST",
        0x5c => "TLOAD",
        0x5d => "TSTORE",
        0x5e => "MCOPY",
        0x5f => "PUSH0",
        0x60..=0x7f => return format!("PUSH{}", op - 0x5f),
        0x80..=0x8f => return format!("DUP{}", op - 0x7f),
        0x90..=0x9f => return format!("SWAP{}", op - 0x8f),
        0xa0..=0xa4 => return format!("LOG{}", op - 0xa0),
        0xf0 => "CREATE",
        0xf1 => "CALL",
        0xf2 => "CALLCODE",
        0xf3 => "RETURN",
        0xf4 => "DELEGATECALL",
        0xf5 => "CREATE2",
        0xfa => "STATICCALL",
        0xfd => "REVERT",
        0xfe => "INVALID",
        0xff => "SELFDESTRUCT",
        _ => return format!("UNKNOWN_{op:#04x}"),
    };
    s.to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvmFunction {
    /// `0x` + 8 hex digits.
    pub selector: String,
    /// Where the dispatcher sends calls with this selector.
    pub bytecode_offset: usize,
    /// Offset of the comparison in the dispatcher.
    pub compare_offset: usize,
    pub dispatch: &'static str,
    /// `eq-jumpi` (jump on match) or `eq-iszero-fallthrough` (body follows the check).
    pub pattern: &'static str,
    /// `nonpayable`, `payable` or `unknown` (view/pure cannot be told from the dispatcher).
    pub mutability: &'static str,
    /// Why the mutability was chosen.
    pub mutability_reason: String,
    /// False when the destination is not a JUMPDEST (the match is then doubtful).
    pub jumpdest_valid: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EvmMetadata {
    pub offset: usize,
    pub length: usize,
    pub entries: Value,
    pub compiler: Option<String>,
    pub ipfs: Option<String>,
    pub swarm: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EvmInterface {
    pub code_size: usize,
    /// Bytes analysed after removing the metadata tail.
    pub analysed_size: usize,
    pub metadata: Option<EvmMetadata>,
    pub instructions: usize,
    pub jumpdests: usize,
    /// How the selector is pulled from calldata: `shr-224`, `div-2^224`, `and-mask`, or none.
    pub selector_extraction: Option<&'static str>,
    pub dispatcher_offset: Option<usize>,
    /// `solc-like`, `vyper-like`, `mixed` or `none`.
    pub style: &'static str,
    pub functions: Vec<EvmFunction>,
    /// Comparisons that split the selector range (binary-search dispatchers).
    pub binary_search_splits: usize,
    /// A CALLVALUE check that runs before any selector comparison (all functions non-payable).
    pub callvalue_check_before_dispatch: bool,
    pub short_calldata_check: bool,
    /// `present`, `absent` or `unknown`.
    pub fallback: &'static str,
    /// `present` or `not-detected`.
    pub receive: &'static str,
    /// The dispatcher region as text (first instructions only).
    pub dispatcher_listing: Vec<String>,
    pub warnings: Vec<String>,
}

fn sweep(code: &[u8]) -> Vec<Ins> {
    let mut out = Vec::with_capacity(code.len().min(1 << 20) / 2);
    let mut i = 0usize;
    while i < code.len() {
        let op = code.get(i).copied().unwrap_or(0);
        let n = push_len(op);
        out.push(Ins { off: i, op, imm: (i + 1, n) });
        i = i.saturating_add(1).saturating_add(n);
    }
    out
}

fn imm<'a>(code: &'a [u8], ins: &Ins) -> &'a [u8] {
    // Truncated PUSH data at the end of code reads as zero-padded; we only use what exists.
    let (s, n) = ins.imm;
    code.get(s..s.saturating_add(n).min(code.len())).unwrap_or(&[])
}

fn imm_u64(code: &[u8], ins: &Ins) -> Option<u64> {
    let b = imm(code, ins);
    if ins.imm.1 > 8 || b.len() != ins.imm.1 {
        return None;
    }
    Some(b.iter().fold(0u64, |a, &x| (a << 8) | u64::from(x)))
}

fn is_push(op: u8) -> bool {
    (0x60..=0x7f).contains(&op)
}

/// Strip and decode a trailing CBOR metadata block.
fn split_metadata(code: &[u8]) -> (usize, Option<EvmMetadata>) {
    let n = code.len();
    if n < 4 {
        return (n, None);
    }
    let len = usize::from(u16::from_be_bytes([code.get(n - 2).copied().unwrap_or(0), code.get(n - 1).copied().unwrap_or(0)]));
    if len == 0 || len.saturating_add(2) > n {
        return (n, None);
    }
    let start = n - 2 - len;
    let Some(cbor) = code.get(start..n - 2) else { return (n, None) };
    if !matches!(cbor.first(), Some(0xa1..=0xb7 | 0xbf)) {
        return (n, None);
    }
    let mut pos = 0usize;
    let Some(v) = cbor_item(cbor, &mut pos, 0) else { return (n, None) };
    if pos != cbor.len() {
        return (n, None);
    }
    let Value::Object(map) = &v else { return (n, None) };
    let compiler = map
        .get("solc")
        .and_then(|s| s.as_str().map(String::from))
        .map(|v| format!("solc {v}"))
        .or_else(|| map.get("vyper").and_then(Value::as_str).map(|v| format!("vyper {v}")));
    let ipfs = map.get("ipfs").and_then(Value::as_str).map(String::from);
    let swarm = map.get("bzzr1").or_else(|| map.get("bzzr0")).and_then(Value::as_str).map(String::from);
    (start, Some(EvmMetadata { offset: start, length: len + 2, entries: v.clone(), compiler, ipfs, swarm }))
}

fn cbor_arg(b: &[u8], pos: &mut usize, info: u8) -> Option<u64> {
    let take = |pos: &mut usize, k: usize| -> Option<u64> {
        let s = b.get(*pos..pos.checked_add(k)?)?;
        *pos += k;
        Some(s.iter().fold(0u64, |a, &x| (a << 8) | u64::from(x)))
    };
    match info {
        0..=23 => Some(u64::from(info)),
        24 => take(pos, 1),
        25 => take(pos, 2),
        26 => take(pos, 4),
        27 => take(pos, 8),
        _ => None,
    }
}

fn cbor_item(b: &[u8], pos: &mut usize, depth: u32) -> Option<Value> {
    if depth > 4 {
        return None;
    }
    let ib = *b.get(*pos)?;
    *pos += 1;
    let major = ib >> 5;
    let info = ib & 0x1f;
    match major {
        0 => cbor_arg(b, pos, info).map(Value::from),
        2 | 3 => {
            let n = usize::try_from(cbor_arg(b, pos, info)?).ok()?;
            let s = b.get(*pos..pos.checked_add(n)?)?;
            *pos += n;
            Some(if major == 3 { Value::from(String::from_utf8_lossy(s).into_owned()) } else { Value::from(bytes_value(s)) })
        }
        4 => {
            let n = usize::try_from(cbor_arg(b, pos, info)?).ok()?.min(64);
            let mut items = Vec::new();
            for _ in 0..n {
                items.push(cbor_item(b, pos, depth + 1)?);
            }
            // A version array like [0, 3, 10] reads as "0.3.10".
            if items.len() == 3 && items.iter().all(Value::is_u64) {
                return Some(Value::from(items.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(".")));
            }
            Some(Value::Array(items))
        }
        5 => {
            let n = usize::try_from(cbor_arg(b, pos, info)?).ok()?.min(32);
            let mut m = Map::new();
            for _ in 0..n {
                let k = match cbor_item(b, pos, depth + 1)? {
                    Value::String(s) => s,
                    other => other.to_string(),
                };
                let v = cbor_item(b, pos, depth + 1)?;
                let v = match (k.as_str(), &v) {
                    ("solc", Value::String(s)) if s.starts_with("0x") && s.len() == 8 => {
                        let d: Vec<u8> = (0..3).filter_map(|i| u8::from_str_radix(s.get(2 + 2 * i..4 + 2 * i)?, 16).ok()).collect();
                        if let [a, bb, c] = d.as_slice() { Value::from(format!("{a}.{bb}.{c}")) } else { v }
                    }
                    ("ipfs", Value::String(s)) if s.starts_with("0x1220") => Value::from(base58(&hex_decode(s.get(2..).unwrap_or("")))),
                    _ => v,
                };
                m.insert(k, v);
            }
            Some(Value::Object(m))
        }
        7 => match info {
            20 => Some(Value::Bool(false)),
            21 => Some(Value::Bool(true)),
            22 => Some(Value::Null),
            _ => None,
        },
        _ => None,
    }
}

fn bytes_value(s: &[u8]) -> String {
    let mut out = String::with_capacity(2 + s.len() * 2);
    out.push_str("0x");
    for b in s {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len() / 2).filter_map(|i| u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()).collect()
}

fn base58(b: &[u8]) -> String {
    const A: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let zeros = b.iter().take_while(|&&x| x == 0).count();
    let mut digits: Vec<u8> = Vec::new();
    for &byte in b.iter().take(128) {
        let mut carry = u32::from(byte);
        for d in digits.iter_mut() {
            carry += u32::from(*d) << 8;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let mut s: String = std::iter::repeat_n('1', zeros).collect();
    s.extend(digits.iter().rev().map(|&d| char::from(*A.get(usize::from(d)).unwrap_or(&b'1'))));
    s
}

/// Analyse bytecode (already decoded from hex if needed).
pub fn analyze(code: &[u8]) -> Result<EvmInterface, FormatError> {
    if code.is_empty() {
        return Err(FormatError::Truncated("no bytecode".into()));
    }
    if code.len() > MAX_CODE {
        return Err(FormatError::TooLarge(format!("{} bytes; the limit is {MAX_CODE}", code.len())));
    }
    if code.starts_with(&[0xef, 0x00]) {
        return Err(FormatError::Unsupported("EVM Object Format (EOF, 0xEF00) containers are not analysed".into()));
    }
    let mut warnings = Vec::new();
    let (end, metadata) = split_metadata(code);
    let body = code.get(..end).unwrap_or(code);
    let ins = sweep(body);
    let jumpdests = ins.iter().filter(|i| i.op == 0x5b).count();
    let at = |k: usize| ins.get(k);
    let op_at = |k: usize| at(k).map(|i| i.op);
    let index_of = |off: usize| ins.binary_search_by_key(&off, |i| i.off).ok();

    // Selector extraction: CALLDATALOAD followed by SHR 224, DIV 2^224 or a 4-byte mask.
    let mut extraction: Option<(&'static str, usize)> = None;
    for (k, i) in ins.iter().enumerate() {
        if i.op != 0x35 {
            continue;
        }
        for d in 1..=6 {
            let Some(n) = at(k + d) else { break };
            let kind = match n.op {
                0x60 if imm(body, n) == [0xe0] && op_at(k + d + 1) == Some(0x1c) => Some("shr-224"),
                0x7c if imm(body, n).first() == Some(&1) && imm(body, n).iter().skip(1).all(|&b| b == 0) => Some("div-2^224"),
                0x63 if imm(body, n) == [0xff, 0xff, 0xff, 0xff] && (op_at(k + d + 1) == Some(0x16) || op_at(k + d + 2) == Some(0x16)) => Some("and-mask"),
                _ => None,
            };
            if let Some(kind) = kind {
                extraction = Some((kind, k));
                break;
            }
        }
        if extraction.is_some() {
            break;
        }
    }
    let disp_start = extraction.map(|(_, k)| k).unwrap_or(0);
    if extraction.is_none() {
        warnings.push("no selector extraction (CALLDATALOAD then SHR/DIV/AND) was seen; matches below are less certain".into());
    }

    // CALLVALUE check before the dispatcher: CALLVALUE DUP1 ISZERO PUSH JUMPI.
    let callvalue_before = ins.iter().take(disp_start).any(|i| i.op == 0x34);

    let mut functions: Vec<EvmFunction> = Vec::new();
    let mut splits = 0usize;
    let mut last_jumpi: Option<usize> = None;
    let mut solc_like = 0usize;
    let mut vyper_like = 0usize;
    for k in disp_start..ins.len() {
        let Some(i) = at(k) else { break };
        let n = push_len(i.op);
        if !(1..=4).contains(&n) {
            continue;
        }
        let Some(sel) = imm_u64(body, i) else { continue };
        let found = |dk: usize| -> Option<usize> {
            let p = at(dk)?;
            (is_push(p.op) && p.imm.1 <= 3 && op_at(dk + 1) == Some(0x57)).then_some(dk)
        };
        // PUSH sel, [DUP2], EQ, PUSH dest, JUMPI
        let eq_k = if op_at(k + 1) == Some(0x14) {
            Some(k + 1)
        } else if matches!(op_at(k + 1), Some(0x80..=0x8f)) && op_at(k + 2) == Some(0x14) {
            Some(k + 2)
        } else {
            None
        };
        let mut hit: Option<(usize, usize, &'static str)> = None;
        if let Some(e) = eq_k {
            if let Some(pk) = found(e + 1)
                && let Some(dest) = at(pk).and_then(|p| imm_u64(body, p))
            {
                hit = Some((usize::try_from(dest).unwrap_or(usize::MAX), pk + 1, "eq-jumpi"));
            } else if n == 4 && op_at(e + 1) == Some(0x15) && found(e + 2).is_some() {
                // Vyper-style: if selector != x jump to the next check; the body follows.
                if let Some(next) = at(e + 4) {
                    hit = Some((next.off, e + 3, "eq-iszero-fallthrough"));
                }
            }
        } else if n == 4 {
            // Vyper also loads the selector from memory: PUSH sel, PUSH1 0, MLOAD, EQ, ISZERO …
            for d in 1..=3 {
                if op_at(k + d) == Some(0x14) && op_at(k + d + 1) == Some(0x15) && found(k + d + 2).is_some() {
                    if let Some(next) = at(k + d + 4) {
                        hit = Some((next.off, k + d + 3, "eq-iszero-fallthrough"));
                    }
                    break;
                }
            }
            let direct = matches!(op_at(k + 1), Some(0x10 | 0x11)) && found(k + 2).is_some();
            let via_dup = matches!(op_at(k + 1), Some(0x80..=0x8f)) && matches!(op_at(k + 2), Some(0x10 | 0x11)) && found(k + 3).is_some();
            if hit.is_none() && (direct || via_dup) {
                splits += 1;
            }
        }
        let Some((dest, jumpi_k, pattern)) = hit else { continue };
        if n < 4 && extraction.is_none() {
            continue;
        }
        let selector = format!("0x{sel:08x}");
        if functions.iter().any(|f| f.selector == selector) {
            continue;
        }
        if pattern == "eq-jumpi" {
            solc_like += 1;
        } else {
            vyper_like += 1;
        }
        last_jumpi = Some(jumpi_k);
        let di = index_of(dest);
        let jumpdest_valid = pattern != "eq-jumpi" || di.and_then(at).is_some_and(|x| x.op == 0x5b);
        let (mutability, reason) = if callvalue_before {
            ("nonpayable", "a CALLVALUE check runs before the dispatcher, so every function refuses payments".to_string())
        } else {
            match di {
                Some(d) => payability(body, &ins, d, &index_of),
                None => ("unknown", "the destination is not an instruction boundary".to_string()),
            }
        };
        functions.push(EvmFunction {
            selector,
            bytecode_offset: dest,
            compare_offset: i.off,
            dispatch: "abi",
            pattern,
            mutability,
            mutability_reason: reason,
            jumpdest_valid,
        });
        if functions.len() >= MAX_FUNCTIONS {
            warnings.push(format!("stopped at {MAX_FUNCTIONS} selectors"));
            break;
        }
    }
    if functions.iter().any(|f| !f.jumpdest_valid) {
        warnings.push("some destinations are not JUMPDESTs; those matches may be data, not dispatch".into());
    }

    // Short calldata path and receive().
    let mut short_calldata = false;
    let mut receive = "not-detected";
    let mut fallback = "unknown";
    for (k, i) in ins.iter().enumerate().take(disp_start.saturating_add(64)) {
        if i.op != 0x36 {
            continue;
        }
        if op_at(k + 1) == Some(0x15) && at(k + 2).is_some_and(|p| is_push(p.op)) && op_at(k + 3) == Some(0x57) {
            receive = "present";
        }
        let lt = (1..=3).find(|d| op_at(k + d) == Some(0x10));
        if let Some(d) = lt
            && at(k + d + 1).is_some_and(|p| is_push(p.op))
            && op_at(k + d + 2) == Some(0x57)
        {
            short_calldata = true;
        }
    }
    if let Some(j) = last_jumpi
        && solc_like > 0
    {
        fallback = if reverts_soon(body, &ins, j + 1, &index_of) { "absent" } else { "present" };
    }
    let style = match (solc_like > 0, vyper_like > 0) {
        (true, true) => "mixed",
        (true, false) => "solc-like",
        (false, true) => "vyper-like",
        _ => "none",
    };
    let listing_end = last_jumpi.map(|j| j + 8).unwrap_or(LISTING_CAP).min(LISTING_CAP);
    let dispatcher_listing = ins
        .iter()
        .take(listing_end)
        .map(|i| {
            let d = imm(body, i);
            if d.is_empty() && push_len(i.op) == 0 {
                format!("{:#06x}: {}", i.off, mnemonic(i.op))
            } else {
                format!("{:#06x}: {} {}", i.off, mnemonic(i.op), bytes_value(d))
            }
        })
        .collect();
    if functions.is_empty() {
        warnings.push(
            "no selector dispatcher was recognised (the code may be creation code without runtime, a proxy, a library, or use a jump-table dispatcher)".into(),
        );
    }
    Ok(EvmInterface {
        code_size: code.len(),
        analysed_size: body.len(),
        metadata,
        instructions: ins.len(),
        jumpdests,
        selector_extraction: extraction.map(|(k, _)| k),
        dispatcher_offset: extraction.and_then(|(_, k)| at(k)).map(|i| i.off),
        style,
        functions,
        binary_search_splits: splits,
        callvalue_check_before_dispatch: callvalue_before,
        short_calldata_check: short_calldata,
        fallback,
        receive,
        dispatcher_listing,
        warnings,
    })
}

/// Follow a function's entry for a few instructions (and up to two unconditional jumps) looking
/// for a CALLVALUE check.
fn payability(code: &[u8], ins: &[Ins], start: usize, index_of: &dyn Fn(usize) -> Option<usize>) -> (&'static str, String) {
    let mut k = start;
    let mut hops = 0;
    for _ in 0..48 {
        let Some(i) = ins.get(k) else { break };
        match i.op {
            0x34 => return ("nonpayable", format!("checks CALLVALUE at {:#x}", i.off)),
            0x56 => {
                let target = k.checked_sub(1).and_then(|p| ins.get(p)).filter(|p| is_push(p.op)).and_then(|p| imm_u64(code, p));
                match target.and_then(|t| index_of(usize::try_from(t).ok()?)) {
                    Some(t) if hops < 2 => {
                        hops += 1;
                        k = t;
                        continue;
                    }
                    _ => return ("unknown", "jumps somewhere we cannot follow before any CALLVALUE check".into()),
                }
            }
            0x57 => return ("unknown", "branches before any CALLVALUE check".into()),
            0x00 | 0xf3 | 0xfd | 0xfe | 0xff | 0x35 | 0x37 | 0x54 | 0x55 | 0xf1 | 0xf4 | 0xfa | 0xa0..=0xa4 => {
                return ("payable", format!("reaches {} at {:#x} without a CALLVALUE check", mnemonic(i.op), i.off));
            }
            _ => {}
        }
        k += 1;
    }
    ("unknown", "no CALLVALUE check in the first instructions".into())
}

fn reverts_soon(code: &[u8], ins: &[Ins], mut k: usize, index_of: &dyn Fn(usize) -> Option<usize>) -> bool {
    let mut hops = 0;
    for _ in 0..12 {
        let Some(i) = ins.get(k) else { return false };
        match i.op {
            0xfd | 0xfe => return true,
            0x56 => {
                let target = k.checked_sub(1).and_then(|p| ins.get(p)).filter(|p| is_push(p.op)).and_then(|p| imm_u64(code, p));
                match target.and_then(|t| index_of(usize::try_from(t).ok()?)) {
                    Some(t) if hops < 2 => {
                        hops += 1;
                        k = t;
                        continue;
                    }
                    _ => return false,
                }
            }
            0x5b | 0x50 | 0x80..=0x9f | 0x5f | 0x60..=0x7f => {}
            _ => return false,
        }
        k += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_anything_testkit::formats as fx;

    #[test]
    fn dispatcher_selectors_and_metadata() {
        let (code, dests) = fx::evm_runtime();
        let r = analyze(&code).unwrap();
        assert_eq!(r.selector_extraction, Some("shr-224"));
        assert_eq!(r.style, "solc-like");
        let sels: Vec<_> = r.functions.iter().map(|f| f.selector.clone()).collect();
        assert_eq!(sels, fx::EVM_SELECTORS.iter().map(|s| format!("0x{s:08x}")).collect::<Vec<_>>());
        for (f, d) in r.functions.iter().zip(dests) {
            assert_eq!(f.bytecode_offset, d);
            assert!(f.jumpdest_valid);
        }
        assert_eq!(r.functions[0].mutability, "nonpayable");
        assert_eq!(r.functions[1].mutability, "nonpayable");
        assert_eq!(r.functions[2].mutability, "payable");
        assert!(r.short_calldata_check);
        assert_eq!(r.fallback, "absent");
        let m = r.metadata.unwrap();
        assert_eq!(m.compiler.as_deref(), Some("solc 0.8.19"));
        assert!(m.ipfs.unwrap().starts_with("Qm"));
        assert_eq!(r.analysed_size + m.length, code.len());
    }

    #[test]
    fn hex_input_rules() {
        let (code, _) = fx::evm_runtime();
        let hex = fx::to_hex(&code);
        assert_eq!(decode_input(format!("  {hex}\n").as_bytes(), Encoding::Auto).unwrap(), (code.clone(), "hex"));
        assert_eq!(decode_input(&code, Encoding::Auto).unwrap().1, "raw");
        assert!(decode_input(b"0x123", Encoding::Hex).is_err());
        assert!(decode_input(b"0xzz", Encoding::Hex).is_err());
        assert!(decode_input(b"", Encoding::Hex).is_err());
        assert!(matches!(analyze(&[0xef, 0x00, 0x01]), Err(FormatError::Unsupported(_))));
        assert_eq!(base58(&[0, 0, 1]), "112");
    }

    #[test]
    fn vyper_style_fallthrough() {
        // PUSH1 0 CALLDATALOAD PUSH1 0xe0 SHR PUSH1 0 MSTORE PUSH4 sel PUSH1 0 MLOAD EQ ISZERO PUSH2 next JUMPI <body: CALLVALUE ...>
        let mut c =
            vec![0x60, 0x00, 0x35, 0x60, 0xe0, 0x1c, 0x60, 0x00, 0x52, 0x63, 0x12, 0x34, 0x56, 0x78, 0x60, 0x00, 0x51, 0x14, 0x15, 0x61, 0x00, 0x20, 0x57];
        let body = c.len();
        c.extend_from_slice(&[0x34, 0x15, 0x00]);
        let r = analyze(&c).unwrap();
        assert_eq!(r.style, "vyper-like");
        assert_eq!(r.functions[0].selector, "0x12345678");
        assert_eq!(r.functions[0].bytecode_offset, body);
        assert_eq!(r.functions[0].mutability, "nonpayable");
    }

    #[test]
    fn hostile_inputs_do_not_panic() {
        let (code, _) = fx::evm_runtime();
        for m in craft_anything_testkit::mutations(&code) {
            let _ = analyze(&m);
        }
        for n in 0..code.len() {
            let _ = analyze(&code[..n]);
        }
        for seed in 0..200 {
            let n = craft_anything_testkit::noise(seed, 300);
            let _ = analyze(&n);
            let _ = decode_input(&n, Encoding::Auto);
        }
        // A metadata length that claims everything, and nested CBOR.
        let _ = analyze(&[0xa1, 0xa1, 0xa1, 0xa1, 0xa1, 0xa1, 0xa1, 0x00, 0x07]);
        let _ = analyze(&[0x7f; 40]);
    }
}
