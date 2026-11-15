use super::decode::{Layout, Patched, Prepared, Src, is_eof};
use super::{CodecError, Decoder, SampleType, err, f16, map_err};

/// The a* and b* samples (1 and 2 of every pixel) of a Lab image between the file's encoding and
/// PhotoCraft's (8 bits: `a + 128`; 16 bits: `(a + 128) · 257`). In the file, CIELab holds them
/// signed (8 bits: `a`; 16 bits: `a · 256`) and ICCLab offset (8 bits: `a + 128`; 16 bits:
/// `(a + 128) · 256`). `to_file` picks the direction; samples are native-endian.
pub(super) fn convert_ab(data: &mut [u8], channels: usize, sample: SampleType, signed: bool, to_file: bool) {
    match sample {
        SampleType::U8 if signed => data.chunks_exact_mut(channels).for_each(|px| {
            px[1] ^= 0x80;
            px[2] ^= 0x80;
        }),
        SampleType::U16 => data.chunks_exact_mut(channels * 2).for_each(|px| {
            for c in [1, 2] {
                let v = u16::from_ne_bytes([px[2 * c], px[2 * c + 1]]);
                // The file's value as `a · 256` (signed) and back.
                let file_a256 = |v: u16| if signed { f64::from(v as i16) } else { f64::from(v) - 32768.0 };
                let out = if to_file {
                    let a256 = (f64::from(v) - 32896.0) * 256.0 / 257.0;
                    let a256 = a256.round().clamp(-32768.0, 32767.0);
                    if signed { (a256 as i16) as u16 } else { (a256 + 32768.0) as u16 }
                } else {
                    (file_a256(v) * 257.0 / 256.0 + 32896.0).round().clamp(0.0, 65535.0) as u16
                };
                px[2 * c..2 * c + 2].copy_from_slice(&out.to_ne_bytes());
            }
        }),
        _ => {}
    }
}

/// File sample indices that are not a colour channel and not the one alpha sample, capped at
/// [`crate::MAX_EXTRA_SAMPLES`], and how many past the cap were left out.
pub(super) fn extra_sample_indices(spp: usize, color: usize, alpha_at: Option<usize>, has_alpha: bool) -> (Vec<usize>, usize) {
    let mut used = vec![false; spp];
    for i in 0..color.min(spp) {
        if let Some(slot) = used.get_mut(i) {
            *slot = true;
        }
    }
    if has_alpha {
        let a = alpha_at.map(|k| color.saturating_add(k)).unwrap_or(color);
        if let Some(slot) = used.get_mut(a) {
            *slot = true;
        }
    }
    let mut all: Vec<usize> = used.into_iter().enumerate().filter(|(_, on)| !*on).map(|(i, _)| i).collect();
    let over = all.len().saturating_sub(crate::MAX_EXTRA_SAMPLES);
    all.truncate(crate::MAX_EXTRA_SAMPLES);
    (all, over)
}

/// Interleaved extra samples (`extra_at.len()` per pixel) → one plane per sample. Each plane is
/// allocated fallibly (the caller has checked their total against the limits).
pub(super) fn split_extra_samples(
    buf: &[u8],
    w: usize,
    h: usize,
    at: &[usize],
    sample: SampleType,
    mut kind_of: impl FnMut(usize) -> u16,
) -> Result<Vec<crate::ExtraSample>, CodecError> {
    let b = sample.bytes();
    let n = w.saturating_mul(h);
    let spp = at.len();
    let mut out = Vec::with_capacity(spp);
    for (ei, &si) in at.iter().enumerate() {
        let mut data = zeroed(n.saturating_mul(b))?;
        for i in 0..n {
            let src = (i.saturating_mul(spp).saturating_add(ei)).saturating_mul(b);
            let dst = i.saturating_mul(b);
            if let (Some(s), Some(d)) = (buf.get(src..src.saturating_add(b)), data.get_mut(dst..dst.saturating_add(b))) {
                d.copy_from_slice(s);
            }
        }
        out.push(crate::ExtraSample { kind: kind_of(si), data });
    }
    Ok(out)
}

/// A zero-filled buffer, allocated fallibly. On native targets the zeroing (and the first
/// touch of every page, which dominates on a fresh multi-hundred-megabyte buffer) is spread over
/// all cores.
pub(super) fn zeroed(len: usize) -> Result<Vec<u8>, CodecError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| CodecError::LimitExceeded(format!("cannot allocate {len} bytes for the image")))?;
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        // The capacity is already there: this only writes the zeros.
        v.par_extend(rayon::iter::repeat_n(0u8, len));
    }
    #[cfg(target_arch = "wasm32")]
    v.resize(len, 0);
    Ok(v)
}

