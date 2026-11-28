//! Allocation preflight for the compact-Thrift envelopes. Apache owns all actual Parquet decoding.
//! Checks lengths before its metadata vectors/page buffers are allocated; no payload values are read here.
use crate::{Result, TableLimits, invalid};
use parquet::file::metadata::ParquetMetaData;

pub(crate) const MAX_DECODED: usize = 512 << 20;
const MAX_PAGE: usize = 64 << 20;
const MAX_FOOTER: usize = 16 << 20;
const DATA_PAGE: i64 = 0;
const DICTIONARY_PAGE: i64 = 2;
const DATA_PAGE_V2: i64 = 3;
#[derive(Clone, Copy)]
enum Context {
    Footer,
    SchemaRoot,
    Schema,
    Page,
    Counts,
    Other,
}
struct Compact<'a> {
    bytes: &'a [u8],
    limits: &'a TableLimits,
    pos: usize,
    uncompressed: Option<usize>,
    compressed: Option<usize>,
    page_type: Option<i64>,
    /// The PageHeader field whose struct `Context::Counts` is reading.
    counts_field: i64,
    encoding: Option<i64>,
}
impl<'a> Compact<'a> {
    fn new(bytes: &'a [u8], limits: &'a TableLimits) -> Self {
        Self { bytes, limits, pos: 0, uncompressed: None, compressed: None, page_type: None, counts_field: 0, encoding: None }
    }
    fn byte(&mut self) -> Result<u8> {
        let b = self.bytes.get(self.pos).copied().ok_or_else(|| invalid("Truncated Parquet metadata"))?;
        self.pos += 1;
        Ok(b)
    }
    fn take(&mut self, n: usize) -> Result<()> {
        self.pos = self.pos.checked_add(n).filter(|end| *end <= self.bytes.len()).ok_or_else(|| invalid("Invalid Parquet metadata length"))?;
        Ok(())
    }
    fn varint(&mut self) -> Result<u64> {
        let mut n = 0u64;
        for shift in (0..70).step_by(7) {
            let b = self.byte()?;
            if shift == 63 && b > 1 {
                break;
            }
            n |= u64::from(b & 127) << shift;
            if b & 128 == 0 {
                return Ok(n);
            }
        }
        Err(invalid("Invalid Parquet metadata integer"))
    }
    fn integer(&mut self) -> Result<i64> {
        let n = self.varint()?;
        Ok((n >> 1) as i64 ^ -((n & 1) as i64))
    }
    fn length(&mut self) -> Result<usize> {
        usize::try_from(self.varint()?).map_err(|_| invalid("Parquet length overflow"))
    }
    fn structure(&mut self, depth: u8, context: Context) -> Result<()> {
        if depth > 32 {
            return Err(invalid("Parquet metadata nesting exceeds 32 levels"));
        }
        let mut id = 0i64;
        loop {
            let header = self.byte()?;
            if header == 0 {
                return Ok(());
            }
            let delta = header >> 4;
            id = if delta == 0 { self.integer()? } else { id.checked_add(i64::from(delta)).ok_or_else(|| invalid("Parquet field id overflow"))? };
            let kind = header & 15;
            // Apache's reader picks a known field by its id and reads the declared type whatever
            // the wire type says, so a field sent with another type would shift its reading of
            // every later byte away from this one.
            if expected_kind(context, self.counts_field, id).is_some_and(|expected| !expected.contains(&kind)) {
                return Err(invalid("A Parquet metadata field has the wrong type"));
            }
            if matches!(context, Context::Footer) && id == 2 && kind == 9 {
                let (n, element) = self.collection()?;
                if element != 12 || n == 0 || n > (self.limits.max_columns as usize).saturating_add(1) {
                    return Err(invalid("Invalid or oversized Parquet schema"));
                }
                for i in 0..n {
                    self.structure(depth + 1, if i == 0 { Context::SchemaRoot } else { Context::Schema })?;
                }
            } else if matches!(kind, 4..=6) {
                let value = self.integer()?;
                if matches!(context, Context::SchemaRoot) && id == 5 && !(1..=i64::from(self.limits.max_columns)).contains(&value) {
                    return Err(invalid("Invalid Parquet schema field count"));
                }
                if matches!(context, Context::Schema) && id == 5 && value != 0 {
                    return Err(invalid("Nested Parquet fields are not supported; flatten the table first"));
                }
                if matches!(context, Context::Page) && id == 1 {
                    self.page_type = Some(value);
                }
                if matches!(context, Context::Page) && matches!(id, 2 | 3) {
                    let size = usize::try_from(value).map_err(|_| invalid("Negative Parquet page size"))?;
                    if size > MAX_PAGE {
                        return Err(invalid("Parquet page exceeds the 64 MiB decoded-page budget"));
                    }
                    if id == 2 {
                        self.uncompressed = Some(size);
                    } else {
                        self.compressed = Some(size);
                    }
                }
                if matches!(context, Context::Counts) {
                    if value < 0 || value > MAX_PAGE as i64 {
                        return Err(invalid("Invalid Parquet page count or level length"));
                    }
                    if id == 1 && value > i64::from(self.limits.max_rows) + 1 {
                        return Err(invalid("Invalid Parquet page value count"));
                    }
                    // DataPageHeader field 2 and DataPageHeaderV2 field 4 hold the value encoding.
                    if (self.counts_field, id) == (5, 2) || (self.counts_field, id) == (8, 4) {
                        self.encoding = Some(value);
                    }
                }
            } else if matches!(context, Context::Page) && kind == 12 && matches!(id, 5 | 7 | 8) {
                self.counts_field = id;
                self.structure(depth + 1, Context::Counts)?;
            } else {
                self.skip(kind, depth + 1)?;
            }
        }
    }
    fn collection(&mut self) -> Result<(usize, u8)> {
        let header = self.byte()?;
        let n = if header >> 4 == 15 { self.length()? } else { usize::from(header >> 4) };
        if n > self.bytes.len().saturating_sub(self.pos) {
            return Err(invalid("Parquet collection length exceeds its metadata"));
        }
        Ok((n, header & 15))
    }
    fn skip(&mut self, kind: u8, depth: u8) -> Result<()> {
        if depth > 32 {
            return Err(invalid("Parquet metadata nesting exceeds 32 levels"));
        }
        match kind {
            1 | 2 => {}
            3 => self.take(1)?,
            4..=6 => {
                self.varint()?;
            }
            7 => self.take(8)?,
            8 => {
                let n = self.length()?;
                self.take(n)?;
            }
            9 | 10 => {
                let (n, element) = self.collection()?;
                for _ in 0..n {
                    if matches!(element, 1 | 2) {
                        self.take(1)?;
                    } else {
                        self.skip(element, depth + 1)?;
                    }
                }
            }
            11 => {
                let n = self.length()?;
                if n > self.bytes.len().saturating_sub(self.pos) / 2 {
                    return Err(invalid("Oversized Parquet metadata map"));
                }
                if n != 0 {
                    let types = self.byte()?;
                    for _ in 0..n {
                        for t in [types >> 4, types & 15] {
                            if matches!(t, 1 | 2) {
                                self.take(1)?;
                            } else {
                                self.skip(t, depth + 1)?;
                            }
                        }
                    }
                }
            }
            12 => self.structure(depth + 1, Context::Other)?,
            _ => return Err(invalid("Invalid compact-Thrift type in Parquet metadata")),
        }
        Ok(())
    }
}
/// The compact-Thrift types of the fields this preflight reads, and of every field Apache reads
/// from a page header before its payload: i32 and enums are 5, structs 12, lists 9, Booleans 1
/// or 2. `counts_field` names the page-header struct that `Context::Counts` is reading.
fn expected_kind(context: Context, counts_field: i64, id: i64) -> Option<&'static [u8]> {
    const I32: &[u8] = &[5];
    const BOOL: &[u8] = &[1, 2];
    match (context, counts_field, id) {
        (Context::Footer, _, 2) => Some(&[9]),
        (Context::SchemaRoot | Context::Schema, _, 5) => Some(I32),
        (Context::Page, _, 1..=4) => Some(I32),
        (Context::Page, _, 5..=8) => Some(&[12]),
        (Context::Counts, 5, 1..=4) | (Context::Counts, 7, 1 | 2) | (Context::Counts, 8, 1..=6) => Some(I32),
        (Context::Counts, 7, 3) | (Context::Counts, 8, 7) => Some(BOOL),
        _ => None,
    }
}
pub(crate) fn footer(bytes: &[u8], limits: &TableLimits) -> Result<usize> {
    if bytes.starts_with(b"PARE") {
        return Err(invalid("Encrypted Parquet files are not supported"));
    }
    if bytes.len() < 12 || !bytes.starts_with(b"PAR1") || !bytes.ends_with(b"PAR1") {
        return Err(invalid("Not a complete Parquet file (PAR1 header/footer required)"));
    }
    let tail = bytes.len() - 8;
    let length = bytes
        .get(tail..tail + 4)
        .and_then(|s| <[u8; 4]>::try_from(s).ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| invalid("Missing Parquet footer length"))? as usize;
    if length > MAX_FOOTER {
        return Err(invalid("Parquet footer exceeds the 16 MiB metadata budget"));
    }
    let start = tail.checked_sub(length).filter(|start| *start >= 4).ok_or_else(|| invalid("Invalid Parquet footer length"))?;
    let mut compact = Compact::new(bytes.get(start..tail).ok_or_else(|| invalid("Invalid Parquet footer"))?, limits);
    compact.structure(0, Context::Footer)?;
    if compact.pos != length {
        return Err(invalid("Unexpected bytes after Parquet metadata"));
    }
    Ok(start)
}
pub(crate) fn pages(bytes: &[u8], metadata: &ParquetMetaData, footer_start: usize, limits: &TableLimits) -> Result<()> {
    let mut decoded = 0usize;
    let mut pages = 0usize;
    for group in metadata.row_groups() {
        if group.num_rows() < 0 || group.num_rows() > metadata.file_metadata().num_rows() {
            return Err(invalid("Invalid Parquet row-group row count"));
        }
        for column in group.columns() {
            if column.file_path().is_some() {
                return Err(invalid("External Parquet column files are not supported"));
            }
            if column.num_values() < 0 || column.num_values() > group.num_rows() {
                return Err(invalid("Invalid flat Parquet column value count"));
            }
            let start = usize::try_from(column.dictionary_page_offset().unwrap_or(column.data_page_offset()))
                .map_err(|_| invalid("Invalid Parquet column offset"))?;
            let length = usize::try_from(column.compressed_size()).map_err(|_| invalid("Invalid Parquet column length"))?;
            // A writer records offset 0 for a chunk with no pages, such as one in an empty row
            // group. The chunk has no bytes to check.
            if length == 0 && column.num_values() == 0 {
                continue;
            }
            let end = start
                .checked_add(length)
                .filter(|end| *end <= footer_start && start >= 4)
                .ok_or_else(|| invalid("Parquet column lies outside the file data"))?;
            let mut cursor = start;
            let mut dictionary = false;
            while cursor < end {
                pages += 1;
                if pages > 1_000_000 {
                    return Err(invalid("Parquet file exceeds the 1,000,000-page budget"));
                }
                let mut header = Compact::new(bytes.get(cursor..end).ok_or_else(|| invalid("Truncated Parquet column"))?, limits);
                header.structure(0, Context::Page)?;
                let size = header.compressed.ok_or_else(|| invalid("Missing Parquet compressed-page size"))?;
                let uncompressed = header.uncompressed.ok_or_else(|| invalid("Missing Parquet decoded-page size"))?;
                // The decoder panics on a dictionary-encoded data page (PLAIN_DICTIONARY or
                // RLE_DICTIONARY) that no dictionary page precedes.
                match header.page_type {
                    Some(DICTIONARY_PAGE) => dictionary = true,
                    Some(DATA_PAGE | DATA_PAGE_V2) if matches!(header.encoding, Some(2 | 8)) && !dictionary => {
                        return Err(invalid("A dictionary-encoded Parquet page has no dictionary page before it"));
                    }
                    _ => {}
                }
                decoded = decoded
                    .checked_add(uncompressed)
                    .filter(|n| *n <= MAX_DECODED)
                    .ok_or_else(|| invalid("Parquet data exceeds the 512 MiB decoded-data budget; select a smaller table first"))?;
                cursor = cursor
                    .checked_add(header.pos)
                    .and_then(|n| n.checked_add(size))
                    .filter(|n| *n <= end)
                    .ok_or_else(|| invalid("Parquet page lies outside its column"))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    const LIMITS: TableLimits = crate::tests::LIMITS;
    #[test]
    fn oversized_lengths_and_deep_envelopes_fail_before_allocations() {
        // A list claiming more elements than its entire enclosing buffer.
        assert!(Compact::new(&[0x19, 0xfc, 0xff, 0xff, 0xff, 0x7f], &LIMITS).structure(0, Context::Other).is_err());
        let mut nested = vec![0x1c; 40];
        nested.extend([0; 41]);
        assert!(Compact::new(&nested, &LIMITS).structure(0, Context::Other).is_err());
        // PageHeader field 2 (uncompressed_page_size), zigzag 128 MiB.
        let header = [0x25, 0x80, 0x80, 0x80, 0x80, 0x01, 0];
        assert!(Compact::new(&header, &LIMITS).structure(0, Context::Page).unwrap_err().to_string().contains("64 MiB"));
        // Negative decoded-page size and a data-page value count of -1.
        assert!(Compact::new(&[0x25, 1, 0], &LIMITS).structure(0, Context::Page).is_err());
        assert!(Compact::new(&[0x15, 1, 0], &LIMITS).structure(0, Context::Counts).is_err());
    }
    #[test]
    fn known_page_header_fields_must_carry_their_declared_type() {
        // Page type, sizes and a data page header with num_values 3 and PLAIN_DICTIONARY encoding.
        let header =
            |num_values_type: u8| [0x15, 0x00, 0x15, 0x40, 0x15, 0x40, 0x2c, num_values_type, 0x06, 0x15, 0x04, 0x15, 0x00, 0x15, 0x00, 0, 0];
        let plain = header(0x15);
        let mut page = Compact::new(&plain, &LIMITS);
        page.structure(0, Context::Page).unwrap();
        assert_eq!((page.page_type, page.encoding), (Some(0), Some(2)));
        // As a Boolean, num_values would hide the encoding here while Apache still reads it.
        let message = Compact::new(&header(0x11), &LIMITS).structure(0, Context::Page).unwrap_err().to_string();
        assert!(message.contains("wrong type"), "{message}");
        // An uncompressed_page_size sent as a binary of 0x40 bytes.
        assert!(Compact::new(&[0x15, 0x00, 0x18, 0x40], &LIMITS).structure(0, Context::Page).unwrap_err().to_string().contains("wrong type"));
    }
    #[test]
    fn impossible_footer_and_encryption_are_rejected() {
        assert!(footer(b"PAR1\xff\xff\xff\x7fPAR1", &LIMITS).unwrap_err().to_string().contains("metadata budget"));
        assert!(footer(b"PARE12345678", &LIMITS).unwrap_err().to_string().contains("Encrypted"));
        assert!(footer(b"PAR1\x05\0\0\0PAR1", &LIMITS).is_err());
    }
}
