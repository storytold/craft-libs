//! BC1 to BC5 (DXT1, DXT3, DXT5, ATI1, ATI2) blocks, after Microsoft's public block compression
//! documentation ("Block Compression (Direct3D 10)" and "Texture Block Compression in Direct3D
//! 11"): the block decoders and the block encoder the DDS reader and writer use. Nothing here
//! knows about DDS headers, so other containers of the same blocks can use it.
//!
//! BC4 is one block of 16 single-channel values, the same block BC3 stores its alpha in; BC5 is
//! two of them, red then green. [`value_block`] and [`decode_values`] serve all three;
//! [`value_block16`] and [`decode_values16`] are the same blocks for 16-bit values (BC4 and BC5
//! from 16-bit sources).
//!
//! The encoder fits a block's colours with two RGB565 endpoints along their principal axis,
//! refines them by least squares, and scores every candidate against the exact palette the
//! reader rebuilds, so the indices it picks are the best ones for what a decoder shows.

/// The 16 texels of a 4×4 block, row by row, as RGBA.
pub(crate) type Block = [[u8; 4]; 16];

/// Bit `t` set for each texel `t` of a block that counts: edge blocks of a texture whose size is
/// not a multiple of four have texels outside the image.
pub(crate) type Mask = u16;

pub(crate) fn rgb565(c: u16) -> [u8; 3] {
    let scale = |v: u16, max: u16| ((u32::from(v) * 255 + u32::from(max) / 2) / u32::from(max)) as u8;
    [scale(c >> 11, 31), scale((c >> 5) & 63, 63), scale(c & 31, 31)]
}

/// The colours a colour block's indices select. Four-colour mode (BC1 with `c0 > c1`, and every
/// BC2 and BC3 block) adds two colours at thirds; three-colour mode adds the midpoint and
/// transparent black.
pub(crate) fn colour_palette(c0: u16, c1: u16, four: bool) -> [[u8; 4]; 4] {
    let (a, b) = (rgb565(c0), rgb565(c1));
    let mix = |wa: u32, wb: u32| {
        let d = wa + wb;
        let at = |i: usize| ((wa * u32::from(a[i]) + wb * u32::from(b[i]) + d / 2) / d) as u8;
        [at(0), at(1), at(2), 255]
    };
    if four {
        [[a[0], a[1], a[2], 255], [b[0], b[1], b[2], 255], mix(2, 1), mix(1, 2)]
    } else {
        [[a[0], a[1], a[2], 255], [b[0], b[1], b[2], 255], mix(1, 1), [0; 4]]
    }
}

/// A value block's eight values: six interpolated at sevenths when `a0 > a1`, else four
/// interpolated at fifths followed by 0 and 255.
pub(crate) fn alpha_palette(a0: u8, a1: u8) -> [u8; 8] {
    let (a0, a1) = (u32::from(a0), u32::from(a1));
    let mut palette = [a0, a1, 0, 0, 0, 0, 0, 255];
    if a0 > a1 {
        for (k, v) in palette.iter_mut().enumerate().skip(2) {
            let k = k as u32;
            *v = ((8 - k) * a0 + (k - 1) * a1 + 3) / 7;
        }
    } else {
        for (k, v) in palette.iter_mut().enumerate().take(6).skip(2) {
            let k = k as u32;
            *v = ((6 - k) * a0 + (k - 1) * a1 + 2) / 5;
        }
    }
    palette.map(|v| v as u8)
}

/// A colour block's 16 texels as RGBA. BC2 and BC3 colour blocks always have four colours
/// (`one_bit_alpha` false); only BC1's `c0 <= c1` order selects three colours and transparent
/// black. Missing bytes read as 0.
pub(crate) fn decode_colour(block: &[u8], one_bit_alpha: bool) -> Block {
    let byte = |i: usize| block.get(i).copied().unwrap_or(0);
    let (c0, c1) = (u16::from_le_bytes([byte(0), byte(1)]), u16::from_le_bytes([byte(2), byte(3)]));
    let palette = colour_palette(c0, c1, c0 > c1 || !one_bit_alpha);
    let indices = u32::from_le_bytes([byte(4), byte(5), byte(6), byte(7)]);
    std::array::from_fn(|t| palette[(indices >> (2 * t)) as usize & 3])
}

