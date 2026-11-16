//! Reading `presentation.json`, including files written with NaN or infinity saved as `null`.

use serde::de::DeserializeOwned;

use crate::{FormatError, Manifest, Result};

/// Most `null`s standing in for numbers that [`parse_manifest`] patches in one file, and the
/// candidates it tries for each.
const MAX_NULL_FIXES: usize = 256;
const MAX_TRIES_PER_FIX: usize = 32;
/// Bytes [`parse_manifest`] may re-parse in all while patching, so a hostile file full of `null`s
/// can't keep it busy: past this the file is reported damaged.
const MAX_REPARSE_BYTES: usize = 256 * 1024 * 1024;

/// Where a parse failed, comparable: later is progress.
fn failure<T: DeserializeOwned>(src: &[u8]) -> std::result::Result<Manifest<T>, (String, (usize, usize))> {
    serde_json::from_slice::<Manifest<T>>(src).map_err(|e| (e.to_string(), (e.line(), e.column())))
}

/// Parse `presentation.json`. serde_json writes a NaN or infinite number as `null`, which then
/// fails to read back as a number ("invalid type: null, expected f64"), so DeckCraft could save
/// a file it couldn't open. Each such `null` is read as 0 instead (the app then brings the
/// value into range); anything else that doesn't parse is damage.
///
/// serde reports the failing `null` exactly, except inside tagged enums (buffered), where it
/// reports the end of the enclosing object: there the `null`s before that point are tried
/// nearest first, and a patch is kept only if the parse then gets further. (Files saved since
/// [`crate::Finite`] never contain such `null`s; this reads older ones.)
pub fn parse_manifest<T: DeserializeOwned>(json: &[u8]) -> Result<Manifest<T>> {
    // Parse the inflated text as it is: only a file that fails on a `null` number is copied for
    // patching (the copy is bounded by `MAX_MANIFEST`).
    let mut last = match failure(json) {
        Ok(m) => return Ok(m),
        Err(f) => f,
    };
    if !is_null_number(&last.0) {
        return Err(FormatError::Damaged(last.0));
    }
    let mut src = json.to_vec();
    let mut budget = MAX_REPARSE_BYTES;
    'fixes: for _ in 0..MAX_NULL_FIXES {
        let (msg, pos) = &last;
        if !is_null_number(msg) {
            break;
        }
        let end = offset(&src, pos.0, pos.1).unwrap_or(src.len());
        let mut progressed = false;
        let candidates: Vec<usize> = value_nulls_before(&src, end + 4).take(MAX_TRIES_PER_FIX).collect();
        for at in candidates {
            let (Some(head), Some(tail)) = (src.get(..at), src.get(at.saturating_add(4)..)) else { continue };
            let Some(left) = budget.checked_sub(src.len()) else { break 'fixes };
            budget = left;
            let mut next = Vec::with_capacity(src.len());
            next.extend_from_slice(head);
            next.push(b'0');
            next.extend_from_slice(tail);
            match failure(&next) {
                Ok(m) => return Ok(m),
                // The patch shortened the text by 3 bytes before the error: compare positions
                // allowing for that.
                Err(f) if f.1 > (pos.0, pos.1.saturating_sub(3)) => {
                    src = next;
                    last = f;
                    progressed = true;
                    break;
                }
                Err(_) => {}
            }
        }
        if !progressed {
            break;
        }
    }
    Err(FormatError::Damaged(last.0))
}

/// Is this parse error a `null` where a number belongs (a NaN or infinity serde_json wrote)?
fn is_null_number(msg: &str) -> bool {
    msg.starts_with("invalid type: null, expected f") || msg.starts_with("invalid type: null, expected a number")
}

/// Byte offset of a 1-based `line`:`column`.
fn offset(src: &[u8], line: usize, column: usize) -> Option<usize> {
    let mut start = 0;
    for _ in 1..line {
        start += src.get(start..)?.iter().position(|b| *b == b'\n')? + 1;
    }
    Some((start + column).min(src.len()))
}

/// Starts of `null` tokens in value position (after `:`, `[` or `,`) before `end`, nearest first.
fn value_nulls_before(src: &[u8], end: usize) -> impl Iterator<Item = usize> + '_ {
    let end = end.min(src.len());
    (0..end.saturating_sub(3)).rev().filter(move |&i| {
        src.get(i..i + 4) == Some(b"null")
            && src[..i].iter().rev().find(|b| !b.is_ascii_whitespace()).is_some_and(|b| matches!(b, b':' | b'[' | b','))
            && src.get(i + 4).is_none_or(|b| !b.is_ascii_alphanumeric())
    })
}
