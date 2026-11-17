//! Encoded ICC normalization extracted from LightCraft.
pub(super) fn align_tags(b: &[u8]) -> Vec<u8> {
    let be = |i: usize| b.get(i..i + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize);
    let Some(n) = be(128) else { return b.to_vec() };
    let table_end = 132 + 12 * n;
    let tags: Option<Vec<(usize, usize)>> = (0..n).map(|t| Some((be(132 + 12 * t + 4)?, be(132 + 12 * t + 8)?))).collect();
    let Some(tags) = tags.filter(|t| table_end <= b.len() && t.iter().all(|&(o, l)| o >= table_end && o + l <= b.len())) else {
        return b.to_vec();
    };
    let mut out = b[..table_end].to_vec();
    let mut placed: Vec<((usize, usize), usize)> = Vec::new();
    for (t, &(off, len)) in tags.iter().enumerate() {
        let new_off = match placed.iter().find(|(k, _)| *k == (off, len)) {
            // tags sharing data keep sharing it
            Some(&(_, o)) => o,
            None => {
                while !out.len().is_multiple_of(4) {
                    out.push(0);
                }
                let o = out.len();
                out.extend_from_slice(&b[off..off + len]);
                placed.push(((off, len), o));
                o
            }
        };
        out[132 + 12 * t + 4..132 + 12 * t + 8].copy_from_slice(&(new_off as u32).to_be_bytes());
    }
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
    let size = out.len() as u32;
    out[0..4].copy_from_slice(&size.to_be_bytes());
    // the profile ID (MD5) would no longer match: clear it (= "not computed")
    out[84..100].fill(0);
    out
}

pub(super) fn fixed_date(mut b: Vec<u8>) -> Vec<u8> {
    if let Some(date) = b.get_mut(24..36) {
        date.fill(0);
    }
    b
}