/// [`alpha_palette`] at 16 bits: the same eight values before they are rounded to 8 bits, as a
/// decoder that interpolates at a higher precision shows them (endpoints scaled by 257, so 255
/// is 65535).
pub(crate) fn alpha_palette16(a0: u8, a1: u8) -> [u16; 8] {
    let (a0, a1) = (u32::from(a0) * 257, u32::from(a1) * 257);
    let mut palette = [a0, a1, 0, 0, 0, 0, 0, 65535];
    if a0 > a1 {
        for (k, v) in palette.iter_mut().enumerate().skip(2) {
            let k = k as u32;
            *v = ((8 - k) * a0 + (k - 1) * a1 + 3) / 7;
        }
    } else {
        for (k, v) in palette.iter_mut().enumerate().take(6).skip(2) {
            let k = k as u32;
            *v = ((6 - k) * a0 + (k - 1) * a1 + 2) / 5;
        }
    }
    palette.map(|v| v as u16)
}

/// BC2's alpha: four explicit bits per texel.
pub(crate) fn decode_bc2_alpha(block: &[u8]) -> [u8; 16] {
    let bits = u64::from_le_bytes(std::array::from_fn(|i| block.get(i).copied().unwrap_or(0)));
    std::array::from_fn(|t| ((bits >> (4 * t)) & 0xf) as u8 * 17)
}

/// A value block (BC3's alpha, BC4, either half of BC5): two endpoints and a 3-bit index per
/// texel into eight values, or into six plus 0 and 255 when `a0 <= a1`. Missing bytes read as 0.
pub(crate) fn decode_values(block: &[u8]) -> [u8; 16] {
    let byte = |i: usize| block.get(i).copied().unwrap_or(0);
    let palette = alpha_palette(byte(0), byte(1));
    let bits = u64::from_le_bytes([byte(2), byte(3), byte(4), byte(5), byte(6), byte(7), 0, 0]);
    std::array::from_fn(|t| palette[((bits >> (3 * t)) & 7) as usize])
}

/// [`decode_values`] at 16 bits, from [`alpha_palette16`].
pub(crate) fn decode_values16(block: &[u8]) -> [u16; 16] {
    let byte = |i: usize| block.get(i).copied().unwrap_or(0);
    let palette = alpha_palette16(byte(0), byte(1));
    let bits = u64::from_le_bytes([byte(2), byte(3), byte(4), byte(5), byte(6), byte(7), 0, 0]);
    std::array::from_fn(|t| palette[((bits >> (3 * t)) & 7) as usize])
}

/// A BC1 block. Texels under half opacity become BC1's transparent black, which needs the
/// three-colour mode; an opaque block takes whichever mode fits its colours better.
pub(crate) fn encode_bc1(block: &Block, inside: Mask) -> [u8; 8] {
    let transparent = select(block, inside, |t| t[3] < 128);
    colour_block(block, inside & !transparent, transparent, true)
}

/// A BC2 block: four bits of alpha per texel, then a four-colour block.
pub(crate) fn encode_bc2(block: &Block, inside: Mask) -> [u8; 16] {
    let bits = block.iter().enumerate().fold(0u64, |bits, (t, texel)| bits | ((u64::from(texel[3]) * 15 + 127) / 255) << (4 * t));
    join(bits.to_le_bytes(), colour_block(block, inside, 0, false))
}

/// A BC3 block: an interpolated alpha block, then a four-colour block.
pub(crate) fn encode_bc3(block: &Block, inside: Mask) -> [u8; 16] {
    let alphas = std::array::from_fn(|t| block[t][3]);
    join(value_block(&alphas, inside), colour_block(block, inside, 0, false))
}

fn join(a: [u8; 8], b: [u8; 8]) -> [u8; 16] {
    std::array::from_fn(|i| if i < 8 { a.get(i) } else { b.get(i - 8) }.copied().unwrap_or(0))
}

