//! Committed-prefix scanning for SQLite write-ahead logs.
//! Wire algorithm moved from LightCraft (Orthic Labs, 4a427894; Dylan M. Taylor, be760ee5).

fn be_word(bytes: &[u8], at: usize) -> Result<u32, &'static str> {
    let word = bytes.get(at..at + 4).and_then(|s| <[u8; 4]>::try_from(s).ok()).ok_or("truncated WAL")?;
    Ok(u32::from_be_bytes(word))
}

fn wal_checksum(bytes: &[u8], big: bool, mut sum: (u32, u32)) -> (u32, u32) {
    for pair in bytes.as_chunks::<8>().0 {
        let first = [pair[0], pair[1], pair[2], pair[3]];
        let second = [pair[4], pair[5], pair[6], pair[7]];
        let (first, second) =
            if big { (u32::from_be_bytes(first), u32::from_be_bytes(second)) } else { (u32::from_le_bytes(first), u32::from_le_bytes(second)) };
        sum.0 = sum.0.wrapping_add(first).wrapping_add(sum.1);
        sum.1 = sum.1.wrapping_add(second).wrapping_add(sum.0);
    }
    sum
}

// SQLite file format §4: validate the live checksum chain in constant space, then overlay
// only its last committed prefix. The forensic timeline materializes EVERY historical
// database snapshot; it must never be used to validate a large, actively used catalog.
/// Returns the byte length through the last checksum-valid commit, or zero if none.
/// An incomplete or uncommitted suffix is not applied; a malformed header errors.
/// The database header supplies the expected page size. Uses constant memory.
pub fn committed_prefix_len(main: &[u8], wal: &[u8]) -> Result<usize, &'static str> {
    if wal.is_empty() {
        return Ok(0);
    }
    let header = wal.get(..32).ok_or("truncated WAL header")?;
    let magic = be_word(header, 0)?;
    if !matches!(magic, 0x377f0682 | 0x377f0683) || be_word(header, 4)? != 3_007_000 {
        return Err("invalid WAL header");
    }
    let raw_size = main.get(16..18).and_then(|s| <[u8; 2]>::try_from(s).ok()).map(u16::from_be_bytes).ok_or("truncated SQLite header")?;
    let size = crate::page_size_from_raw(raw_size).map_err(|_| "WAL page size mismatch")?;
    if be_word(header, 8)? != size {
        return Err("WAL page size mismatch");
    }
    let big = magic == 0x377f0683;
    let mut sum = wal_checksum(&header[..24], big, (0, 0));
    if sum != (be_word(header, 24)?, be_word(header, 28)?) {
        return Err("invalid WAL header checksum");
    }
    let salt = (be_word(header, 16)?, be_word(header, 20)?);
    let mut committed = 0;
    let stride = size as usize + 24;
    for (i, frame) in wal[32..].chunks_exact(stride).enumerate() {
        if be_word(frame, 0)? == 0 || salt != (be_word(frame, 8)?, be_word(frame, 12)?) {
            break;
        }
        sum = wal_checksum(&frame[..8], big, sum);
        sum = wal_checksum(&frame[24..], big, sum);
        if sum != (be_word(frame, 16)?, be_word(frame, 20)?) {
            break;
        }
        if be_word(frame, 4)? != 0 {
            committed = 32 + (i + 1) * stride;
        }
    }
    Ok(committed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_be_bytes());
    }

    fn fixture(big: bool) -> ([u8; 18], [u8; 32 + 3 * 536]) {
        let mut main = [0u8; 18];
        main[16..18].copy_from_slice(&512u16.to_be_bytes());
        let mut wal = [0; 32 + 3 * 536];
        word(&mut wal, 0, if big { 0x377f0683 } else { 0x377f0682 });
        word(&mut wal, 4, 3_007_000);
        word(&mut wal, 8, 512);
        word(&mut wal, 16, 17);
        word(&mut wal, 20, 29);
        let mut sum = wal_checksum(&wal[..24], big, (0, 0));
        word(&mut wal, 24, sum.0);
        word(&mut wal, 28, sum.1);
        for (i, frame) in wal[32..].as_chunks_mut::<536>().0.iter_mut().enumerate() {
            word(frame, 0, i as u32 + 1);
            word(frame, 4, [1, 3, 0][i]);
            word(frame, 8, 17);
            word(frame, 12, 29);
            frame[24..].fill(i as u8 + 10);
            sum = wal_checksum(&frame[..8], big, sum);
            sum = wal_checksum(&frame[24..], big, sum);
            word(frame, 16, sum.0);
            word(frame, 20, sum.1);
        }
        (main, wal)
    }

    #[test]
    fn committed_prefix_stops_at_a_frame_checksum_mismatch() {
        for big in [false, true] {
            let (main, wal) = fixture(big);
            let mut second_checksum_wrong = wal;
            second_checksum_wrong[32 + 536 + 16] ^= 1;
            assert_eq!(committed_prefix_len(&main, &second_checksum_wrong), Ok(32 + 536));

            let mut first_checksum_wrong = wal;
            first_checksum_wrong[32 + 16] ^= 1;
            assert_eq!(committed_prefix_len(&main, &first_checksum_wrong), Ok(0));
        }
    }

    #[test]
    fn committed_prefix_stops_before_uncommitted_or_partial_frame() {
        for big in [false, true] {
            let (main, wal) = fixture(big);
            let committed = 32 + 2 * 536;
            assert_eq!(committed_prefix_len(&main, &wal), Ok(committed));
            assert_eq!(committed_prefix_len(&main, &wal[..committed + 23]), Ok(committed));
            assert_eq!(committed_prefix_len(&main, &wal[..32]), Ok(0));
        }
    }
}
