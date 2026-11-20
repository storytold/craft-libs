//! Allocation-free access to the selected composite channels, one encoded row at a time.

use super::{Merged, take};
use crate::{PsdError, Result};

/// Validated locations of the first requested composite channels.
///
/// This view allocates no plane or row index. Unselected channel payloads need not be present.
pub struct EncodedRows<'a> {
    table: &'a [u8],
    payload: &'a [u8],
    entry_bytes: usize,
    row_bytes: usize,
    height: usize,
    used: usize,
}

/// A sequential cursor over one selected channel, without a decoded plane allocation.
pub struct ChannelRows<'a> {
    table: &'a [u8],
    payload: &'a [u8],
    entry_bytes: usize,
    row_bytes: usize,
    remaining: usize,
}

/// A borrowed raw or PackBits row with its declared decoded length.
pub struct EncodedRow<'a> {
    bytes: &'a [u8],
    row_bytes: usize,
    rle: bool,
}

fn row_length(entry: &[u8]) -> Result<usize> {
    match *entry {
        [a, b] => Ok(usize::from(u16::from_be_bytes([a, b]))),
        [a, b, c, d] => usize::try_from(u32::from_be_bytes([a, b, c, d])).map_err(|_| PsdError::LimitExceeded("RLE row size")),
        _ => Err(PsdError::invalid("RLE row table entry")),
    }
}

impl<'a> Merged<'a> {
    /// Borrow the first `used` raw or PackBits channels for row-at-a-time decoding.
    ///
    /// Checks the full declared row table and the selected payload extents before returning,
    /// without allocating. Eight-, sixteen- and thirty-two-bit samples remain big-endian.
    /// `max_row_bytes` limits one decoded row. RLE packet contents are checked on decoding;
    /// a short or malformed row is an error, not zero-padded recovery. Existing
    /// [`read_planes`](Self::read_planes) keeps its tolerant recovery behaviour.
    pub fn encoded_rows(&self, used: usize, max_row_bytes: usize) -> Result<EncodedRows<'a>> {
        let h = &self.head.header;
        let channels = usize::from(h.channels);
        if used > channels {
            return Err(PsdError::invalid("too few channels"));
        }
        let bpc = match h.depth {
            8 => 1usize,
            16 => 2,
            32 => 4,
            _ => return Err(PsdError::Unsupported("merged depth".into())),
        };
        let width = usize::try_from(h.width).map_err(|_| PsdError::LimitExceeded("row size"))?;
        let height = usize::try_from(h.height).map_err(|_| PsdError::LimitExceeded("row count"))?;
        let row_bytes = width.checked_mul(bpc).ok_or(PsdError::LimitExceeded("row size"))?;
        if row_bytes > max_row_bytes || row_bytes as u64 > crate::compression::MAX_DECODED_BYTES {
            return Err(PsdError::LimitExceeded("merged row size"));
        }
        let mut at = self.data_offset;
        let (table, entry_bytes, payload_bytes) = match self.compression {
            0 => {
                let len = used.checked_mul(height).and_then(|n| n.checked_mul(row_bytes)).ok_or(PsdError::LimitExceeded("merged data"))?;
                (&[][..], 0, len)
            }
            1 => {
                let entry = if h.version.is_psb() { 4 } else { 2 };
                // Even when only the first channel is requested, every declared row-length
                // entry precedes the payload. Skipping only selected entries reads table bytes.
                let table_len = channels.checked_mul(height).and_then(|n| n.checked_mul(entry)).ok_or(PsdError::LimitExceeded("row table"))?;
                let full_table = take(self.bytes, &mut at, table_len)?;
                let selected_len = used.checked_mul(height).and_then(|n| n.checked_mul(entry)).ok_or(PsdError::LimitExceeded("row table"))?;
                let table = full_table.get(..selected_len).ok_or_else(|| PsdError::invalid("row table"))?;
                let minimum = row_bytes.div_ceil(128).checked_mul(2).ok_or(PsdError::LimitExceeded("row size"))?;
                let mut total = 0usize;
                for entry in table.chunks_exact(entry) {
                    let len = row_length(entry)?;
                    // Reject impossible expansion before callers allocate the final image.
                    if len < minimum {
                        return Err(PsdError::invalid("RLE row too short for its width"));
                    }
                    total = total.checked_add(len).ok_or(PsdError::LimitExceeded("merged data"))?;
                }
                (table, entry, total)
            }
            _ => return Err(PsdError::Unsupported("merged compression".into())),
        };
        let payload = take(self.bytes, &mut at, payload_bytes)?;
        Ok(EncodedRows { table, payload, entry_bytes, row_bytes, height, used })
    }
}