/// The texels of `inside` for which `f` holds.
fn select(block: &Block, inside: Mask, f: impl Fn(&[u8; 4]) -> bool) -> Mask {
    block.iter().enumerate().filter(|(t, texel)| inside >> t & 1 != 0 && f(texel)).fold(0, |m, (t, _)| m | 1 << t)
}

fn rgb_of(texel: &[u8; 4]) -> [f32; 3] {
    [f32::from(texel[0]), f32::from(texel[1]), f32::from(texel[2])]
}

/// RGB565 of a colour, rounding each channel to the nearest level.
fn to565(c: [f32; 3]) -> u16 {
    let q = |v: f32, max: f32| (v.clamp(0.0, 255.0) * max / 255.0).round() as u16;
    q(c[0], 31.0) << 11 | q(c[1], 63.0) << 5 | q(c[2], 31.0)
}

fn distance(a: &[u8; 4], b: &[u8; 4]) -> u32 {
    let d = |i: usize| u32::from(a[i].abs_diff(b[i])).pow(2);
    d(0) + d(1) + d(2)
}

/// The index of the first of `palette`'s entries nearest by `distance`, and that distance: the
/// entry `Iterator::min_by_key` picks, found without the iterator (a texel equal to an entry
/// stops the search, as no later entry can be nearer and an equal one doesn't win).
fn nearest<T>(palette: &[T], distance: impl Fn(&T) -> u32) -> (usize, u32) {
    let mut best = (0, u32::MAX);
    for (i, p) in palette.iter().enumerate() {
        let d = distance(p);
        if d < best.1 {
            best = (i, d);
            if d == 0 {
                break;
            }
        }
    }
    if palette.is_empty() { (0, 0) } else { best }
}

/// A colour block (endpoints, then 2-bit indices) for the `opaque` texels' colours, with the
/// `transparent` ones on BC1's transparent index. `bc1` allows the three-colour mode; BC2 and
/// BC3 always decode four colours.
fn colour_block(block: &Block, opaque: Mask, transparent: Mask, bc1: bool) -> [u8; 8] {
    // The opaque texels' colours in texel order, on the stack (after storytold/photocraft#10669,
    // which took the per-block allocations out of PhotoCraft's copy of this encoder).
    let mut buffer = [[0f32; 3]; 16];
    let mut n = 0;
    for (t, texel) in block.iter().enumerate() {
        if opaque >> t & 1 != 0
            && let Some(slot) = buffer.get_mut(n)
        {
            *slot = rgb_of(texel);
            n += 1;
        }
    }
    let colours = buffer.get(..n).unwrap_or_default();
    let Some((first, rest)) = colours.split_first() else {
        // Nothing visible: every texel on the transparent index (or index 0 of black).
        let indices = if transparent != 0 { u32::MAX } else { 0 };
        return pack(0, 0, indices);
    };
    // One colour has no axis: `principal_endpoints` returns that colour (their mean) for both
    // ends, so it is skipped. Everything after it runs as for any block.
    let (lo, hi) = if rest.iter().all(|c| c == first) { (*first, *first) } else { principal_endpoints(colours) };
    let mut candidate = (to565(lo), to565(hi));
    // The endpoint pairs fitted so far: at most one per round.
    let mut fitted = [candidate; 3];
    let mut count = 1;
    let modes: &[bool] = if transparent != 0 {
        &[false]
    } else if bc1 {
        &[true, false]
    } else {
        &[true]
    };
    let mut best: Option<(u32, [u8; 8])> = None;
    // Each refinement starts from the best block so far: its indices give the least-squares
    // endpoints, which become one more candidate.
    for round in 0..3 {
        let (e0, e1) = candidate;
        for &four in modes {
            let (bytes, error) = fit(block, opaque, transparent, e0, e1, four);
            if best.is_none_or(|(e, _)| error < e) {
                best = Some((error, bytes));
            }
        }
        let Some((error, bytes)) = best else { break };
        if error == 0 || round == 2 {
            break;
        }
        let Some(next) = least_squares(block, opaque, &bytes) else { break };
        // A pair fitted before (in either order: `fit` orders the endpoints itself) fits the same
        // again and can't beat the best block, which then gives the same pair again: the rounds
        // left would change nothing.
        let seen = fitted.get(..count).unwrap_or_default();
        if seen.iter().any(|&(a, b)| (a, b) == next || (b, a) == next) {
            break;
        }
        if let Some(slot) = fitted.get_mut(count) {
            *slot = next;
            count += 1;
        }
        candidate = next;
    }
    best.map_or([0; 8], |(_, bytes)| bytes)
}

