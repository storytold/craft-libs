//! `udta` and `meta` text metadata, bounded by a byte budget.

use super::*;

/// Takes an entry of `key_len` and `value_len` bytes from `left`; false once it would not fit.
fn charge_metadata(left: &mut usize, key_len: usize, value_len: usize) -> bool {
    let cost = key_len.saturating_add(value_len).saturating_add(METADATA_ENTRY_COST);
    match left.checked_sub(cost) {
        Some(rest) => {
            *left = rest;
            true
        }
        None => false,
    }
}

/// Length of `String::from_utf8_lossy(bytes)` without building it: each invalid sequence
/// becomes one three-byte U+FFFD.
pub(super) fn lossy_len(bytes: &[u8]) -> usize {
    bytes.utf8_chunks().map(|c| c.valid().len() + if c.invalid().is_empty() { 0 } else { 3 }).sum()
}

pub(super) fn parse_udta(p: &[u8], md: &mut Metadata, left: &mut usize) {
    for b in boxes(p).map_while(|b| b.ok()) {
        if b.kind == b"meta" {
            parse_meta(b.payload, md, left);
        } else if b.kind.0[0] == 0xA9 && b.payload.len() >= 4 {
            let n = u16::from_be_bytes([b.payload[0], b.payload[1]]) as usize;
            // Charged at the stored text's length, before the value is converted.
            let key = b.kind.to_string_lossy();
            if let Some(s) = b.payload.get(4..4 + n)
                && charge_metadata(left, key.len(), lossy_len(s))
            {
                md.entries.push((key, String::from_utf8_lossy(s).into_owned()));
            }
        }
    }
}

pub(super) fn parse_meta(p: &[u8], md: &mut Metadata, left: &mut usize) {
    // QuickTime `meta` is a plain box; ISO `meta` is a full box.
    let body = if p.len() >= 8 && &p[4..8] == b"hdlr" { p } else { p.get(4..).unwrap_or(&[]) };
    let mut keys: Vec<String> = Vec::new();
    if let Some(k) = find(body, b"keys") {
        let mut c = Cur::labeled(k, "keys");
        if c.full_header().is_ok()
            && let Ok(n) = c.count(8)
        {
            for _ in 0..n {
                let Ok(sz) = c.u32() else { break };
                let Ok(_ns) = c.u32() else { break };
                let Ok(name) = c.bytes((sz as usize).saturating_sub(8)) else { break };
                keys.push(String::from_utf8_lossy(name).into_owned());
            }
        }
    }
    let Some(ilst) = find(body, b"ilst") else { return };
    for item in boxes(ilst).map_while(|b| b.ok()) {
        let idx = item.kind.as_u32() as usize;
        let named = if idx >= 1 { keys.get(idx - 1) } else { None };
        if let Some(d) = find(item.payload, b"data")
            && d.len() >= 8
        {
            let ty = u32::from_be_bytes([d[0], d[1], d[2], d[3]]) & 0xFF_FFFF;
            // Charged before the key is copied (every item naming one long key is another copy)
            // and before the value is converted, at the lengths they will be stored at.
            if ty == 1 || ty == 0 {
                let fallback = if named.is_none() { Some(item.kind.to_string_lossy()) } else { None };
                let key_len = named.or(fallback.as_ref()).map_or(0, String::len);
                if charge_metadata(left, key_len, lossy_len(&d[8..])) {
                    let key = named.cloned().or(fallback).unwrap_or_default();
                    md.entries.push((key, String::from_utf8_lossy(&d[8..]).into_owned()));
                }
            }
        }
    }
}