/// `v` refilled with `len` zero bytes, reusing its allocation when it is large enough.
pub(super) fn refill_zeroed(mut v: Vec<u8>, len: usize) -> Result<Vec<u8>, CodecError> {
    if v.capacity() < len {
        drop(v);
        return zeroed(len);
    }
    v.clear();
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        v.par_extend(rayon::iter::repeat_n(0u8, len));
    }
    #[cfg(target_arch = "wasm32")]
    v.resize(len, 0);
    Ok(v)
}

/// Chunks decoded and chunks whose data ended early.
#[derive(Default, Clone, Copy)]
pub(super) struct Tally {
    pub(super) decoded: usize,
    pub(super) missing: usize,
}

impl std::ops::Add for Tally {
    type Output = Tally;
    fn add(self, o: Tally) -> Tally {
        Tally { decoded: self.decoded + o.decoded, missing: self.missing + o.missing }
    }
}

/// Decodes the bands from `first` on into `data` (`band_bytes` each), in parallel on native
/// targets (one decoder and scratch buffer per worker). `extra` receives samples that are not
/// part of the colour image, `extra_band` bytes per band, when it is non-empty.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn decode_bands(
    p: &Prepared,
    b: &[u8],
    data: &mut [u8],
    band_bytes: usize,
    first: usize,
    extra: &mut [u8],
    extra_band: usize,
) -> Result<Tally, CodecError> {
    use rayon::prelude::*;
    if extra.is_empty() {
        return data
            .par_chunks_mut(band_bytes.max(1))
            .enumerate()
            .map_init(
                || p.open(b).map(|d| (d, Vec::new())).map_err(|e| e.to_string()),
                |worker, (i, band)| {
                    let (dec, scratch) = worker.as_mut().map_err(|e| err(e.clone()))?;
                    decode_band(&p.lay, dec, scratch, first + i, band, None)
                },
            )
            .try_reduce(Tally::default, |a, b| Ok(a + b));
    }
    data.par_chunks_mut(band_bytes.max(1))
        .zip(extra.par_chunks_mut(extra_band.max(1)))
        .enumerate()
        .map_init(
            || p.open(b).map(|d| (d, Vec::new())).map_err(|e| e.to_string()),
            |worker, (i, (band, extra_band))| {
                let (dec, scratch) = worker.as_mut().map_err(|e| err(e.clone()))?;
                decode_band(&p.lay, dec, scratch, first + i, band, Some(extra_band))
            },
        )
        .try_reduce(Tally::default, |a, b| Ok(a + b))
}

/// Decodes the bands from `first` on into `data` (`band_bytes` each), one after the other.
#[cfg(target_arch = "wasm32")]
pub(super) fn decode_bands(
    p: &Prepared,
    b: &[u8],
    data: &mut [u8],
    band_bytes: usize,
    first: usize,
    extra: &mut [u8],
    extra_band: usize,
) -> Result<Tally, CodecError> {
    let mut dec = p.open(b)?;
    let mut scratch = Vec::new();
    let mut tally = Tally::default();
    if extra.is_empty() {
        for (i, band) in data.chunks_mut(band_bytes.max(1)).enumerate() {
            tally = tally + decode_band(&p.lay, &mut dec, &mut scratch, first + i, band, None)?;
        }
        return Ok(tally);
    }
    for (i, (band, extra_band)) in data.chunks_mut(band_bytes.max(1)).zip(extra.chunks_mut(extra_band.max(1))).enumerate() {
        tally = tally + decode_band(&p.lay, &mut dec, &mut scratch, first + i, band, Some(extra_band))?;
    }
    Ok(tally)
}

