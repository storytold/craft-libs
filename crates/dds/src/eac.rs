//! ETC1 and EAC 4×4 block decoders.
//!
//! EAC's 11-bit channel blocks are reduced to display bytes after reconstruction and clamping.
//! The signed layouts map -1..1 to 0..255 so negative values remain visible; zero is 128.
//! Layout and modifier tables: <https://registry.khronos.org/DataFormat/specs/1.4/dataformat.1.4.html>.

// Positive halves of the Khronos EAC modifier tables. Negative entries are -positive - 1.
const MODIFIERS: [[i16; 4]; 16] = [
    [2, 5, 8, 14],
    [2, 6, 9, 12],
    [1, 4, 7, 12],
    [1, 3, 5, 12],
    [2, 5, 7, 11],
    [2, 6, 8, 10],
    [3, 6, 7, 10],
    [2, 4, 7, 10],
    [1, 5, 7, 9],
    [1, 4, 7, 9],
    [1, 3, 7, 9],
    [1, 4, 6, 9],
    [2, 3, 6, 9],
    [0, 1, 2, 9],
    [3, 5, 7, 8],
    [2, 4, 6, 8],
];

fn split_halves(block: &[u8; 16]) -> ([u8; 8], [u8; 8]) {
    let mut first = [0u8; 8];
    let mut second = [0u8; 8];
    for (i, x) in block.iter().enumerate() {
        if i < 8 {
            first[i] = *x;
        } else {
            second[i - 8] = *x;
        }
    }
    (first, second)
}

/// ETC1 uses big-endian blocks and column-major selector bits, independently of the split.
/// Differential bases outside RGB555 are undefined by the format and are refused.
pub(crate) fn etc1(block: &[u8; 8]) -> Option<[[u8; 4]; 16]> {
    const ETC1_MODIFIERS: [[i16; 2]; 8] = [[2, 8], [5, 17], [9, 29], [13, 42], [18, 60], [24, 80], [33, 106], [47, 183]];
    let bits = u64::from_be_bytes(*block);
    let differential = bits & (1 << 33) != 0;
    let horizontal = bits & (1 << 32) != 0;
    let mut bases = [[0i16; 3]; 2];
    for (channel, shift) in [56, 48, 40].into_iter().enumerate() {
        let value = ((bits >> shift) & 255) as i16;
        let (first, second) = if differential {
            let first = value >> 3;
            let delta = ((value & 7) ^ 4) - 4;
            let second = first + delta;
            if !(0..=31).contains(&second) {
                return None;
            }
            ((first << 3) | (first >> 2), (second << 3) | (second >> 2))
        } else {
            ((value >> 4) * 17, (value & 15) * 17)
        };
        bases[0][channel] = first;
        bases[1][channel] = second;
    }
    let first_table = ETC1_MODIFIERS.get(((bits >> 37) & 7) as usize)?;
    let second_table = ETC1_MODIFIERS.get(((bits >> 34) & 7) as usize)?;
    Some(std::array::from_fn(|pixel| {
        let (x, y) = (pixel % 4, pixel / 4);
        let second = if horizontal { y >= 2 } else { x >= 2 };
        let (base, table) = if second { (bases[1], second_table) } else { (bases[0], first_table) };
        let selector = x * 4 + y;
        let magnitude = if bits & (1 << selector) == 0 { table[0] } else { table[1] };
        let delta = if bits & (1 << (selector + 16)) == 0 { magnitude } else { -magnitude };
        let rgb = base.map(|channel| (channel + delta).clamp(0, 255) as u8);
        [rgb[0], rgb[1], rgb[2], 255]
    }))
}

fn channel(block: &[u8; 8], signed: bool) -> [u8; 16] {
    let bits = u64::from_be_bytes(*block);
    let base = if signed { i16::from((block[0] as i8).max(-127)) * 8 } else { i16::from(block[0]) * 8 + 4 };
    let scale = (i16::from(block[1] >> 4) * 8).max(1);
    let table = MODIFIERS.get(usize::from(block[1] & 15)).copied().unwrap_or_default();
    std::array::from_fn(|pixel| {
        let (x, y) = (pixel % 4, pixel / 4);
        let selector = ((bits >> (45 - 3 * (x * 4 + y))) & 7) as usize;
        let magnitude = table.get(selector & 3).copied().unwrap_or_default();
        let modifier = if selector < 4 { -magnitude - 1 } else { magnitude };
        let value = base + modifier * scale;
        if signed {
            let value = u32::try_from(value.clamp(-1023, 1023) + 1023).unwrap_or_default();
            ((value * 255 + 1023) / 2046) as u8
        } else {
            let value = u32::try_from(value.clamp(0, 2047)).unwrap_or_default();
            ((value * 255 + 1023) / 2047) as u8
        }
    })
}

pub(crate) fn scalar(block: &[u8; 8], signed: bool) -> [[u8; 4]; 16] {
    channel(block, signed).map(|value| [value, value, value, 255])
}

pub(crate) fn dual(block: &[u8; 16], signed: bool) -> [[u8; 4]; 16] {
    let (red, green) = split_halves(block);
    let (red, green) = (channel(&red, signed), channel(&green, signed));
    std::array::from_fn(|pixel| [red[pixel], green[pixel], 0, 255])
}
