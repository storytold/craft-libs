//! Incremental assembly of a new PDF 1.7 file with a classic xref table.
//!
//! This low-level writer accepts **trusted, already serialized** object bodies and stream
//! dictionary entries. It is not a parser or sanitizer. Applications retain page trees,
//! resource dictionaries and drawing policy. Unlike `cos::write_full`, it does not retain
//! an object model or a second copy of stream bodies. Objects may arrive out of order;
//! every ID from 1 through the largest ID must be written exactly once before finishing.
use std::fmt;

/// Bound sparse object numbers before growing the offset table (one million entries).
/// Vector capacity may exceed its logical length because allocation grows geometrically.
pub const MAX_OBJECTS: u32 = 1_000_000;
const MAX_OFFSET: u64 = 9_999_999_999;

/// Invalid object numbering, classic-xref size limits, or allocation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Zero or an object number beyond the offset-table budget.
    ObjectNumber,
    /// An object number was already written.
    DuplicateObject,
    /// A referenced root/info object or an ID within the table was not written.
    MissingObject,
    /// A byte offset does not fit a classic ten-digit xref entry.
    OffsetLimit,
    /// The output or offset table could not be allocated.
    Allocation,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ObjectNumber => "PDF object number exceeds the classic-writer budget",
            Self::DuplicateObject => "PDF object number was written twice",
            Self::MissingObject => "PDF object table is incomplete",
            Self::OffsetLimit => "PDF output exceeds the classic-xref offset limit",
            Self::Allocation => "PDF output could not be allocated",
        })
    }
}
impl std::error::Error for Error {}

/// New-document framing; stores only the output bytes and one offset per object.
pub struct Writer {
    bytes: Vec<u8>,
    offsets: Vec<usize>,
}
impl Writer {
    /// Reserve an output-size hint; this is not a promise to allocate all future output.
    pub fn with_capacity(capacity: usize) -> Result<Self, Error> {
        let mut bytes = Vec::new();
        if capacity as u64 > MAX_OFFSET {
            return Err(Error::OffsetLimit);
        }
        bytes.try_reserve(capacity.max(15)).map_err(|_| Error::Allocation)?;
        bytes.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");
        Ok(Self { bytes, offsets: Vec::new() })
    }

    fn reserve(&mut self, extra: usize) -> Result<(), Error> {
        let size = self.bytes.len().checked_add(extra).ok_or(Error::OffsetLimit)?;
        if size as u64 > MAX_OFFSET {
            return Err(Error::OffsetLimit);
        }
        self.bytes.try_reserve(extra).map_err(|_| Error::Allocation)
    }

    // Complete every fallible step before publishing the object offset or any object bytes.
    fn begin(&mut self, id: u32, extra: usize) -> Result<String, Error> {
        if id == 0 || id > MAX_OBJECTS {
            return Err(Error::ObjectNumber);
        }
        let index = id as usize - 1;
        if self.offsets.get(index).is_some_and(|offset| *offset != 0) {
            return Err(Error::DuplicateObject);
        }
        let header = format!("{id} 0 obj\n");
        self.reserve(header.len().checked_add(extra).ok_or(Error::OffsetLimit)?)?;
        if self.offsets.len() <= index {
            self.offsets.try_reserve(index + 1 - self.offsets.len()).map_err(|_| Error::Allocation)?;
            self.offsets.resize(index + 1, 0);
        }
        self.offsets[index] = self.bytes.len();
        Ok(header)
    }

    /// Write one already serialized object body, without `obj`/`endobj` framing.
    pub fn object(&mut self, id: u32, body: &str) -> Result<(), Error> {
        let header = self.begin(id, body.len().checked_add(8).ok_or(Error::OffsetLimit)?)?;
        self.bytes.extend_from_slice(header.as_bytes());
        self.bytes.extend_from_slice(body.as_bytes());
        self.bytes.extend_from_slice(b"\nendobj\n");
        Ok(())
    }

    /// Write a stream. `entries` contains trusted dictionary entries, without delimiters
    /// or `/Length`; the writer calculates `/Length` from the exact binary byte slice.
    pub fn stream(&mut self, id: u32, entries: &str, data: &[u8]) -> Result<(), Error> {
        let separator = if entries.is_empty() { "" } else { " " };
        let dictionary = format!("<<{separator}{entries} /Length {} >>\nstream\n", data.len());
        let size = dictionary.len().checked_add(data.len()).and_then(|n| n.checked_add(18)).ok_or(Error::OffsetLimit)?;
        let header = self.begin(id, size)?;
        self.bytes.extend_from_slice(header.as_bytes());
        self.bytes.extend_from_slice(dictionary.as_bytes());
        self.bytes.extend_from_slice(data);
        self.bytes.extend_from_slice(b"\nendstream\nendobj\n");
        Ok(())
    }