impl<'a> EncodedRows<'a> {
    /// Number of decoded bytes in each row.
    pub fn row_bytes(&self) -> usize {
        self.row_bytes
    }

    /// Cursor for a selected channel. RLE setup scans preceding row lengths, without allocating.
    pub fn channel(&self, channel: usize) -> Result<ChannelRows<'a>> {
        if channel >= self.used {
            return Err(PsdError::invalid("channel was not selected"));
        }
        let first = channel.checked_mul(self.height).ok_or(PsdError::LimitExceeded("row count"))?;
        let (table, offset) = if self.entry_bytes == 0 {
            (&[][..], first.checked_mul(self.row_bytes).ok_or(PsdError::LimitExceeded("channel offset"))?)
        } else {
            let start = first.checked_mul(self.entry_bytes).ok_or(PsdError::LimitExceeded("row table"))?;
            let before = self.table.get(..start).ok_or_else(|| PsdError::invalid("row table"))?;
            let mut offset = 0usize;
            for entry in before.chunks_exact(self.entry_bytes) {
                offset = offset.checked_add(row_length(entry)?).ok_or(PsdError::LimitExceeded("channel offset"))?;
            }
            (self.table.get(start..).ok_or_else(|| PsdError::invalid("row table"))?, offset)
        };
        let payload = self.payload.get(offset..).ok_or_else(|| PsdError::invalid("channel offset"))?;
        Ok(ChannelRows { table, payload, entry_bytes: self.entry_bytes, row_bytes: self.row_bytes, remaining: self.height })
    }
}

impl<'a> ChannelRows<'a> {
    /// Read the next encoded row, or `None` after the declared channel height.
    pub fn next_row(&mut self) -> Result<Option<EncodedRow<'a>>> {
        if self.remaining == 0 {
            return Ok(None);
        }
        let len = if self.entry_bytes == 0 {
            self.row_bytes
        } else {
            let entry = self.table.get(..self.entry_bytes).ok_or_else(|| PsdError::invalid("row table"))?;
            row_length(entry)?
        };
        let bytes = self.payload.get(..len).ok_or_else(|| PsdError::invalid("truncated row"))?;
        self.payload = self.payload.get(len..).ok_or_else(|| PsdError::invalid("truncated row"))?;
        if self.entry_bytes != 0 {
            self.table = self.table.get(self.entry_bytes..).ok_or_else(|| PsdError::invalid("row table"))?;
        }
        self.remaining -= 1;
        Ok(Some(EncodedRow { bytes, row_bytes: self.row_bytes, rle: self.entry_bytes != 0 }))
    }
}

