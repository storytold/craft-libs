//! The block-compressed texture formats Hap carries: DXT1 (BC1), DXT5 (BC3) and RGTC1 (BC4), with
//! the scaled YCoCg colour transform of Hap Q. Blocks are 4x4 pixels, 8 bytes (DXT1, BC4) or 16
//! bytes (DXT5). The decoders give RGBA and the encoders are range-fit block compressors: the
//! two endpoints are the extremes of the block's colours (or alphas) and each pixel takes the
//! nearest of the palette those endpoints make.

/// A 4x4 block of RGBA pixels, row-major.
pub(super) type Block = [[u8; 4]; 16];

// The 5- and 6-bit endpoint channels widened to 8 bits as ffmpeg widens them (observed: its decoder
// is the reference for Hap pictures): about `v * 255 / 31` and `v * 255 / 63`, rounded, in fixed point.
fn expand5(v: u16) -> u8 {
    ((u32::from(v) * 263 + 16) >> 5) as u8
}
fn expand6(v: u16) -> u8 {
    ((u32::from(v) * 259 + 32) >> 6) as u8
}

fn rgb565(c: u16) -> [u8; 3] {
    [expand5((c >> 11) & 31), expand6((c >> 5) & 63), expand5(c & 31)]
}

/// The four colours of a colour block. With `always_four` (the colour half of a DXT5 block) two
/// colours are interpolated at thirds between the endpoints; in DXT1 that is so only when the first
/// endpoint is above the second, and otherwise one colour is the half and one is black.
fn palette(c0: u16, c1: u16, always_four: bool) -> [[u8; 3]; 4] {
    let (a, b) = (rgb565(c0), rgb565(c1));
    let mix = |wa: u32, wb: u32, d: u32| -> [u8; 3] { std::array::from_fn(|i| ((wa * u32::from(a[i]) + wb * u32::from(b[i])) / d) as u8) };
    if always_four || c0 > c1 { [a, b, mix(2, 1, 3), mix(1, 2, 3)] } else { [a, b, mix(1, 1, 2), [0; 3]] }
}

/// Decode a DXT1 colour block (8 bytes) into the RGB of 16 pixels.
pub(super) fn decode_color(block: &[u8], always_four: bool) -> [[u8; 3]; 16] {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let pal = palette(c0, c1, always_four);
    let bits = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    std::array::from_fn(|i| pal[((bits >> (2 * i)) & 3) as usize])
}

/// The eight alphas of a BC4 / DXT5 alpha block, from its two endpoints.
fn alpha_palette(a0: u8, a1: u8) -> [u8; 8] {
    let (x, y) = (u32::from(a0), u32::from(a1));
    let mut p = [a0, a1, 0, 0, 0, 0, 0, 0];
    if a0 > a1 {
        for i in 2..8u32 {
            p[i as usize] = (((8 - i) * x + (i - 1) * y) / 7) as u8;
        }
    } else {
        for i in 2..6u32 {
            p[i as usize] = (((6 - i) * x + (i - 1) * y) / 5) as u8;
        }
        p[6] = 0;
        p[7] = 255;
    }
    p
}

/// Decode a BC4 / DXT5 alpha block (8 bytes) into 16 alphas.
pub(super) fn decode_alpha(block: &[u8]) -> [u8; 16] {
    let pal = alpha_palette(block[0], block[1]);
    let mut bits = 0u64;
    for (i, &b) in block[2..8].iter().enumerate() {
        bits |= u64::from(b) << (8 * i);
    }
    std::array::from_fn(|i| pal[((bits >> (3 * i)) & 7) as usize])
}

pub(super) fn decode_dxt1(block: &[u8]) -> Block {
    let c = decode_color(block, false);
    std::array::from_fn(|i| [c[i][0], c[i][1], c[i][2], 255])
}

pub(super) fn decode_dxt5(block: &[u8]) -> Block {
    let a = decode_alpha(&block[..8]);
    let c = decode_color(&block[8..16], true);
    std::array::from_fn(|i| [c[i][0], c[i][1], c[i][2], a[i]])
}

/// Scaled YCoCg-DXT5 (Hap Q): the colour block holds Co, Cg and a scale in the red, green and blue
/// channels, the alpha block holds Y.
pub(super) fn decode_ycocg(block: &[u8]) -> Block {
    let y = decode_alpha(&block[..8]);
    let c = decode_color(&block[8..16], true);
    std::array::from_fn(|i| {
        let s = i32::from(c[i][2] >> 3) + 1;
        let co = i32::from(c[i][0]) - 128;
        let cg = i32::from(c[i][1]) - 128;
        let y = i32::from(y[i]);
        let (co, cg) = (co / s, cg / s);
        let r = y + co - cg;
        let g = y + cg;
        let b = y - co - cg;
        [r.clamp(0, 255) as u8, g.clamp(0, 255) as u8, b.clamp(0, 255) as u8, 255]
    })
}

/// Decode a BC4 block (8 bytes) as an alpha mask.
pub(super) fn decode_bc4(block: &[u8]) -> [u8; 16] {
    decode_alpha(block)
}

