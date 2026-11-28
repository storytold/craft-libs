//! Embroidery machine files read as stitch runs: Tajima DST, Melco EXP, Janome JEF and
//! Brother PES (its PEC stitch block). Written from the published descriptions of the record
//! layouts; no thread-colour tables are included (they are vendors' data), so each colour block
//! is reported by its index.
//!
//! Every reader works on untrusted bytes: lengths and offsets are checked, the number of
//! stitches is capped ([`MAX_STITCHES`]) and coordinates are kept within ±[`MAX_EXTENT_MM`].
//! Coordinates are millimetres with y down (the files store 0.1 mm steps; DST, EXP and JEF
//! count y upwards, PEC downwards).

use kurbo::{BezPath, Point};

/// The most stitches read from one file.
pub const MAX_STITCHES: usize = 2_000_000;
/// How far from the start any stitch may land, in millimetres.
pub const MAX_EXTENT_MM: f64 = 10_000.0;

/// The stitch-file families this crate reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Dst,
    Exp,
    Jef,
    Pes,
}

impl Format {
    /// The format a file named `name` holding `bytes` is in: by its signature where it has one
    /// (PES), else by its extension.
    pub fn detect(name: &str, bytes: &[u8]) -> Option<Format> {
        if bytes.starts_with(b"#PES") {
            return Some(Format::Pes);
        }
        let extension = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
        match extension.as_str() {
            "dst" => Some(Format::Dst),
            "exp" => Some(Format::Exp),
            "jef" => Some(Format::Jef),
            _ => None,
        }
    }
}

/// One run of stitches sewn without a jump, in one colour block.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    /// The colour block, counting from 0 at each colour change.
    pub colour: usize,
    /// Needle positions in millimetres, y down; at least two.
    pub points: Vec<Point>,
}

impl Run {
    /// The run as an open polyline path.
    pub fn path(&self) -> BezPath {
        let mut path = BezPath::new();
        for (i, p) in self.points.iter().enumerate() {
            if i == 0 {
                path.move_to(*p);
            } else {
                path.line_to(*p);
            }
        }
        path
    }
}

/// A design: its stitch runs and how many colour blocks it has.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Design {
    pub runs: Vec<Run>,
    pub colours: usize,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("{0}: the file is too short")]
    Truncated(&'static str),
    #[error("{0}: {1}")]
    Invalid(&'static str, String),
    #[error("more than {MAX_STITCHES} stitches")]
    TooManyStitches,
    #[error("a stitch lands more than {MAX_EXTENT_MM} mm from the start")]
    TooFar,
}

/// Reads a stitch file.
pub fn read(format: Format, bytes: &[u8]) -> Result<Design, Error> {
    match format {
        Format::Dst => read_dst(bytes),
        Format::Exp => read_exp(bytes),
        Format::Jef => read_jef(bytes),
        Format::Pes => read_pes(bytes),
    }
}

/// What a stitch record does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Move {
    Stitch,
    Jump,
    ColourChange,
}

/// Collects needle moves (in 0.1 mm, y as stored) into runs.
struct Builder {
    design: Design,
    current: Vec<Point>,
    at: (i64, i64),
    /// −1 when the file counts y upwards, so the design comes out y down.
    y_sign: f64,
    count: usize,
}

impl Builder {
    fn new(y_up: bool) -> Self {
        Builder { design: Design { runs: Vec::new(), colours: 1 }, current: Vec::new(), at: (0, 0), y_sign: if y_up { -1.0 } else { 1.0 }, count: 0 }
    }

    fn point(&self) -> Point {
        Point::new(self.at.0 as f64 / 10.0, self.y_sign * self.at.1 as f64 / 10.0)
    }

    fn end_run(&mut self) {
        let points = std::mem::take(&mut self.current);
        if points.len() >= 2 {
            self.design.runs.push(Run { colour: self.design.colours.saturating_sub(1), points });
        }
    }