/// Orders endpoints `e0`, `e1` for the mode (`c0 > c1` for four colours, `c0 <= c1` for three)
/// and picks each texel's nearest palette colour. Returns the block and its squared error.
fn fit(block: &Block, opaque: Mask, transparent: Mask, e0: u16, e1: u16, four: bool) -> ([u8; 8], u32) {
    let (c0, c1) = if four == (e0 >= e1) { (e0, e1) } else { (e1, e0) };
    // Equal endpoints can't be in four-colour order; the palette is then one colour either way.
    let palette = colour_palette(c0, c1, four && c0 > c1);
    let choices = if four && c0 > c1 { 4 } else { 3 };
    let mut indices = 0u32;
    let mut error = 0u32;
    for (t, texel) in block.iter().enumerate() {
        let index = if transparent >> t & 1 != 0 {
            3
        } else if opaque >> t & 1 != 0 {
            let (i, d) = nearest(palette.get(..choices).unwrap_or_default(), |p| distance(texel, p));
            error += d;
            i as u32
        } else {
            0
        };
        indices |= index << (2 * t);
    }
    (pack(c0, c1, indices), error)
}

fn pack(c0: u16, c1: u16, indices: u32) -> [u8; 8] {
    let (c0, c1, i) = (c0.to_le_bytes(), c1.to_le_bytes(), indices.to_le_bytes());
    [c0[0], c0[1], c1[0], c1[1], i[0], i[1], i[2], i[3]]
}

/// The ends of the colours' spread along their principal axis (the covariance's dominant
/// eigenvector, by power iteration), clamped to the RGB cube.
fn principal_endpoints(colours: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let n = colours.len().max(1) as f32;
    let mean = [0, 1, 2].map(|i| colours.iter().map(|c| c[i]).sum::<f32>() / n);
    let mut cov = [[0f32; 3]; 3];
    for c in colours {
        let d = [c[0] - mean[0], c[1] - mean[1], c[2] - mean[2]];
        for (i, row) in cov.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v += d[i] * d[j];
            }
        }
    }
    let times = |a: [f32; 3]| [0, 1, 2].map(|i| cov[i][0] * a[0] + cov[i][1] * a[1] + cov[i][2] * a[2]);
    let iterate = |start: usize| {
        let mut axis = [0f32; 3];
        axis[start] = 1.0;
        for _ in 0..8 {
            let next = times(axis);
            let len = next.iter().map(|v| v * v).sum::<f32>().sqrt();
            if !len.is_normal() {
                break;
            }
            let normalised = next.map(|v| v / len);
            // A step that returns the same axis, bit for bit, returns it on every later step too:
            // stopping there gives the very axis the remaining steps would. Kept only if timing
            // shows the comparison pays for itself.
            if normalised.map(f32::to_bits) == axis.map(f32::to_bits) {
                break;
            }
            axis = normalised;
        }
        axis
    };
    // From each channel's axis, keeping the one the colours spread along most: a channel that
    // varies on its own is already an eigenvector, so a single start there would never leave it
    // even when the others vary more together.
    let spread = |a: [f32; 3]| times(a).iter().zip(a).map(|(x, y)| x * y).sum::<f32>();
    let axis = [0, 1, 2].map(iterate).into_iter().max_by(|&a, &b| spread(a).total_cmp(&spread(b))).unwrap_or([1.0, 0.0, 0.0]);
    let along = |c: &[f32; 3]| (c[0] - mean[0]) * axis[0] + (c[1] - mean[1]) * axis[1] + (c[2] - mean[2]) * axis[2];
    let (lo, hi) = colours.iter().map(along).fold((f32::MAX, f32::MIN), |(lo, hi), t| (lo.min(t), hi.max(t)));
    let at = |t: f32| [0, 1, 2].map(|i| (mean[i] + t * axis[i]).clamp(0.0, 255.0));
    if lo > hi { (mean, mean) } else { (at(lo), at(hi)) }
}

