//! AV1 in a transport stream (Carriage of AV1 in MPEG-2 TS, §3): each OBU after the start code
//! 0x000001, with emulation prevention so that 0x000000, 0x000001, 0x000002 and 0x000003 never
//! appear inside it. Temporal delimiters are left out, and so are padding OBUs that end with a
//! zero byte (an empty one included), which decoders ignore.

use super::Error;

const OBU_SEQUENCE_HEADER: u8 = 1;
const OBU_TEMPORAL_DELIMITER: u8 = 2;
const OBU_TILE_LIST: u8 = 8;
const OBU_PADDING: u8 = 15;

struct Obu {
    /// Bytes from the OBU header to the end of its payload.
    len: usize,
    kind: u8,
}

/// The OBU at the start of `d`.
fn obu(d: &[u8]) -> Result<Obu, Error> {
    let &h = d.first().ok_or(Error::BadFrame("an AV1 frame ends inside an OBU header"))?;
    if h & 0x80 != 0 {
        return Err(Error::BadFrame("an AV1 OBU has its forbidden bit set"));
    }
    if h & 0x02 == 0 {
        return Err(Error::BadFrame("AV1 OBUs must carry obu_size (the low overhead bitstream format)"));
    }
    let mut at = 1 + usize::from(h & 0x04 != 0);
    let mut size: u64 = 0;
    for i in 0..8 {
        let &b = d.get(at).ok_or(Error::BadFrame("an AV1 frame ends inside an OBU header"))?;
        at += 1;
        size |= u64::from(b & 0x7F) << (7 * i);
        if b & 0x80 == 0 {
            break;
        }
        if i == 7 {
            return Err(Error::BadFrame("an AV1 obu_size is longer than eight bytes"));
        }
    }
    let len = usize::try_from(size).ok().filter(|&s| s <= u32::MAX as usize).and_then(|s| at.checked_add(s)).filter(|&l| l <= d.len());
    let len = len.ok_or(Error::BadFrame("an AV1 OBU runs past the end of the frame"))?;
    Ok(Obu { len, kind: (h >> 3) & 0x0F })
}

/// The OBU `bytes` (as `obu` found it) is not written: a temporal delimiter, or a padding OBU
/// that ends with a zero byte.
fn left_out(o: &Obu, bytes: &[u8]) -> bool {
    o.kind == OBU_TEMPORAL_DELIMITER || (o.kind == OBU_PADDING && bytes.last() == Some(&0))
}

/// Emulation prevention bytes `bytes` needs after a start code.
fn escapes(bytes: &[u8]) -> usize {
    let (mut n, mut zeros) = (0, 0u8);
    for &b in bytes {
        if zeros >= 2 && b <= 3 {
            n += 1;
            zeros = 0;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
    }
    n
}

/// Checks a temporal unit and returns the bytes it takes in the PES packet: start codes, OBUs
/// and emulation prevention bytes.
pub(crate) fn check(data: &[u8], key: bool) -> Result<usize, Error> {
    let (mut at, mut body, mut seq, mut any) = (0, 0usize, false, false);
    while at < data.len() {
        let o = obu(&data[at..])?;
        let bytes = &data[at..at + o.len];
        at += o.len;
        if left_out(&o, bytes) {
            continue;
        }
        match o.kind {
            OBU_TILE_LIST => return Err(Error::BadFrame("tile list OBUs are not carried in a transport stream")),
            OBU_SEQUENCE_HEADER => seq = true,
            _ => {}
        }
        if bytes.last() == Some(&0) {
            return Err(Error::BadFrame("an AV1 OBU ends with a zero byte, which the next start code would turn into a forbidden sequence"));
        }
        body = body.saturating_add(3 + bytes.len() + escapes(bytes));
        any = true;
    }
    if !any {
        return Err(Error::BadFrame("the AV1 frame holds only temporal delimiters and padding"));
    }
    if key && !seq {
        return Err(Error::BadFrame("an AV1 keyframe must carry a sequence header"));
    }
    Ok(body)
}

/// Writes a checked temporal unit in start code form, a packet at a time.
pub(crate) struct Escaper<'a> {
    data: &'a [u8],
    at: usize,
    /// End of the OBU being written.
    end: usize,
    /// Start code bytes still to write.
    start_code: u8,
    zeros: u8,
}