    fn apply(&mut self, dx: i64, dy: i64, kind: Move) -> Result<(), Error> {
        self.count += 1;
        if self.count > MAX_STITCHES {
            return Err(Error::TooManyStitches);
        }
        let limit = (MAX_EXTENT_MM * 10.0) as i64;
        let (x, y) = (self.at.0.saturating_add(dx), self.at.1.saturating_add(dy));
        if x.abs() > limit || y.abs() > limit {
            return Err(Error::TooFar);
        }
        match kind {
            Move::ColourChange => {
                self.end_run();
                self.at = (x, y);
                self.design.colours = self.design.colours.saturating_add(1);
            }
            Move::Jump => {
                self.end_run();
                self.at = (x, y);
            }
            Move::Stitch => {
                if self.current.is_empty() {
                    let start = self.point();
                    self.current.push(start);
                }
                self.at = (x, y);
                let p = self.point();
                self.current.push(p);
            }
        }
        Ok(())
    }

    fn finish(mut self) -> Design {
        self.end_run();
        // A file that only changes colour at its end does not add a block.
        self.design.colours = self.design.runs.iter().map(|r| r.colour + 1).max().unwrap_or(0);
        self.design
    }
}

/// Tajima DST: a 512-byte text header, then 3-byte records whose bits add ±1, ±3, ±9, ±27 and
/// ±81 steps on each axis; 0xF3 in the third byte ends the design.
fn read_dst(bytes: &[u8]) -> Result<Design, Error> {
    const NAME: &str = "DST";
    let body = bytes.get(512..).ok_or(Error::Truncated(NAME))?;
    let mut b = Builder::new(true);
    for &[b0, b1, b2] in body.as_chunks::<3>().0 {
        if b2 == 0xF3 {
            break;
        }
        let bit = |byte: u8, mask: u8, value: i64| if byte & mask != 0 { value } else { 0 };
        let dx = bit(b0, 0x01, 1) - bit(b0, 0x02, 1) + bit(b0, 0x04, 9) - bit(b0, 0x08, 9) + bit(b1, 0x01, 3) - bit(b1, 0x02, 3) + bit(b1, 0x04, 27)
            - bit(b1, 0x08, 27)
            + bit(b2, 0x04, 81)
            - bit(b2, 0x08, 81);
        let dy = bit(b0, 0x80, 1) - bit(b0, 0x40, 1) + bit(b0, 0x20, 9) - bit(b0, 0x10, 9) + bit(b1, 0x80, 3) - bit(b1, 0x40, 3) + bit(b1, 0x20, 27)
            - bit(b1, 0x10, 27)
            + bit(b2, 0x20, 81)
            - bit(b2, 0x10, 81);
        let kind = if b2 & 0xC3 == 0xC3 {
            Move::ColourChange
        } else if b2 & 0x80 != 0 {
            Move::Jump
        } else {
            Move::Stitch
        };
        b.apply(dx, dy, kind)?;
    }
    Ok(b.finish())
}

/// Two signed bytes (dx, dy), or 0x80 and a command byte: 0x01 colour change, 0x02 or 0x04 a
/// move (its dx, dy follow), 0x10 or 0x80 the end. Melco EXP and Janome JEF stitch data.
fn read_byte_pairs(name: &'static str, data: &[u8], y_up: bool) -> Result<Design, Error> {
    let mut b = Builder::new(y_up);
    let mut i = 0usize;
    let signed = |v: u8| i64::from(v as i8);
    while let (Some(&x), Some(&y)) = (data.get(i), data.get(i + 1)) {
        i += 2;
        if x != 0x80 {
            b.apply(signed(x), signed(y), Move::Stitch)?;
            continue;
        }
        match y {
            0x01 => {
                // A colour change may carry the move to the next colour's start.
                let (dx, dy) = match (data.get(i), data.get(i + 1)) {
                    (Some(&dx), Some(&dy)) if dx != 0x80 => {
                        i += 2;
                        (signed(dx), signed(dy))
                    }
                    _ => (0, 0),
                };
                b.apply(dx, dy, Move::ColourChange)?;
            }
            0x02 | 0x04 => {
                let (Some(&dx), Some(&dy)) = (data.get(i), data.get(i + 1)) else {
                    return Err(Error::Truncated(name));
                };
                i += 2;
                b.apply(signed(dx), signed(dy), Move::Jump)?;
            }
            0x10 | 0x80 => break,
            other => return Err(Error::Invalid(name, format!("unknown command 0x{other:02x}"))),
        }
    }
    Ok(b.finish())
}

