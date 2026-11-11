//! ASCII85 (`ASCII85Encode` / `ASCII85Decode`): four bytes as five characters `!`…`u`, four zero
//! bytes as `z`, ended by `~>`.

/// Characters a line of encoded data has, about.
const LINE: usize = 72;

/// `bytes` in ASCII85 with its `~>` end, in lines of about 72 characters, none starting with `%`
/// (a DSC reader would take such a line for a comment).
pub fn encode(bytes: &[u8]) -> String {
    let mut raw = String::with_capacity(bytes.len() * 5 / 4 + 8);
    let (chunks, rest) = bytes.as_chunks::<4>();
    for c in chunks {
        let v = u32::from_be_bytes(*c);
        if v == 0 {
            raw.push('z');
        } else {
            push85(&mut raw, v, 5);
        }
    }
    if !rest.is_empty() {
        let mut c = [0u8; 4];
        for (d, s) in c.iter_mut().zip(rest) {
            *d = *s;
        }
        push85(&mut raw, u32::from_be_bytes(c), rest.len() + 1);
    }
    raw.push_str("~>");
    let mut out = String::with_capacity(raw.len() + raw.len() / LINE + 2);
    if raw.starts_with('%') {
        // Whitespace is ignored by readers, and keeps the first line from starting with `%`.
        out.push(' ');
    }
    let mut col = 0;
    for ch in raw.chars() {
        if col >= LINE && ch != '%' {
            out.push('\n');
            col = 0;
        }
        out.push(ch);
        col += 1;
    }
    out.push('\n');
    out
}

/// The first `n` base-85 digits of `v`.
fn push85(s: &mut String, mut v: u32, n: usize) {
    let mut d = [0u8; 5];
    for slot in d.iter_mut().rev() {
        *slot = (v % 85) as u8 + b'!';
        v /= 85;
    }
    for b in d.iter().take(n) {
        s.push(char::from(*b));
    }
}

/// ASCII85 `text` (whitespace ignored, up to `~>`) decoded. `None` when it is malformed, has no
/// end, or decodes to more than `max` bytes: a `z` is four bytes for one character, so the length
/// of the text doesn't bound what it becomes.
pub fn decode(text: &[u8], max: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity((text.len() / 5 * 4).min(max));
    let mut group = [0u8; 5];
    let mut n = 0;
    let mut chars = text.iter().copied().filter(|b| !b.is_ascii_whitespace());
    while let Some(b) = chars.next() {
        if out.len() > max {
            return None;
        }
        match b {
            b'~' => {
                // A single left-over character can't be a byte.
                if chars.next() != Some(b'>') || n == 1 {
                    return None;
                }
                if n > 1 {
                    let mut padded = [b'u'; 5];
                    for (d, s) in padded.iter_mut().zip(group.iter().take(n)) {
                        *d = *s;
                    }
                    out.extend(value85(&padded)?.to_be_bytes().iter().take(n - 1));
                }
                return (out.len() <= max).then_some(out);
            }
            b'z' if n == 0 => out.extend([0; 4]),
            b'!'..=b'u' => {
                *group.get_mut(n)? = b;
                n += 1;
                if n == 5 {
                    out.extend(value85(&group)?.to_be_bytes());
                    n = 0;
                }
            }
            _ => return None,
        }
    }
    None
}

/// The value of five base-85 digits; `None` past `u32::MAX`.
fn value85(digits: &[u8]) -> Option<u32> {
    digits.iter().try_fold(0u32, |acc, d| acc.checked_mul(85)?.checked_add(u32::from(d.checked_sub(b'!')?)))
}