/// Decodes band `b` (one strip, or one row of tiles, in every plane) into its output rows.
fn decode_band(
    lay: &Layout,
    dec: &mut Decoder<Patched<'_>>,
    scratch: &mut Vec<u8>,
    b: usize,
    band: &mut [u8],
    mut extra: Option<&mut [u8]>,
) -> Result<Tally, CodecError> {
    let out_row = lay.out_row();
    let rows = band.len() / out_row.max(1);
    let px = lay.out_channels() * lay.out.bytes();
    let mut tally = Tally::default();
    for plane in 0..lay.planes {
        let file_plane = lay.source_sample(plane);
        for col in 0..lay.across {
            let chunk = file_plane
                .checked_mul(lay.chunks_per_plane)
                .and_then(|c| c.checked_add(b.checked_mul(lay.across)?))
                .and_then(|c| c.checked_add(col))
                .and_then(|c| u32::try_from(c).ok())
                .ok_or_else(|| err("chunk index out of range"))?;
            let pref = dec.image_chunk_buffer_layout(chunk).map_err(map_err)?;
            let stride = pref.row_stride.map(|s| s.get()).ok_or_else(|| err("empty chunk row"))?;
            // In place: the strip's decoded bytes are exactly its output rows.
            if lay.direct() && pref.len == band.len() && stride == out_row {
                match dec.read_chunk_bytes(chunk, band) {
                    Ok(()) => tally.decoded += 1,
                    // Rows read before the end would survive: a missing strip is zeros.
                    Err(e) if is_eof(&e) => {
                        band.fill(0);
                        tally.missing += 1;
                    }
                    Err(e) => return Err(map_err(e)),
                }
                continue;
            }
            if pref.len > lay.scratch_cap {
                return Err(CodecError::LimitExceeded(format!("a {} byte strip or tile", pref.len)));
            }
            if scratch.len() < pref.len {
                scratch
                    .try_reserve_exact(pref.len - scratch.len())
                    .map_err(|_| CodecError::LimitExceeded(format!("cannot allocate {} bytes for a strip or tile", pref.len)))?;
                scratch.resize(pref.len, 0);
            }
            let buf = scratch.get_mut(..pref.len).ok_or_else(|| err("scratch buffer"))?;
            match dec.read_chunk_bytes(chunk, buf) {
                Ok(()) => tally.decoded += 1,
                Err(e) if is_eof(&e) => {
                    tally.missing += 1;
                    continue;
                }
                Err(e) => return Err(map_err(e)),
            }
            let x0 = col * lay.cw;
            let n = lay.cw.min(lay.w.saturating_sub(x0));
            for (ri, (src_row, dst_row)) in buf.chunks_exact(stride).zip(band.chunks_exact_mut(out_row)).take(rows).enumerate() {
                let Some(dst) = dst_row.get_mut(x0 * px..(x0 + n) * px) else { continue };
                convert_row(lay, src_row, dst, n, plane);
                if !lay.planar && plane == 0 && !lay.extra_at.is_empty() {
                    let erow = lay.extra_row();
                    if let Some(extra) = extra.as_deref_mut()
                        && let Some(er) = extra.get_mut(ri.saturating_mul(erow)..ri.saturating_mul(erow).saturating_add(erow))
                    {
                        deposit_extras(lay, src_row, er, n, x0);
                    }
                }
            }
        }
    }
    if lay.planar && !lay.extra_at.is_empty() {
        read_planar_extras(lay, dec, scratch, b, rows, extra, &mut tally)?;
    }
    finish_band(lay, band);
    Ok(tally)
}

/// Copies the non-colour samples of a chunky row into the extra-sample band.
fn deposit_extras(lay: &Layout, src: &[u8], dst: &mut [u8], n: usize, x0: usize) {
    let ech = lay.extra_at.len();
    if ech == 0 {
        return;
    }
    let px = ech * lay.out.bytes();
    let Some(dst) = dst.get_mut(x0.saturating_mul(px)..(x0.saturating_add(n)).saturating_mul(px)) else { return };
    for (ei, &si) in lay.extra_at.iter().enumerate() {
        transfer(lay.src, src, lay.chunk_spp, si, dst, lay.out, ech, ei, n);
    }
}

