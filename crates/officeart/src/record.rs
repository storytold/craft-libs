//! The record framing [MS-ODRAW] shares with [MS-PPT]: an 8-byte header (version, instance, type,
//! length) followed by `len` bytes, which for a container (version `0xF`) are more records.

/// The little-endian `u16` at `at`, if `b` holds it.
pub fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([*s.first()?, *s.get(1)?]))
}

/// The little-endian `u32` at `at`, if `b` holds it.
pub fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([*s.first()?, *s.get(1)?, *s.get(2)?, *s.get(3)?]))
}

/// One record over borrowed bytes.
#[derive(Clone, Copy, Debug)]
pub struct Rec<'a> {
    pub ver: u8,
    pub instance: u16,
    pub kind: u16,
    /// The record's body (after the header), clipped to the input.
    pub data: &'a [u8],
    /// Offset of the header in the slice it was read from.
    pub offset: usize,
}

impl<'a> Rec<'a> {
    /// The record whose header starts at `at`, its body clipped to `bytes` (a truncated last record
    /// keeps what is there).
    pub fn at(bytes: &'a [u8], at: usize) -> Option<Rec<'a>> {
        let vi = u16_at(bytes, at)?;
        let kind = u16_at(bytes, at.checked_add(2)?)?;
        let len = u32_at(bytes, at.checked_add(4)?)? as usize;
        let start = at.checked_add(8)?;
        let end = start.saturating_add(len).min(bytes.len());
        Some(Rec { ver: (vi & 0xF) as u8, instance: vi >> 4, kind, data: bytes.get(start..end)?, offset: at })
    }

    /// The bytes the record spans in what it was read from: its header and its (clipped) body.
    pub fn span(&self) -> usize {
        8 + self.data.len()
    }

    pub fn is_container(&self) -> bool {
        self.ver == 0xF
    }

    /// The records inside this one; none for an atom.
    pub fn children(&self) -> Records<'a> {
        Records::new(if self.is_container() { self.data } else { &[] })
    }

    /// The first child of type `kind`.
    pub fn child(&self, kind: u16) -> Option<Rec<'a>> {
        self.children().find(|r| r.kind == kind)
    }

    /// Every child of type `kind`.
    pub fn children_of(&self, kind: u16) -> impl Iterator<Item = Rec<'a>> {
        self.children().filter(move |r| r.kind == kind)
    }

    pub fn u8(&self, at: usize) -> Option<u8> {
        self.data.get(at).copied()
    }
    pub fn u16(&self, at: usize) -> Option<u16> {
        u16_at(self.data, at)
    }
    pub fn i16(&self, at: usize) -> Option<i16> {
        self.u16(at).map(|v| v as i16)
    }
    pub fn u32(&self, at: usize) -> Option<u32> {
        u32_at(self.data, at)
    }
    pub fn i32(&self, at: usize) -> Option<i32> {
        self.u32(at).map(|v| v as i32)
    }
}

/// Iterates the records laid end to end in a byte slice, whatever their version: a container's
/// body, or a run of records outside any container.
#[derive(Clone, Debug)]
pub struct Records<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Records<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Records { bytes, at: 0 }
    }
}

impl<'a> Iterator for Records<'a> {
    type Item = Rec<'a>;
    fn next(&mut self) -> Option<Rec<'a>> {
        let r = Rec::at(self.bytes, self.at)?;
        // Always advance by at least the header, so a zero-length record can't stall the walk.
        self.at = self.at.checked_add(r.span())?;
        Some(r)
    }
}

#[cfg(test)]
pub(crate) mod build {
    //! Writing records, for tests.

    pub fn atom(kind: u16, instance: u16, body: &[u8]) -> Vec<u8> {
        head(0, kind, instance, body)
    }

    pub fn container(kind: u16, instance: u16, children: &[Vec<u8>]) -> Vec<u8> {
        head(0xF, kind, instance, &children.concat())
    }

    fn head(ver: u16, kind: u16, instance: u16, body: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(body.len() + 8);
        v.extend_from_slice(&(ver | (instance << 4)).to_le_bytes());
        v.extend_from_slice(&kind.to_le_bytes());
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(body);
        v
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::build::*;
    use super::*;

    #[test]
    fn walks_nested_records() {
        let c = container(0x03E8, 0, &[atom(0x03E9, 0, &[1, 2, 3, 4]), atom(0x0FA0, 2, &[9; 6])]);
        let r = Rec::at(&c, 0).unwrap();
        assert!(r.is_container());
        let kids: Vec<_> = r.children().collect();
        assert_eq!(kids.len(), 2);
        assert_eq!(kids[1].instance, 2);
        assert_eq!(r.child(0x03E9).unwrap().u32(0), Some(0x0403_0201));
    }

    #[test]
    fn truncated_and_zero_length_records_end_the_walk() {
        let mut c = atom(0x1, 0, &[]).repeat(3);
        c.extend_from_slice(&[0xF, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0x7F, 1, 2]);
        let all: Vec<_> = Records::new(&c).collect();
        assert_eq!(all.len(), 4);
        assert_eq!(all[3].data, &[1, 2]);
        assert!(Records::new(&[1, 2, 3]).next().is_none());
    }
}