impl EncodedRow<'_> {
    /// Decode into exactly one row of caller-owned storage, without allocating.
    ///
    /// Truncated PackBits packets and underfilled/overfilled rows are errors. Bytes after a
    /// complete row remain tolerated, matching [`crate::compression::packbits::decode_to_slice`].
    /// On error, the output may contain a partially decoded prefix.
    pub fn decode_to_slice(&self, output: &mut [u8]) -> Result<()> {
        if output.len() != self.row_bytes {
            return Err(PsdError::invalid("decoded row has wrong length"));
        }
        if self.rle {
            crate::compression::packbits::decode_to_slice(self.bytes, output)
        } else {
            output.copy_from_slice(self.bytes);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(psb: bool, depth: u16, channels: u16, height: u32, compression: u16) -> Vec<u8> {
        let mut b = b"8BPS".to_vec();
        b.extend_from_slice(&(if psb { 2u16 } else { 1 }).to_be_bytes());
        b.extend_from_slice(&[0; 6]);
        b.extend_from_slice(&channels.to_be_bytes());
        b.extend_from_slice(&height.to_be_bytes());
        b.extend_from_slice(&2u32.to_be_bytes());
        b.extend_from_slice(&depth.to_be_bytes());
        b.extend_from_slice(&1u16.to_be_bytes());
        b.extend_from_slice(&[0; 8]);
        b.extend(std::iter::repeat_n(0, if psb { 8 } else { 4 }));
        b.extend_from_slice(&compression.to_be_bytes());
        b
    }

    #[test]
    fn selected_rows_skip_full_table_and_allow_missing_spot_payload() {
        for psb in [false, true] {
            for depth in [8, 16, 32] {
                let row_bytes = 2 * usize::from(depth / 8);
                let mut b = file(psb, depth, 3, 2, 1);
                for _ in 0..6 {
                    if psb {
                        b.extend_from_slice(&2u32.to_be_bytes());
                    } else {
                        b.extend_from_slice(&2u16.to_be_bytes());
                    }
                }
                for value in 1..=4 {
                    b.extend_from_slice(&[(1i16 - row_bytes as i16) as u8, value]);
                }
                let merged = Merged::open(&b, 4).unwrap();
                let rows = merged.encoded_rows(2, row_bytes).unwrap();
                assert_eq!(rows.row_bytes(), row_bytes);
                assert!(rows.channel(2).is_err());
                for c in 0..2 {
                    let mut channel = rows.channel(c).unwrap();
                    for y in 0..2 {
                        let mut out = vec![0; row_bytes];
                        channel.next_row().unwrap().unwrap().decode_to_slice(&mut out).unwrap();
                        assert_eq!(out, vec![(c * 2 + y + 1) as u8; row_bytes]);
                    }
                    assert!(channel.next_row().unwrap().is_none());
                }
                assert!(merged.encoded_rows(3, row_bytes).is_err());
                assert!(merged.encoded_rows(2, row_bytes - 1).is_err());
                assert!(merged.encoded_rows(4, row_bytes).is_err());
            }
        }
    }

    #[test]
    fn raw_rows_are_borrowed_and_require_exact_output_length() {
        let mut b = file(false, 16, 2, 2, 0);
        b.extend(0..16);
        let merged = Merged::open(&b, 4).unwrap();
        let rows = merged.encoded_rows(2, 4).unwrap();
        let mut channel = rows.channel(1).unwrap();
        let row = channel.next_row().unwrap().unwrap();
        assert!(row.decode_to_slice(&mut [0; 3]).is_err());
        let mut out = [0; 4];
        row.decode_to_slice(&mut out).unwrap();
        assert_eq!(out, [8, 9, 10, 11]);
        channel.next_row().unwrap().unwrap().decode_to_slice(&mut out).unwrap();
        assert_eq!(out, [12, 13, 14, 15]);
        assert!(channel.next_row().unwrap().is_none());
        b.pop();
        assert!(Merged::open(&b, 4).unwrap().encoded_rows(2, 4).is_err());
    }

    #[test]
    fn strict_rows_reject_short_packets_but_keep_trailing_bytes_and_recovery() {
        for encoded in [&[3, 7, 8][..], &[0, 7, 128], &[255, 7, 255], &[252, 7]] {
            let mut b = file(false, 16, 1, 1, 1);
            b.extend_from_slice(&(encoded.len() as u16).to_be_bytes());
            b.extend_from_slice(encoded);
            let merged = Merged::open(&b, 2).unwrap();
            let rows = merged.encoded_rows(1, 4).unwrap();
            let row = rows.channel(0).unwrap().next_row().unwrap().unwrap();
            assert!(row.decode_to_slice(&mut [0; 4]).is_err(), "{encoded:?}");
        }
        let mut b = file(false, 16, 1, 1, 1);
        b.extend_from_slice(&3u16.to_be_bytes());
        b.extend_from_slice(&[253, 7, 127]);
        let merged = Merged::open(&b, 2).unwrap();
        let rows = merged.encoded_rows(1, 4).unwrap();
        let mut out = [0; 4];
        rows.channel(0).unwrap().next_row().unwrap().unwrap().decode_to_slice(&mut out).unwrap();
        assert_eq!(out, [7; 4]);
        b.truncate(b.len() - 3);
        b.extend_from_slice(&[3, 7, 8]);
        let merged = Merged::open(&b, 2).unwrap();
        assert_eq!(merged.read_planes(1).unwrap(), vec![vec![7, 8, 0, 0]]);
    }

    #[test]
    fn rejects_incomplete_table_payload_and_impossible_expansion_before_output() {
        let mut b = file(false, 16, 2, 1, 1);
        b.extend_from_slice(&2u16.to_be_bytes());
        assert!(Merged::open(&b, 2).unwrap().encoded_rows(1, 4).is_err());
        b.extend_from_slice(&2u16.to_be_bytes());
        b.extend_from_slice(&[253]);
        assert!(Merged::open(&b, 2).unwrap().encoded_rows(1, 4).is_err());
        b.push(7);
        assert!(Merged::open(&b, 2).unwrap().encoded_rows(1, 4).is_ok());
        let mut b = file(false, 16, 1, 1, 1);
        b.extend_from_slice(&1u16.to_be_bytes());
        b.push(128);
        assert!(Merged::open(&b, 2).unwrap().encoded_rows(1, 4).is_err());
    }
}