/// Planar files store each extra sample as its own plane. Read those planes into `extra`.
fn read_planar_extras(
    lay: &Layout,
    dec: &mut Decoder<Patched<'_>>,
    scratch: &mut Vec<u8>,
    b: usize,
    rows: usize,
    extra: Option<&mut [u8]>,
    tally: &mut Tally,
) -> Result<(), CodecError> {
    let Some(extra) = extra else { return Ok(()) };
    let ech = lay.extra_at.len();
    let erow = lay.extra_row();
    if ech == 0 || erow == 0 {
        return Ok(());
    }
    for (ei, &file_plane) in lay.extra_at.iter().enumerate() {
        for col in 0..lay.across {
            let chunk = file_plane
                .checked_mul(lay.chunks_per_plane)
                .and_then(|c| c.checked_add(b.checked_mul(lay.across)?))
                .and_then(|c| c.checked_add(col))
                .and_then(|c| u32::try_from(c).ok())
                .ok_or_else(|| err("chunk index out of range"))?;
            let pref = dec.image_chunk_buffer_layout(chunk).map_err(map_err)?;
            let stride = pref.row_stride.map(|s| s.get()).ok_or_else(|| err("empty chunk row"))?;
            if pref.len > lay.scratch_cap {
                return Err(CodecError::LimitExceeded(format!("a {} byte strip or tile", pref.len)));
            }
            if scratch.len() < pref.len {
                scratch
                    .try_reserve_exact(pref.len - scratch.len())
                    .map_err(|_| CodecError::LimitExceeded(format!("cannot allocate {} bytes for a strip or tile", pref.len)))?;
                scratch.resize(pref.len, 0);
            }
            let buf = scratch.get_mut(..pref.len).ok_or_else(|| err("scratch buffer"))?;
            match dec.read_chunk_bytes(chunk, buf) {
                Ok(()) => tally.decoded += 1,
                Err(e) if is_eof(&e) => {
                    tally.missing += 1;
                    continue;
                }
                Err(e) => return Err(map_err(e)),
            }
            let x0 = col * lay.cw;
            let n = lay.cw.min(lay.w.saturating_sub(x0));
            for (ri, src_row) in buf.chunks_exact(stride).take(rows).enumerate() {
                let start = ri.saturating_mul(erow);
                let Some(er) = extra.get_mut(start..start.saturating_add(erow)) else { continue };
                let px = ech * lay.out.bytes();
                let Some(dst) = er.get_mut(x0.saturating_mul(px)..(x0.saturating_add(n)).saturating_mul(px)) else { continue };
                transfer(lay.src, src_row, 1, 0, dst, lay.out, ech, ei, n);
            }
        }
    }
    Ok(())
}

/// Converts `n` pixels of one decoded chunk row into output pixels (`plane` for planar data).
fn convert_row(lay: &Layout, src: &[u8], dst: &mut [u8], n: usize, plane: usize) {
    let och = lay.out_channels();
    if let Some(map) = &lay.palette {
        let ob = lay.out.bytes();
        for (i, d) in dst.chunks_exact_mut(och * ob).take(n).enumerate() {
            let index = sample_index(lay.src, src, i);
            let Some(rgb) = map.get(index) else { continue };
            for (c, v) in d.chunks_exact_mut(ob).zip(rgb) {
                if ob == 1 {
                    c.copy_from_slice(&[(v >> 8) as u8]);
                } else {
                    c.copy_from_slice(&v.to_ne_bytes());
                }
            }
        }
        return;
    }
    if lay.copyable() {
        if let Some(s) = src.get(..dst.len()) {
            dst.copy_from_slice(s);
        }
        return;
    }
    if lay.planar {
        transfer(lay.src, src, 1, 0, dst, lay.out, och, plane, n);
    } else {
        for k in 0..och {
            transfer(lay.src, src, lay.chunk_spp, lay.source_sample(k), dst, lay.out, och, k, n);
        }
    }
}

/// The value of pixel `i` in a row of one-sample indices (palette and sub-byte gray).
fn sample_index(src: Src, row: &[u8], i: usize) -> usize {
    match src {
        Src::Bits(bits) => {
            let bits = usize::from(bits);
            let bit = i * bits;
            let byte = row.get(bit / 8).copied().unwrap_or(0);
            usize::from((byte >> (8 - bits - bit % 8)) & ((1u8 << bits) - 1))
        }
        Src::U8 => row.get(i).copied().map_or(0, usize::from),
        Src::U16 => row.get(i * 2..i * 2 + 2).and_then(|s| s.try_into().ok()).map_or(0, |s| usize::from(u16::from_ne_bytes(s))),
        _ => 0,
    }
}