/// The endpoints that minimise the squared error for the indices `bytes` already chose: each
/// texel's index fixes its weights between the endpoints, giving 2×2 normal equations per
/// channel. `None` when the indices don't constrain both endpoints.
fn least_squares(block: &Block, opaque: Mask, bytes: &[u8; 8]) -> Option<(u16, u16)> {
    let (c0, c1) = (u16::from_le_bytes([bytes[0], bytes[1]]), u16::from_le_bytes([bytes[2], bytes[3]]));
    let indices = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let four = c0 > c1;
    // How much of c0 each index holds.
    let weight = |i: u32| match (four, i) {
        (_, 0) => Some(1.0f32),
        (_, 1) => Some(0.0),
        (true, 2) => Some(2.0 / 3.0),
        (true, _) => Some(1.0 / 3.0),
        (false, 2) => Some(0.5),
        (false, _) => None,
    };
    let (mut aa, mut ab, mut bb) = (0f32, 0f32, 0f32);
    let (mut ax, mut bx) = ([0f32; 3], [0f32; 3]);
    for (t, texel) in block.iter().enumerate() {
        if opaque >> t & 1 == 0 {
            continue;
        }
        let Some(a) = weight(indices >> (2 * t) & 3) else { continue };
        let b = 1.0 - a;
        let c = rgb_of(texel);
        aa += a * a;
        ab += a * b;
        bb += b * b;
        for ((x0, x1), v) in ax.iter_mut().zip(bx.iter_mut()).zip(c) {
            *x0 += a * v;
            *x1 += b * v;
        }
    }
    let det = aa * bb - ab * ab;
    if det.abs() < 1e-3 {
        return None;
    }
    let e0 = [0, 1, 2].map(|i| (ax[i] * bb - bx[i] * ab) / det);
    let e1 = [0, 1, 2].map(|i| (bx[i] * aa - ax[i] * ab) / det);
    Some((to565(e0), to565(e1)))
}

/// A block of 16 single-channel values (BC3's alpha, BC4, either half of BC5): whichever of the two modes
/// (eight values between the extremes, or six between the extremes other than 0 and 255, plus
/// exact 0 and 255) fits better. Only the values of `inside` count.
pub(crate) fn value_block(values: &[u8; 16], inside: Mask) -> [u8; 8] {
    // Both ranges in one pass: of all values, and of those other than 0 and 255.
    let mut all: Option<(u8, u8)> = None;
    let mut mid = all;
    let widen = |r: Option<(u8, u8)>, v: u8| Some(r.map_or((v, v), |(lo, hi)| (lo.min(v), hi.max(v))));
    for (t, &v) in values.iter().enumerate() {
        if inside >> t & 1 != 0 {
            all = widen(all, v);
            if !matches!(v, 0 | 255) {
                mid = widen(mid, v);
            }
        }
    }
    let (lo, hi) = all.unwrap_or((0, 0));
    // Eight values need a0 > a1; equal ends fall to the six-value mode, which starts with a0 too.
    let eight = value_fit(values, inside, hi, lo);
    let (mid_lo, mid_hi) = mid.unwrap_or((0, 0));
    let six = value_fit(values, inside, mid_lo, mid_hi);
    if six.1 < eight.1 { six.0 } else { eight.0 }
}

fn value_fit(values: &[u8; 16], inside: Mask, a0: u8, a1: u8) -> ([u8; 8], u32) {
    let palette = alpha_palette(a0, a1);
    let mut bits = 0u64;
    let mut error = 0u32;
    for (t, &v) in values.iter().enumerate() {
        if inside >> t & 1 == 0 {
            continue;
        }
        let (i, d) = nearest(&palette, |&p| u32::from(p.abs_diff(v)).pow(2));
        error += d;
        bits |= (i as u64) << (3 * t);
    }
    let b = bits.to_le_bytes();
    ([a0, a1, b[0], b[1], b[2], b[3], b[4], b[5]], error)
}

