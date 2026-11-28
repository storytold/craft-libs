use super::{Result, State, invalid};

fn take<'a>(input: &mut &'a [u8], size: usize) -> Result<&'a [u8]> {
    let (head, tail) = input.split_at_checked(size).ok_or_else(|| invalid("truncated XZ envelope"))?;
    *input = tail;
    Ok(head)
}

fn byte(input: &mut &[u8]) -> Result<u8> {
    let Some(&[byte]) = take(input, 1)?.as_array() else { return Err(invalid("truncated XZ byte")) };
    Ok(byte)
}

fn vli(input: &mut &[u8]) -> Result<u64> {
    let mut value = 0;
    for shift in (0..63).step_by(7) {
        let encoded = byte(input)?;
        value |= u64::from(encoded & 127) << shift;
        if encoded < 128 {
            if shift != 0 && encoded == 0 {
                return Err(invalid("noncanonical XZ variable-length integer"));
            }
            return Ok(value);
        }
    }
    Err(invalid("XZ variable-length integer is too long"))
}

fn pair(hash: &mut blake3::Hasher, unpadded: u64, uncompressed: u64) {
    hash.update(&unpadded.to_le_bytes());
    hash.update(&uncompressed.to_le_bytes());
}

/// Read the already-decoded block's LZMA2 chunk framing. Each decoded chunk
/// produced its declared output length. XZ's BCJ and Delta filters preserve that length.
fn block_record(input: &mut &[u8], header_size: usize, check_size: usize, ctl: &State<'_>) -> Result<(u64, u64)> {
    let header = take(input, header_size)?;
    let flags = header.get(1).ok_or_else(|| invalid("truncated XZ Block Header"))?;
    let mut sizes = header.get(2..header_size.saturating_sub(4)).ok_or_else(|| invalid("truncated XZ Block Header"))?;
    let declared_compressed = (flags & 0x40 != 0).then(|| vli(&mut sizes)).transpose()?;
    let declared_uncompressed = (flags & 0x80 != 0).then(|| vli(&mut sizes)).transpose()?;
    for _ in 0..=(flags & 0x03) {
        vli(&mut sizes)?; // Filter ID.
        let size = usize::try_from(vli(&mut sizes)?).map_err(|_| invalid("XZ Filter Properties size overflows"))?;
        take(&mut sizes, size)?;
    }
    let start = input.len();
    let mut uncompressed = 0u64;
    loop {
        ctl.check()?;
        let control = byte(input)?;
        let (encoded, decoded) = match control {
            0 => break,
            1 | 2 => {
                let Some(&length) = take(input, 2)?.as_array() else { return Err(invalid("truncated XZ LZMA2 chunk")) };
                let size = usize::from(u16::from_be_bytes(length)) + 1;
                (size, size)
            }
            0x80..=0xff => {
                let Some(&[high, low, packed_high, packed_low]) = take(input, 4)?.as_array() else { return Err(invalid("truncated XZ LZMA2 chunk")) };
                let decoded = ((usize::from(control & 31) << 16) | usize::from(u16::from_be_bytes([high, low]))) + 1;
                if control >= 0xc0 {
                    take(input, 1)?; // New LZMA properties.
                }
                (usize::from(u16::from_be_bytes([packed_high, packed_low])) + 1, decoded)
            }
            _ => return Err(invalid("invalid XZ LZMA2 chunk control")),
        };
        take(input, encoded)?;
        uncompressed = uncompressed.checked_add(decoded as u64).ok_or_else(|| invalid("XZ block output size overflow"))?;
    }
    let data_size = start - input.len();
    if declared_compressed.is_some_and(|declared| declared != data_size as u64) {
        return Err(invalid("XZ Block Header compressed size does not match its data"));
    }
    if declared_uncompressed.is_some_and(|declared| declared != uncompressed) {
        return Err(invalid("XZ Block Header uncompressed size does not match its data"));
    }
    let padding = (4 - data_size % 4) % 4;
    take(input, padding + check_size)?;
    Ok((header_size as u64 + data_size as u64 + check_size as u64, uncompressed))
}

/// Verify the Index against every decoded Block and the footer's Backward Size.
/// XzReader already checks the stream magic, checksums and record count. Using
/// the size-pair hash described in XZ §4.3 keeps this pass allocation-free and
/// retains support for the blocking decoder's full filter chains.
pub(super) fn validate_envelope(mut bytes: &[u8], inflated: usize, ctl: &State<'_>) -> Result<()> {
    if !bytes.len().is_multiple_of(4) {
        return Err(invalid("XZ stream padding must be a multiple of four bytes"));
    }
    let mut total_output = 0u64;
    while !bytes.is_empty() {
        ctl.check()?;
        let header = take(&mut bytes, 12)?;
        let check_size = match header.get(7) {
            Some(0) => 0,
            Some(1) => 4,
            Some(4) => 8,
            Some(10) => 32,
            _ => return Err(invalid("unsupported XZ Check type")),
        };
        let mut actual = blake3::Hasher::new();
        let mut blocks = 0u64;
        while let Some(&size) = bytes.first().filter(|&&size| size != 0) {
            let (unpadded, output) = block_record(&mut bytes, (usize::from(size) + 1) * 4, check_size, ctl)?;
            pair(&mut actual, unpadded, output);
            total_output = total_output.checked_add(output).ok_or_else(|| invalid("XZ stream output size overflow"))?;
            blocks = blocks.checked_add(1).ok_or_else(|| invalid("XZ block count overflow"))?;
        }
        let index_start = bytes.len();
        if byte(&mut bytes)? != 0 || vli(&mut bytes)? != blocks {
            return Err(invalid("XZ Index record count does not match the decoded blocks"));
        }
        let mut indexed = blake3::Hasher::new();
        for _ in 0..blocks {
            ctl.check()?;
            pair(&mut indexed, vli(&mut bytes)?, vli(&mut bytes)?);
        }
        if indexed.finalize() != actual.finalize() {
            return Err(invalid("XZ Index sizes do not match the decoded blocks"));
        }
        let index_padding = (4 - (index_start - bytes.len()) % 4) % 4;
        if take(&mut bytes, index_padding)?.iter().any(|&byte| byte != 0) {
            return Err(invalid("nonzero XZ Index padding"));
        }
        take(&mut bytes, 4)?; // Index CRC32, already checked by XzReader.
        let index_size = index_start - bytes.len();
        let footer = take(&mut bytes, 12)?;
        let Some(&backward_size) = footer.get(4..8).and_then(<[u8]>::as_array) else { return Err(invalid("truncated XZ footer")) };
        if (u64::from(u32::from_le_bytes(backward_size)) + 1) * 4 != index_size as u64 {
            return Err(invalid("XZ stream Index does not match the footer Backward Size"));
        }
        while bytes.first() == Some(&0) {
            ctl.check()?;
            if take(&mut bytes, 4)? != [0; 4] {
                return Err(invalid("invalid XZ stream padding"));
            }
        }
    }
    if total_output != inflated as u64 {
        return Err(invalid("XZ Index uncompressed size does not match the decoded stream"));
    }
    Ok(())
}