/// Copies sample `si` of each of `n` decoded pixels (`sspp` samples each) to output channel
/// `di` (`dch` channels), converting the sample type.
#[allow(clippy::too_many_arguments)]
fn transfer(src: Src, row: &[u8], sspp: usize, si: usize, dst: &mut [u8], out: SampleType, dch: usize, di: usize, n: usize) {
    let ob = out.bytes();
    if let Src::Bits(bits) = src {
        let max = (1u32 << bits) - 1;
        for (i, d) in dst.chunks_exact_mut(dch * ob).take(n).enumerate() {
            if let Some(d) = d.get_mut(di) {
                *d = (sample_index(src, row, i) as u32 * 255 / max) as u8;
            }
        }
        return;
    }
    let sb = src.bytes();
    if src.is_identity() {
        // Same sample type: a strided copy.
        if sb == 1 {
            for (d, s) in dst.iter_mut().skip(di).step_by(dch).zip(row.iter().skip(si).step_by(sspp)).take(n) {
                *d = *s;
            }
        } else {
            for (d, s) in dst.chunks_exact_mut(ob).skip(di).step_by(dch).zip(row.chunks_exact(sb).skip(si).step_by(sspp)).take(n) {
                d.copy_from_slice(s);
            }
        }
        return;
    }
    for (s, d) in row.chunks_exact(sspp * sb).zip(dst.chunks_exact_mut(dch * ob)).take(n) {
        let (Some(s), Some(d)) = (s.get(si * sb..(si + 1) * sb), d.get_mut(di * ob..(di + 1) * ob)) else { continue };
        match src {
            Src::U32 => {
                let v = s.try_into().map_or(0, u32::from_ne_bytes);
                d.copy_from_slice(&((v >> 16) as u16).to_ne_bytes());
            }
            Src::U64 => {
                let v = s.try_into().map_or(0, u64::from_ne_bytes);
                d.copy_from_slice(&((v >> 48) as u16).to_ne_bytes());
            }
            Src::F64 => {
                let v = s.try_into().map_or(0.0, f64::from_ne_bytes);
                d.copy_from_slice(&(v as f32).to_ne_bytes());
            }
            _ => d.copy_from_slice(s),
        }
    }
}

/// Applies what the stored samples leave to the reader: WhiteIsZero inversion of the gray
/// channel, and turning associated (premultiplied) alpha into straight alpha.
fn finish_band(lay: &Layout, band: &mut [u8]) {
    if !lay.invert && !lay.premultiplied {
        return;
    }
    let ch = lay.out_channels();
    let ob = lay.out.bytes();
    let color = lay.out_layout.color_channels();
    for p in band.chunks_exact_mut(ch * ob) {
        match lay.out {
            SampleType::U8 => {
                if lay.invert
                    && let Some(g) = p.first_mut()
                {
                    *g = 255 - *g;
                }
                if lay.premultiplied {
                    let a = u32::from(p.get(ch - 1).copied().unwrap_or(255));
                    for c in p.iter_mut().take(color) {
                        *c = (u32::from(*c) * 255 + a / 2).checked_div(a).unwrap_or(0).min(255) as u8;
                    }
                }
            }
            SampleType::U16 => {
                let get = |p: &[u8], i: usize| p.get(i * 2..i * 2 + 2).and_then(|s| s.try_into().ok()).map_or(0, u16::from_ne_bytes);
                let a = u32::from(get(p, ch - 1));
                for i in 0..color {
                    let mut v = u32::from(get(p, i));
                    if lay.invert && i == 0 {
                        v = 65535 - v;
                    }
                    if lay.premultiplied {
                        v = (v * 65535 + a / 2).checked_div(a).unwrap_or(0).min(65535);
                    }
                    if let Some(s) = p.get_mut(i * 2..i * 2 + 2) {
                        s.copy_from_slice(&(v as u16).to_ne_bytes());
                    }
                }
            }
            SampleType::F16 | SampleType::F32 => {
                let get = |p: &[u8], i: usize| -> f32 {
                    if ob == 2 {
                        p.get(i * 2..i * 2 + 2).and_then(|s| s.try_into().ok()).map_or(0.0, |s| f16::from_bits(u16::from_ne_bytes(s)).to_f32())
                    } else {
                        p.get(i * 4..i * 4 + 4).and_then(|s| s.try_into().ok()).map_or(0.0, f32::from_ne_bytes)
                    }
                };
                let a = get(p, ch - 1);
                for i in 0..color {
                    let mut v = get(p, i);
                    if lay.invert && i == 0 {
                        v = 1.0 - v;
                    }
                    if lay.premultiplied && a > 0.0 {
                        v /= a;
                    }
                    if ob == 2 {
                        if let Some(s) = p.get_mut(i * 2..i * 2 + 2) {
                            s.copy_from_slice(&f16::from_f32(v).to_bits().to_ne_bytes());
                        }
                    } else if let Some(s) = p.get_mut(i * 4..i * 4 + 4) {
                        s.copy_from_slice(&v.to_ne_bytes());
                    }
                }
            }
        }
    }
}