fn read_exp(bytes: &[u8]) -> Result<Design, Error> {
    read_byte_pairs("EXP", bytes, true)
}

/// Janome JEF: the stitch data's offset is the header's first little-endian word.
fn read_jef(bytes: &[u8]) -> Result<Design, Error> {
    const NAME: &str = "JEF";
    let offset = bytes.get(0..4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).ok_or(Error::Truncated(NAME))?;
    let offset = usize::try_from(offset).map_err(|_| Error::Truncated(NAME))?;
    let data = bytes.get(offset..).ok_or(Error::Truncated(NAME))?;
    read_byte_pairs(NAME, data, true)
}

/// Brother PES: "#PES", a version, then the PEC block's offset; the PEC block's stitches start
/// 532 bytes in. Each coordinate is 7 bits (one byte) or, with the high bit set, 12 bits (two
/// bytes, 0x10 marking a jump and 0x20 a trim); 0xFE 0xB0 and a byte change colour; 0xFF ends.
fn read_pes(bytes: &[u8]) -> Result<Design, Error> {
    const NAME: &str = "PES";
    if !bytes.starts_with(b"#PES") {
        return Err(Error::Invalid(NAME, "no #PES signature".into()));
    }
    let pec = bytes.get(8..12).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).ok_or(Error::Truncated(NAME))?;
    let start = usize::try_from(pec).ok().and_then(|p| p.checked_add(532)).ok_or(Error::Truncated(NAME))?;
    let data = bytes.get(start..).ok_or(Error::Truncated(NAME))?;
    let mut b = Builder::new(false);
    let mut i = 0usize;
    // One coordinate: its value and whether it was a jump or trim.
    let coordinate = |i: &mut usize| -> Result<(i64, bool), Error> {
        let first = *data.get(*i).ok_or(Error::Truncated(NAME))?;
        *i += 1;
        if first & 0x80 == 0 {
            let v = i64::from(first & 0x7F);
            return Ok((if v & 0x40 != 0 { v - 0x80 } else { v }, false));
        }
        let second = *data.get(*i).ok_or(Error::Truncated(NAME))?;
        *i += 1;
        let v = (i64::from(first & 0x0F) << 8) | i64::from(second);
        let v = if v & 0x800 != 0 { v - 0x1000 } else { v };
        Ok((v, first & 0x30 != 0))
    };
    while let Some(&lead) = data.get(i) {
        if lead == 0xFF {
            break;
        }
        if lead == 0xFE && data.get(i + 1) == Some(&0xB0) {
            i += 3;
            b.apply(0, 0, Move::ColourChange)?;
            continue;
        }
        let (dx, jump_x) = coordinate(&mut i)?;
        let (dy, jump_y) = coordinate(&mut i)?;
        b.apply(dx, dy, if jump_x || jump_y { Move::Jump } else { Move::Stitch })?;
    }
    Ok(b.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DST record for a move of (dx, dy) in −121..=121, with the third byte's flags.
    fn dst_record(mut dx: i64, mut dy: i64, flags: u8) -> [u8; 3] {
        let mut b = [0u8, 0u8, 0x03 | flags];
        let mut put = |v: &mut i64, weight: i64, byte: usize, plus: u8, minus: u8| {
            // Balanced ternary: each weight is taken once, up or down, or not at all.
            if *v > weight / 2 {
                *v -= weight;
                b[byte] |= plus;
            } else if *v < -(weight / 2) {
                *v += weight;
                b[byte] |= minus;
            }
        };
        for (weight, byte, xp, xm, yp, ym) in [
            (81, 2, 0x04, 0x08, 0x20, 0x10),
            (27, 1, 0x04, 0x08, 0x20, 0x10),
            (9, 0, 0x04, 0x08, 0x20, 0x10),
            (3, 1, 0x01, 0x02, 0x80, 0x40),
            (1, 0, 0x01, 0x02, 0x80, 0x40),
        ] {
            put(&mut dx, weight, byte, xp, xm);
            put(&mut dy, weight, byte, yp, ym);
        }
        assert_eq!((dx, dy), (0, 0), "encodable move");
        b
    }

    fn dst(records: &[[u8; 3]]) -> Vec<u8> {
        let mut out = vec![b' '; 512];
        out[..3].copy_from_slice(b"LA:");
        for r in records {
            out.extend_from_slice(r);
        }
        out.extend_from_slice(&[0, 0, 0xF3]);
        out
    }

    #[test]
    fn dst_moves_stitches_jumps_and_colours() {
        let bytes = dst(&[
            dst_record(100, 0, 0),
            dst_record(0, 50, 0),
            dst_record(-121, 121, 0x80),
            dst_record(10, 0, 0),
            dst_record(0, 0, 0xC0),
            dst_record(5, 0, 0),
            dst_record(5, 0, 0),
        ]);
        let d = read(Format::Dst, &bytes).unwrap();
        assert_eq!(d.colours, 2);
        assert_eq!(d.runs.len(), 3, "a jump and a colour change split the runs");
        let first = &d.runs[0].points;
        assert_eq!(first[0], Point::new(0.0, 0.0));
        assert_eq!(first[1], Point::new(10.0, 0.0));
        assert_eq!(first[2], Point::new(10.0, -5.0), "y up in the file, down in the design");
        assert_eq!(d.runs[2].colour, 1);
    }

    #[test]
    fn byte_pair_formats_read_moves_and_commands() {
        // EXP: two stitches, a jump, two more, a colour change, one more.
        let exp = [10, 0, 0, 10, 0x80, 0x04, 20, 0, 5, 0, 5, 0, 0x80, 0x01, 0, 0, 3, 3];
        let d = read(Format::Exp, &exp).unwrap();
        assert_eq!(d.runs.len(), 3);
        assert_eq!(d.colours, 2);
        assert_eq!(d.runs[0].points.last(), Some(&Point::new(1.0, -1.0)));
        // JEF: the same records after a 4-byte offset.
        let mut jef = 4u32.to_le_bytes().to_vec();
        jef.extend_from_slice(&exp);
        assert_eq!(read(Format::Jef, &jef).unwrap(), d);
        assert!(matches!(read(Format::Exp, &[0x80, 0x33]), Err(Error::Invalid(..))));
        assert!(matches!(read(Format::Exp, &[0x80, 0x04, 1]), Err(Error::Truncated(_))));
    }

    #[test]
    fn pes_reads_its_pec_stitches() {
        let mut pes = b"#PES0001".to_vec();
        pes.extend_from_slice(&12u32.to_le_bytes());
        pes.resize(12 + 532, 0);
        // A 7-bit stitch (+10, −5), a 12-bit stitch (+300, 0), a colour change, a 12-bit jump,
        // a stitch, the end.
        pes.extend_from_slice(&[10, 0x7B]);
        pes.extend_from_slice(&[0x81, 0x2C, 0x00]);
        pes.extend_from_slice(&[0xFE, 0xB0, 0x01]);
        pes.extend_from_slice(&[0x90, 0x10, 0x00, 4, 4]);
        pes.extend_from_slice(&[0xFF, 0x00]);
        let d = read(Format::Pes, &pes).unwrap();
        assert_eq!(d.runs.len(), 2, "{d:?}");
        assert_eq!(d.runs[0].points, [Point::new(0.0, 0.0), Point::new(1.0, -0.5), Point::new(31.0, -0.5)]);
        assert_eq!(d.runs[1].colour, 1);
        assert_eq!(Format::detect("x.bin", &pes), Some(Format::Pes));
    }

    #[test]
    fn hostile_files_are_errors_not_crashes() {
        assert_eq!(read(Format::Dst, &[0; 100]), Err(Error::Truncated("DST")));
        assert!(read(Format::Jef, &u32::MAX.to_le_bytes()).is_err());
        let mut pes = b"#PES0001".to_vec();
        pes.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(read(Format::Pes, &pes).is_err());
        // Runaway moves stop at the extent limit.
        let far: Vec<u8> = std::iter::repeat_n([127u8, 0], 10_000).flatten().collect();
        assert_eq!(read(Format::Exp, &far), Err(Error::TooFar));
        assert_eq!(Format::detect("A.DST", b""), Some(Format::Dst));
        assert_eq!(Format::detect("a.svg", b""), None);
    }
}