/// [`value_block`] for 16-bit values: the endpoints are still 8 bits, but each end of each mode
/// tries the 8-bit levels just below and above the 16-bit extreme, and every candidate is scored
/// against the 16-bit palette ([`alpha_palette16`]). A block that spans less than one 8-bit level
/// is flat from 8-bit values but gets the interpolated steps between two adjacent levels here.
pub(crate) fn value_block16(values: &[u16; 16], inside: Mask) -> [u8; 8] {
    let mut all: Option<(u16, u16)> = None;
    let mut mid = all;
    let widen = |r: Option<(u16, u16)>, v: u16| Some(r.map_or((v, v), |(lo, hi)| (lo.min(v), hi.max(v))));
    for (t, &v) in values.iter().enumerate() {
        if inside >> t & 1 != 0 {
            all = widen(all, v);
            if !matches!(v, 0 | 65535) {
                mid = widen(mid, v);
            }
        }
    }
    let (lo, hi) = all.unwrap_or((0, 0));
    let (mid_lo, mid_hi) = mid.unwrap_or((0, 0));
    let (down, up) = (|v: u16| (v / 257) as u8, |v: u16| v.div_ceil(257) as u8);
    // The eight-value mode's pairs first (a0 the high end), then the six-value mode's: on a tie
    // the eight-value mode wins, as in `value_block`. Each pair's palette follows its order, so a
    // pair that lands in the other mode is still scored as the decoder reads it.
    let candidates = [
        (up(hi), down(lo)),
        (up(hi), up(lo)),
        (down(hi), down(lo)),
        (down(hi), up(lo)),
        (down(mid_lo), down(mid_hi)),
        (down(mid_lo), up(mid_hi)),
        (up(mid_lo), down(mid_hi)),
        (up(mid_lo), up(mid_hi)),
    ];
    let mut best: Option<([u8; 8], u64)> = None;
    for (a0, a1) in candidates {
        let fit = value_fit16(values, inside, a0, a1);
        if best.is_none_or(|(_, e)| fit.1 < e) {
            best = Some(fit);
        }
    }
    best.map_or([0; 8], |(bytes, _)| bytes)
}

