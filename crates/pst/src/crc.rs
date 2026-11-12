//! The CRC of [MS-PST] 5.3: the reflected CRC-32 polynomial (its table starts `0x00000000,
//! 0x77073096, 0xEE0E612C`), seeded with zero and with no final inversion. It guards the header,
//! every page and every block, and is what the file keeps of its password.

const TABLE: [u32; 256] = table();

#[allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)] // a constant, every index below 256
const fn table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

/// The CRC of `data`.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &b in data {
        let at = ((crc ^ u32::from(b)) & 0xFF) as usize;
        crc = TABLE.get(at).copied().unwrap_or(0) ^ (crc >> 8);
    }
    crc
}

/// The first entries of the table as [MS-PST] 5.3 prints them (a test checks the rest of the
/// algorithm against the spec's sample header).
#[cfg(test)]
pub(crate) fn table_head() -> [u32; 4] {
    [TABLE[0], TABLE[1], TABLE[2], TABLE[255]]
}