impl<'a> Escaper<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0, end: 0, start_code: 0, zeros: 0 }
    }

    /// Fills `out` as far as the temporal unit goes; returns the bytes written.
    pub(crate) fn fill(&mut self, out: &mut [u8]) -> usize {
        let mut n = 0;
        while n < out.len() {
            if self.start_code > 0 {
                out[n] = u8::from(self.start_code == 1);
                self.start_code -= 1;
                n += 1;
            } else if self.at < self.end {
                let b = self.data[self.at];
                if self.zeros >= 2 && b <= 3 {
                    out[n] = 3;
                    self.zeros = 0;
                } else {
                    out[n] = b;
                    self.at += 1;
                    self.zeros = if b == 0 { self.zeros + 1 } else { 0 };
                }
                n += 1;
            } else {
                let rest = self.data.get(self.at..).unwrap_or(&[]);
                let Ok(o) = obu(rest) else { break };
                if left_out(&o, &rest[..o.len]) {
                    self.at += o.len;
                    continue;
                }
                self.end = self.at + o.len;
                self.start_code = 3;
                self.zeros = 0;
            }
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Removes start codes and emulation prevention bytes: the OBUs back.
    fn unescape(es: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut zeros = 0;
        let mut i = 0;
        while i < es.len() {
            if es[i..].starts_with(&[0, 0, 1]) {
                i += 3;
                zeros = 0;
                continue;
            }
            let b = es[i];
            i += 1;
            if zeros >= 2 && b == 3 {
                zeros = 0;
                continue;
            }
            zeros = if b == 0 { zeros + 1 } else { 0 };
            out.push(b);
        }
        out
    }

    fn write(data: &[u8], chunk: usize) -> Vec<u8> {
        let mut e = Escaper::new(data);
        let mut out = Vec::new();
        loop {
            let mut b = vec![0u8; chunk];
            let n = e.fill(&mut b);
            out.extend_from_slice(&b[..n]);
            if n < chunk {
                return out;
            }
        }
    }

    #[test]
    fn obus_get_start_codes_and_escapes() {
        // a temporal delimiter, a sequence header, a frame OBU whose payload holds every
        // sequence that needs an escape
        let tu = [0x12, 0x00, 0x0A, 0x03, 0x00, 0x00, 0x01, 0x32, 0x0A, 0, 0, 0, 0, 0, 1, 0, 0, 3, 0x7F];
        let body = check(&tu, true).unwrap();
        for chunk in [1, 2, 5, 184] {
            let es = write(&tu, chunk);
            assert_eq!(es.len(), body, "chunk {chunk}");
            assert!(es.starts_with(&[0, 0, 1, 0x0A, 0x03, 0x00, 0x00, 0x03, 0x01, 0, 0, 1, 0x32]));
            for w in es[3..].windows(3) {
                assert!(!(w[0] == 0 && w[1] == 0 && w[2] <= 2) || w == [0, 0, 1], "{es:02X?}");
            }
            assert_eq!(unescape(&es), tu[2..].to_vec());
        }
    }

    #[test]
    fn padding_that_ends_with_a_zero_byte_is_left_out() {
        // a sequence header, an empty padding OBU, a frame OBU, a padding OBU ending in 0x00,
        // and a padding OBU that is written
        let tu = [0x0A, 0x01, 0x05, 0x7A, 0x00, 0x32, 0x01, 0x07, 0x7A, 0x02, 0x09, 0x00, 0x7A, 0x01, 0x09];
        let body = check(&tu, true).unwrap();
        for chunk in [1, 2, 5, 184] {
            let es = write(&tu, chunk);
            assert_eq!(es.len(), body, "chunk {chunk}");
            assert_eq!(es, [0, 0, 1, 0x0A, 0x01, 0x05, 0, 0, 1, 0x32, 0x01, 0x07, 0, 0, 1, 0x7A, 0x01, 0x09], "chunk {chunk}");
        }
    }

    #[test]
    fn bad_temporal_units_are_refused() {
        assert!(check(&[0x12, 0x00], false).is_err(), "only a temporal delimiter");
        assert!(check(&[0x32, 0x01, 0x05], true).is_err(), "a keyframe without a sequence header");
        assert!(check(&[0x32, 0x01, 0x05], false).is_ok());
        assert!(check(&[0x30, 0x05], false).is_err(), "no obu_size");
        assert!(check(&[0xB2, 0x01, 0x05], false).is_err(), "forbidden bit");
        assert!(check(&[0x32, 0x05, 0x05], false).is_err(), "past the end");
        assert!(check(&[0x32, 0x01, 0x00], false).is_err(), "ends with a zero byte");
        assert!(check(&[0x42, 0x01, 0x05], false).is_err(), "tile list");
        assert!(check(&[0x7A, 0x00], false).is_err(), "only empty padding");
        assert!(check(&[0x7A, 0x02, 0x05, 0x00], false).is_err(), "only padding that ends with a zero byte");
        assert!(check(&[0x32, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01], false).is_err(), "a nine-byte size");
        assert!(check(&[0x32, 0x80, 0x80, 0x80, 0x80, 0x10], false).is_err(), "over 2^32 - 1");
    }
}
