//! PackBits (TIFF 6.0 §9) decompression.

/// Appends the bytes `src` decompresses to onto `out`, stopping once `out` holds `limit` bytes or
/// at a run that the data ends inside. A header of −128 is a no-op.
pub fn decode(src: &[u8], out: &mut Vec<u8>, limit: usize) {
    let mut p = 0;
    while out.len() < limit {
        let Some(&n) = src.get(p) else { return };
        let n = n as i8;
        p += 1;
        if n >= 0 {
            let len = n as usize + 1;
            let Some(lit) = p.checked_add(len).and_then(|end| src.get(p..end)) else { return };
            out.extend_from_slice(lit.get(..len.min(limit - out.len())).unwrap_or(lit));
            p += len;
        } else if n != -128 {
            let Some(&b) = src.get(p) else { return };
            let len = (1 - isize::from(n)) as usize;
            out.extend(std::iter::repeat_n(b, len.min(limit - out.len())));
            p += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_literals_and_noops() {
        let mut out = Vec::new();
        // A literal of 2, a no-op, a run of 3, then a literal cut short by the end of the data.
        decode(&[1, 10, 11, 0x80, 0xFE, 7, 4, 1], &mut out, 100);
        assert_eq!(out, [10, 11, 7, 7, 7]);
        let mut capped = Vec::new();
        decode(&[0xFE, 7, 0xFE, 8], &mut capped, 4);
        assert_eq!(capped, [7, 7, 7, 8]);
        // The longest run, 128 bytes from two, appended to what is there.
        let mut out = vec![1];
        decode(&[0x81, 9], &mut out, 1000);
        assert_eq!((out.len(), out[1], out[128]), (129, 9, 9));
        let mut out = Vec::new();
        decode(&[0xFF], &mut out, 10);
        assert!(out.is_empty());
    }
}