    /// Finish with generation-zero root and optional info references. Missing IDs are
    /// refused, rather than accidentally writing in-use entries pointing to offset zero.
    pub fn finish(mut self, root: u32, info: Option<u32>) -> Result<Vec<u8>, Error> {
        let exists = |id: u32| id.checked_sub(1).and_then(|i| self.offsets.get(i as usize)).is_some_and(|o| *o != 0);
        if !exists(root) || info.is_some_and(|id| !exists(id)) || self.offsets.contains(&0) {
            return Err(Error::MissingObject);
        }
        let xref = self.bytes.len();
        let count = self.offsets.len() + 1;
        let header = format!("xref\n0 {count}\n0000000000 65535 f \n");
        let info = info.map(|id| format!(" /Info {id} 0 R")).unwrap_or_default();
        let tail = format!("trailer\n<< /Size {count} /Root {root} 0 R{info} >>\nstartxref\n{xref}\n%%EOF\n");
        let extra = self
            .offsets
            .len()
            .checked_mul(20)
            .and_then(|n| n.checked_add(header.len()))
            .and_then(|n| n.checked_add(tail.len()))
            .ok_or(Error::OffsetLimit)?;
        self.reserve(extra)?;
        self.bytes.extend_from_slice(header.as_bytes());
        for offset in self.offsets {
            self.bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        self.bytes.extend_from_slice(tail.as_bytes());
        Ok(self.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_order_objects_have_exact_offsets_and_binary_length() -> Result<(), Error> {
        let mut writer = Writer::with_capacity(0)?;
        writer.stream(3, "/Filter /Example", b"\0\xff\nendstream")?;
        writer.object(1, "<< /Type /Catalog /Pages 2 0 R >>")?;
        writer.object(2, "<< /Type /Pages /Kids [] /Count 0 >>")?;
        let bytes = writer.finish(1, Some(2))?;
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/Length 12 >>"));
        let xref = bytes.windows(5).position(|w| w == b"xref\n").ok_or(Error::MissingObject)?;
        let xref_text = std::str::from_utf8(&bytes[xref..]).map_err(|_| Error::MissingObject)?;
        let rows: Vec<_> = xref_text.lines().skip(3).take(3).collect();
        for (index, row) in rows.iter().enumerate() {
            let offset: usize = row[..10].parse().map_err(|_| Error::MissingObject)?;
            assert!(bytes[offset..].starts_with(format!("{} 0 obj\n", index + 1).as_bytes()));
        }
        assert!(text.ends_with(&format!("startxref\n{xref}\n%%EOF\n")));
        Ok(())
    }

    #[test]
    fn invalid_ids_duplicates_and_holes_are_refused() -> Result<(), Error> {
        let mut writer = Writer::with_capacity(0)?;
        assert_eq!(writer.object(0, "null"), Err(Error::ObjectNumber));
        assert_eq!(writer.object(u32::MAX, "null"), Err(Error::ObjectNumber));
        writer.object(2, "null")?;
        assert_eq!(writer.object(2, "null"), Err(Error::DuplicateObject));
        assert!(matches!(writer.finish(2, None), Err(Error::MissingObject)));
        let mut writer = Writer::with_capacity(0)?;
        writer.object(1, "null")?;
        assert!(matches!(writer.finish(1, Some(2)), Err(Error::MissingObject)));
        Ok(())
    }
    #[test]
    fn matches_legacy_grid_envelope_bytes() -> Result<(), Error> {
        let mut writer = Writer::with_capacity(0)?;
        writer.stream(2, "", b"A\0B")?;
        writer.object(1, "<< /Type /Catalog >>")?;
        writer.object(3, "<< /Title (X) >>")?;
        // Fixed bytes from the prior Grid envelope algorithm, not generated by this API.
        let expected = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n2 0 obj\n<< /Length 3 >>\nstream\nA\0B\nendstream\nendobj\n1 0 obj\n<< /Type /Catalog >>\nendobj\n3 0 obj\n<< /Title (X) >>\nendobj\nxref\n0 4\n0000000000 65535 f \n0000000067 00000 n \n0000000015 00000 n \n0000000103 00000 n \ntrailer\n<< /Size 4 /Root 1 0 R /Info 3 0 R >>\nstartxref\n135\n%%EOF\n";
        assert_eq!(writer.finish(1, Some(3))?, expected);
        Ok(())
    }

    #[test]
    fn rejected_object_does_not_change_output() -> Result<(), Error> {
        let mut writer = Writer::with_capacity(0)?;
        writer.object(1, "null")?;
        assert_eq!(writer.object(1, "123"), Err(Error::DuplicateObject));
        assert_eq!(writer.stream(u32::MAX, "", b"bad"), Err(Error::ObjectNumber));
        writer.stream(2, "", b"good")?;
        let mut clean = Writer::with_capacity(0)?;
        clean.object(1, "null")?;
        clean.stream(2, "", b"good")?;
        assert_eq!(writer.finish(1, None)?, clean.finish(1, None)?);
        Ok(())
    }
}