// ----------------------------------------------------------------------------------- encoders

fn pack565(c: [u8; 3]) -> u16 {
    let r = (u16::from(c[0]) * 31 + 127) / 255;
    let g = (u16::from(c[1]) * 63 + 127) / 255;
    let b = (u16::from(c[2]) * 31 + 127) / 255;
    (r << 11) | (g << 5) | b
}

fn dist(a: [u8; 3], b: [u8; 3]) -> u32 {
    (0..3).map(|i| (i32::from(a[i]) - i32::from(b[i])).pow(2) as u32).sum()
}

/// Encode 16 RGB pixels as a DXT1 colour block.
pub(super) fn encode_color(px: &[[u8; 3]; 16], always_four: bool) -> [u8; 8] {
    let mut lo = [255u8; 3];
    let mut hi = [0u8; 3];
    for p in px {
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    // Pull the endpoints in a little: the extremes of a box are rarely colours of the block.
    for i in 0..3 {
        let inset = (hi[i] - lo[i]) >> 4;
        lo[i] += inset;
        hi[i] -= inset;
    }
    let (mut c0, mut c1) = (pack565(hi), pack565(lo));
    if c0 < c1 {
        std::mem::swap(&mut c0, &mut c1);
    }
    let mut out = [0u8; 8];
    out[..2].copy_from_slice(&c0.to_le_bytes());
    out[2..4].copy_from_slice(&c1.to_le_bytes());
    if c0 == c1 {
        return out; // every index 0
    }
    let pal = palette(c0, c1, always_four);
    let mut bits = 0u32;
    for (i, p) in px.iter().enumerate() {
        let best = (0..4).min_by_key(|&k| dist(*p, pal[k])).unwrap_or(0);
        bits |= (best as u32) << (2 * i);
    }
    out[4..].copy_from_slice(&bits.to_le_bytes());
    out
}

/// Encode 16 alphas as a BC4 / DXT5 alpha block.
pub(super) fn encode_alpha(a: &[u8; 16]) -> [u8; 8] {
    let hi = a.iter().copied().max().unwrap_or(0);
    let lo = a.iter().copied().min().unwrap_or(0);
    let mut out = [0u8; 8];
    out[0] = hi;
    out[1] = lo;
    if hi == lo {
        return out;
    }
    let pal = alpha_palette(hi, lo);
    let mut bits = 0u64;
    for (i, &v) in a.iter().enumerate() {
        let best = (0..8).min_by_key(|&k| (i32::from(v) - i32::from(pal[k])).abs()).unwrap_or(0);
        bits |= (best as u64) << (3 * i);
    }
    for (i, b) in out[2..].iter_mut().enumerate() {
        *b = (bits >> (8 * i)) as u8;
    }
    out
}

fn rgb_of(block: &Block) -> [[u8; 3]; 16] {
    std::array::from_fn(|i| [block[i][0], block[i][1], block[i][2]])
}

pub(super) fn encode_dxt1(block: &Block) -> [u8; 8] {
    encode_color(&rgb_of(block), false)
}

pub(super) fn encode_dxt5(block: &Block) -> [u8; 16] {
    let a = encode_alpha(&std::array::from_fn(|i| block[i][3]));
    let c = encode_color(&rgb_of(block), true);
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&a);
    out[8..].copy_from_slice(&c);
    out
}

pub(super) fn encode_bc4(block: &Block) -> [u8; 8] {
    encode_alpha(&std::array::from_fn(|i| block[i][3]))
}

/// Encode a block as scaled YCoCg-DXT5.
pub(super) fn encode_ycocg(block: &Block) -> [u8; 16] {
    // Co = (R - B) / 2, Cg = (2G - R - B) / 4, Y = (R + 2G + B) / 4, all in 8 bits.
    let co2: [i32; 16] = std::array::from_fn(|i| i32::from(block[i][0]) - i32::from(block[i][2]));
    let cg4: [i32; 16] = std::array::from_fn(|i| 2 * i32::from(block[i][1]) - i32::from(block[i][0]) - i32::from(block[i][2]));
    let m2 = co2.iter().map(|v| v.abs()).max().unwrap_or(0); // twice the largest Co
    let m4 = cg4.iter().map(|v| v.abs()).max().unwrap_or(0); // four times the largest Cg
    let mag = (m2 / 2).max(m4 / 4);
    let s: i32 = if mag <= 31 {
        4
    } else if mag <= 63 {
        2
    } else {
        1
    };
    let colors: [[u8; 3]; 16] = std::array::from_fn(|i| {
        let co = (128 + ((co2[i] * s + 1) >> 1)).clamp(0, 255) as u8;
        let cg = (128 + ((cg4[i] * s + 2) >> 2)).clamp(0, 255) as u8;
        [co, cg, ((s - 1) * 8) as u8]
    });
    let y: [u8; 16] = std::array::from_fn(|i| ((i32::from(block[i][0]) + 2 * i32::from(block[i][1]) + i32::from(block[i][2]) + 2) >> 2) as u8);
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&encode_alpha(&y));
    out[8..].copy_from_slice(&encode_color(&colors, true));
    out
}