fn value_fit16(values: &[u16; 16], inside: Mask, a0: u8, a1: u8) -> ([u8; 8], u64) {
    let palette = alpha_palette16(a0, a1);
    let mut bits = 0u64;
    let mut error = 0u64;
    for (t, &v) in values.iter().enumerate() {
        if inside >> t & 1 == 0 {
            continue;
        }
        let (i, d) = nearest(&palette, |&p| u32::from(p.abs_diff(v)).pow(2));
        error += u64::from(d);
        bits |= (i as u64) << (3 * t);
    }
    let b = bits.to_le_bytes();
    ([a0, a1, b[0], b[1], b[2], b[3], b[4], b[5]], error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_blocks_land_on_the_nearest_565_colour() {
        for colour in [[0, 0, 0, 255], [255, 255, 255, 255], [12, 200, 77, 255]] {
            let block = [colour; 16];
            let out = decode_colour(&encode_bc1(&block, u16::MAX), true);
            for texel in out {
                assert!(texel.iter().zip(colour).all(|(a, b)| a.abs_diff(b) <= 4), "{texel:?} for {colour:?}");
            }
        }
    }

    #[test]
    fn two_colour_blocks_are_exact_when_the_colours_are_565() {
        let (a, b) = ([255, 0, 0, 255], [0, 0, 255, 255]);
        let block: Block = std::array::from_fn(|t| if t % 3 == 0 { a } else { b });
        for bc1 in [true, false] {
            let bytes = if bc1 { encode_bc1(&block, u16::MAX) } else { colour_block(&block, u16::MAX, 0, false) };
            assert_eq!(decode_colour(&bytes, bc1), block);
        }
    }

    #[test]
    fn bc1_punches_through_texels_under_half_alpha() {
        let block: Block = std::array::from_fn(|t| if t < 8 { [200, 100, 50, 255] } else { [9, 9, 9, 127] });
        let bytes = encode_bc1(&block, u16::MAX);
        let (c0, c1) = (u16::from_le_bytes([bytes[0], bytes[1]]), u16::from_le_bytes([bytes[2], bytes[3]]));
        assert!(c0 <= c1, "three-colour mode");
        let out = decode_colour(&bytes, true);
        assert!(out[..8].iter().all(|t| t[3] == 255) && out[8..].iter().all(|t| *t == [0; 4]), "{out:?}");
    }

    #[test]
    fn four_colour_blocks_keep_c0_above_c1() {
        // A ramp needs all four colours; BC2 and BC3 blocks must not rely on the order, but
        // older decoders read c0 <= c1 as three colours, so the encoder never writes that.
        let block: Block = std::array::from_fn(|t| [(t * 16) as u8, 0, 0, 255]);
        let bytes = colour_block(&block, u16::MAX, 0, false);
        assert!(u16::from_le_bytes([bytes[0], bytes[1]]) > u16::from_le_bytes([bytes[2], bytes[3]]));
    }

    #[test]
    fn alpha_blocks_pick_the_mode_that_keeps_0_and_255() {
        // 0, 255 and a cluster in between: the six-value mode stores 0 and 255 exactly.
        let block: Block = std::array::from_fn(|t| [0, 0, 0, [0, 255, 100, 110][t % 4]]);
        let bytes = value_block(&block.map(|t| t[3]), u16::MAX);
        assert!(bytes[0] <= bytes[1], "six-value mode");
        let palette = alpha_palette(bytes[0], bytes[1]);
        let bits = u64::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], 0, 0]);
        for (t, texel) in block.iter().enumerate() {
            let got = palette[(bits >> (3 * t)) as usize & 7];
            assert!(got.abs_diff(texel[3]) <= 2, "texel {t}: {got} for {}", texel[3]);
        }
    }

    #[test]
    fn the_principal_axis_is_found_when_one_channel_varies_on_its_own() {
        // Red alternates on its own (variance 64); green and blue alternate together (variance 49
        // each, 98 along their shared axis), uncorrelated with red. Starting on red alone would
        // keep red's axis and flatten green and blue; the block must do better than its bounding box.
        let block: Block = std::array::from_fn(|t| {
            let gb = if (t / 2) % 2 == 0 { 121 } else { 135 };
            [if t % 2 == 0 { 120 } else { 136 }, gb, gb, 255]
        });
        let error = |bytes: &[u8; 8]| decode_colour(bytes, false).iter().zip(&block).map(|(a, b)| distance(a, b)).sum::<u32>();
        let (_, bounding_box) = fit(&block, u16::MAX, 0, to565([120.0, 121.0, 121.0]), to565([136.0, 135.0, 135.0]), true);
        let ours = error(&colour_block(&block, u16::MAX, 0, false));
        assert!(ours < bounding_box, "{ours} vs the bounding box's {bounding_box}");
    }

    #[test]
    fn value_blocks_decode_the_frozen_vectors() {
        // Eight-value mode (a0 = 200 > a1 = 50), every index 0.
        assert_eq!(decode_values(&[0xc8, 0x32, 0, 0, 0, 0, 0, 0]), [200; 16]);
        // Six-value mode (a0 = 10 <= a1 = 250), every index 1.
        assert_eq!(decode_values(&[0x0a, 0xfa, 0x49, 0x92, 0x24, 0x49, 0x92, 0x24]), [250; 16]);
        // Six-value mode's indices 6 and 7 are 0 and 255 whatever the endpoints.
        let bits: u64 = (0..16).fold(0, |b, t| b | (6 + (t % 2)) << (3 * t));
        let b = bits.to_le_bytes();
        let got = decode_values(&[10, 250, b[0], b[1], b[2], b[3], b[4], b[5]]);
        assert_eq!(got, std::array::from_fn(|t| if t % 2 == 0 { 0 } else { 255 }));
    }

    #[test]
    fn value_blocks_round_trip_ramps_and_solids() {
        for values in [[200u8; 16], [0; 16], [255; 16], std::array::from_fn(|t| (t * 17) as u8), std::array::from_fn(|t| 100 + t as u8)] {
            let back = decode_values(&value_block(&values, u16::MAX));
            let worst = values.iter().zip(back).map(|(a, b)| a.abs_diff(b)).max().unwrap_or(0);
            // Within half a step of either mode: 255 / 7 apart in the eight-value mode, about
            // 221 / 5 in the six-value one.
            assert!(worst <= 23, "{values:?} -> {back:?}");
        }
        assert_eq!(decode_values(&value_block(&[200; 16], u16::MAX)), [200; 16]);
    }

    #[test]
    fn nearest_picks_what_min_by_key_picks() {
        // Ties (equal entries, entries equally far on both sides) and exact hits included.
        let mut seed = 0x2545_f491u32;
        for _ in 0..2000 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let palette = [seed as u8 & 0xf0, (seed >> 8) as u8 & 0xf0, (seed >> 16) as u8 & 0xf0, (seed >> 24) as u8 & 0xf0];
            let v = (seed >> 4) as u8 & 0xf8;
            let d = |p: &u8| u32::from(p.abs_diff(v)).pow(2);
            let want = palette.iter().enumerate().map(|(i, p)| (i, d(p))).min_by_key(|&(_, d)| d).unwrap();
            assert_eq!(nearest(&palette, d), want, "{palette:?} {v}");
        }
        assert_eq!(nearest::<u8>(&[], |_| 7), (0, 0));
    }

    #[test]
    fn value_blocks_decode_the_frozen_vectors_at_16_bits() {
        // Eight-value mode (200 > 50): index 2 is (6 * 200 + 50) / 7 of 255 at 16 bits, which
        // the 8-bit palette rounds to 179.
        let bits: u64 = (0..16).fold(0, |b, t| b | 2 << (3 * t));
        let b = bits.to_le_bytes();
        let block = [200, 50, b[0], b[1], b[2], b[3], b[4], b[5]];
        assert_eq!(decode_values16(&block), [(((6 * 200 + 50) * 257 + 3) / 7_u32) as u16; 16]);
        assert_eq!(decode_values(&block), [179; 16]);
        // Six-value mode's indices 6 and 7 are 0 and 65535.
        let bits: u64 = (0..16).fold(0, |b, t| b | (6 + (t % 2)) << (3 * t));
        let b = bits.to_le_bytes();
        let got = decode_values16(&[10, 250, b[0], b[1], b[2], b[3], b[4], b[5]]);
        assert_eq!(got, std::array::from_fn(|t| if t % 2 == 0 { 0 } else { 65535 }));
    }

    #[test]
    fn value_blocks_from_16_bits_step_between_two_8_bit_levels() {
        // A ramp across 0.8 of one 8-bit level: from 8 bits it is one or two flat values; from 16
        // bits the endpoints straddle it and the steps between them follow the ramp.
        let values: [u16; 16] = std::array::from_fn(|t| 25_700 + (t as u16) * 13);
        let error = |got: [u16; 16]| values.iter().zip(got).map(|(a, b)| u64::from(a.abs_diff(b)).pow(2)).sum::<u64>();
        let ours = error(decode_values16(&value_block16(&values, u16::MAX)));
        let from_8_bits = error(decode_values16(&value_block(&values.map(|v| ((u32::from(v) + 128) / 257) as u8), u16::MAX)));
        assert!(ours * 4 < from_8_bits, "{ours} vs {from_8_bits} from 8 bits");
        // Values outside the image don't count.
        let solid = value_block16(&std::array::from_fn(|t| if t == 0 { 1000 } else { 60_000 }), 1);
        assert!(decode_values16(&solid)[0].abs_diff(1000) < 129);
    }

    #[test]
    fn texels_outside_the_image_are_ignored() {
        // Only the top-left texel is inside: its colour alone decides the endpoints.
        let mut block: Block = [[0, 255, 0, 255]; 16];
        block[0] = [255, 0, 255, 255];
        let out = decode_colour(&encode_bc1(&block, 1), true);
        assert_eq!(out[0], [255, 0, 255, 255]);
    }
}
