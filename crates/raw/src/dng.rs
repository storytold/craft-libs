//! DNG (Adobe Digital Negative Specification 1.7): raw IFD selection, pixel data (via [`crate::tiffraw`]),
//! linearization, black/white levels, active area, default crop, CFA description, colour tags, opcode lists.

use crate::gaintable::GainTableMap;
use crate::profile::{HsvTable, ProfileLook, ToneCurve};
use crate::tiffraw::{Packing, read_image_in};
use crate::{
    BlackLevel, Camera, Cfa, CinemaTags, ColorData, Limits, Matrix3, Mode, Orientation, RawData, RawError, RawFormat, RawImage, Rect, Result,
    Timecode, opcodes,
};
use craft_tiff::tags::{self as t, photometric};
use craft_tiff::{ByteOrder, Ifd, Tiff, Value};

/// The main raw IFD: full-resolution (NewSubfileType 0) CFA or LinearRaw image with the most pixels.
pub(crate) fn raw_ifd(tiff: &Tiff) -> Option<&Ifd> {
    tiff.all_ifds()
        .into_iter()
        .filter(|i| i.u32(t::NEW_SUBFILE_TYPE).unwrap_or(0) == 0)
        .filter(|i| matches!(i.u16(t::PHOTOMETRIC), Some(photometric::CFA) | Some(photometric::LINEAR_RAW)))
        .max_by_key(|i| i.u64(t::IMAGE_WIDTH).unwrap_or(0).saturating_mul(i.u64(t::IMAGE_LENGTH).unwrap_or(0)))
}

/// Undo the DNG 1.7 row/column interleave, which reorders the stored raster without changing its
/// size: with `RowInterleaveFactor = n` the rows are stored as `n` consecutive fields, field `g`
/// holding rows `g, g+n, g+2n, …` of the image (and likewise for columns).
///
/// The tag counts fields, not a divisor of the raster — see [`field_map`] — so a dimension the
/// factor does not divide is decoded, with the trailing fields short, rather than refused.
///
/// Left alone, a file like this decodes as `n × m` scaled-down copies of the scene, one per
/// field, with the CFA pattern then read across permuted samples — so it is not merely misplaced,
/// it is the wrong colour as well (`RowInterleaveFactor = ColumnInterleaveFactor = 2` is what DNG
/// 1.7 JPEG XL files from Adobe's converter carry).
fn deinterleave(data: &mut RawData, w: usize, h: usize, cpp: usize, rows: usize, cols: usize) -> Result<()> {
    if rows <= 1 && cols <= 1 {
        return Ok(());
    }
    // The spec sets no bound on the value (it is only "the number of interleaved fields"); this
    // caps the field table we build and the permutation, and rejects a hostile tag.
    if !(1..=16).contains(&rows) || !(1..=16).contains(&cols) {
        return Err(RawError::Unsupported(format!("interleave factors {rows} × {cols}")));
    }
    let (map_r, map_c) = (field_map(h, rows), field_map(w, cols));
    let dest = |r: usize, c: usize| map_r[r] * w + map_c[c];
    match data {
        RawData::U16(v) => permute(v, w, h, cpp, dest),
        RawData::F32(v) => permute(v, w, h, cpp, dest),
    }
}

/// The stored-line → image-line map for one axis: a factor of `n` splits `len` lines into `n`
/// interleaved fields, field `g` holding lines `g, g+n, g+2n, …`. The fields are stored
/// consecutively, so field `g` starts at stored line `offs[g]` and its `k`-th line is image line
/// `g + n*k`. Field sizes follow from the round-robin split — field `g` has `ceil((len - g) / n)`
/// lines — so when `n` does not divide `len` the trailing fields are short, and when `n` exceeds
/// `len` the trailing fields are empty.
fn field_map(len: usize, n: usize) -> Vec<usize> {
    let mut offs = Vec::with_capacity(n + 1);
    let mut acc = 0usize;
    for g in 0..=n {
        offs.push(acc);
        if g < n && g < len {
            acc += (len - 1 - g) / n + 1;
        }
    }
    let mut map = vec![0usize; len];
    for g in 0..n {
        for (k, i) in (offs[g]..offs[g + 1]).enumerate() {
            map[i] = g + n * k;
        }
    }
    map
}

/// Move every `cpp`-sample pixel of `v` from its stored position to `dest(r, c)`, the image
/// position it belongs at. The permutation is not its own inverse, so it needs a destination.
fn permute<T: Copy + Default, F: Fn(usize, usize) -> usize>(v: &mut Vec<T>, w: usize, h: usize, cpp: usize, dest: F) -> Result<()> {
    let n = w.checked_mul(h).and_then(|n| n.checked_mul(cpp)).ok_or_else(|| RawError::Corrupt("image too large".into()))?;
    if v.len() != n {
        return Err(RawError::Corrupt("mosaic size does not match the image".into()));
    }
    let mut out = vec![T::default(); n];
    for r in 0..h {
        for c in 0..w {
            let (from, to) = ((r * w + c) * cpp, dest(r, c) * cpp);
            out[to..to + cpp].copy_from_slice(&v[from..from + cpp]);
        }
    }
    *v = out;
    Ok(())
}

/// A 3×3 matrix tag: nine finite values that can be inverted.
fn mat3(v: Option<Vec<f64>>) -> Option<Matrix3> {
    let v = v?;
    if v.len() != 9 || v.iter().any(|x| !x.is_finite()) {
        return None;
    }
    let m = [[v[0], v[1], v[2]], [v[3], v[4], v[5]], [v[6], v[7], v[8]]];
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    (det.abs() > 1e-12).then_some(m)
}

fn vec3(v: Option<Vec<f64>>) -> Option<[f64; 3]> {
    let v = v?;
    (v.len() == 3 && v.iter().all(|x| x.is_finite() && *x > 0.0)).then(|| [v[0], v[1], v[2]])
}

/// Colour tags: DNG puts them in IFD0, but some writers use the raw IFD; prefer the raw IFD.
/// `order` is the file's byte order (for the binary gain-table-map tags).
pub(crate) fn color_data(ifd0: &Ifd, raw: &Ifd, order: ByteOrder) -> ColorData {
    let get = |tag: u16| raw.f64s(tag).or_else(|| ifd0.f64s(tag));
    let geti = |tag: u16| raw.u16(tag).or_else(|| ifd0.u16(tag));
    ColorData {
        illuminant: [geti(t::CALIBRATION_ILLUMINANT_1).unwrap_or(0), geti(t::CALIBRATION_ILLUMINANT_2).unwrap_or(0)],
        color_matrix: [mat3(get(t::COLOR_MATRIX_1)), mat3(get(t::COLOR_MATRIX_2))],
        forward_matrix: [mat3(get(t::FORWARD_MATRIX_1)), mat3(get(t::FORWARD_MATRIX_2))],
        camera_calibration: [mat3(get(t::CAMERA_CALIBRATION_1)), mat3(get(t::CAMERA_CALIBRATION_2))],
        analog_balance: vec3(get(t::ANALOG_BALANCE)),
        as_shot_neutral: vec3(get(t::AS_SHOT_NEUTRAL)),
        as_shot_white_xy: get(t::AS_SHOT_WHITE_XY).filter(|v| v.len() == 2 && v.iter().all(|c| *c > 0.0 && *c < 1.0)).map(|v| [v[0], v[1]]),
        baseline_exposure: get(t::BASELINE_EXPOSURE).and_then(|v| v.first().copied()).filter(|v| v.is_finite()).unwrap_or(0.0)
            + get(t::BASELINE_EXPOSURE_OFFSET).and_then(|v| v.first().copied()).filter(|v| v.is_finite()).unwrap_or(0.0),
        // DNG 1.7 BaselineSharpness (IFD 0 or the enhanced IFD); implausible values are ignored
        baseline_sharpness: get(t::BASELINE_SHARPNESS).and_then(|v| v.first().copied()).filter(|v| v.is_finite() && *v > 0.0 && *v <= 16.0),
        profile: profile_look(ifd0, raw, order),
        kelvin_presets: None,
    }
}

/// The profile look tags (`ProfileHueSatMap*`, `ProfileLookTable*`, `ProfileToneCurve`,
/// `ProfileGainTableMap*`); malformed ones are ignored. Like the colour tags, read from the raw IFD
/// first, else IFD 0.
pub(crate) fn profile_look(ifd0: &Ifd, raw: &Ifd, order: ByteOrder) -> ProfileLook {
    let pick = |tag: u16| if raw.contains(tag) { raw } else { ifd0 };
    let table = |dims: u16, data: u16, enc: u16| {
        let ifd = pick(dims);
        let dims = ifd.u64s(dims)?;
        let data = ifd.f64s(data)?;
        HsvTable::from_tags(&dims, &data, ifd.u64s(enc).and_then(|v| v.first().copied()).unwrap_or(0))
    };
    ProfileLook {
        hue_sat_map: [
            table(t::PROFILE_HUE_SAT_MAP_DIMS, t::PROFILE_HUE_SAT_MAP_DATA_1, t::PROFILE_HUE_SAT_MAP_ENCODING),
            table(t::PROFILE_HUE_SAT_MAP_DIMS, t::PROFILE_HUE_SAT_MAP_DATA_2, t::PROFILE_HUE_SAT_MAP_ENCODING),
        ],
        look_table: table(t::PROFILE_LOOK_TABLE_DIMS, t::PROFILE_LOOK_TABLE_DATA, t::PROFILE_LOOK_TABLE_ENCODING),
        tone_curve: pick(t::PROFILE_TONE_CURVE).f64s(t::PROFILE_TONE_CURVE).and_then(|v| ToneCurve::from_tag(&v)),
        gain_table_map: gain_table_map(ifd0, raw, order),
    }
}

/// The gain table map (DNG 1.7 precedence): `ProfileGainTableMap2` from IFD 0 (the embedded
/// camera profile), else from the raw IFD; else `ProfileGainTableMap` from the raw IFD (where DNG
/// 1.6 put it, and where Apple writes it), else from IFD 0 (where DNG 1.7 says it was meant to be).
/// A malformed tag falls through to the next one.
fn gain_table_map(ifd0: &Ifd, raw: &Ifd, order: ByteOrder) -> Option<GainTableMap> {
    let parse = |ifd: &Ifd, tag: u16| ifd.bytes(tag).and_then(|b| GainTableMap::parse(b, order, tag == t::PROFILE_GAIN_TABLE_MAP_2));
    parse(ifd0, t::PROFILE_GAIN_TABLE_MAP_2)
        .or_else(|| parse(raw, t::PROFILE_GAIN_TABLE_MAP_2))
        .or_else(|| parse(raw, t::PROFILE_GAIN_TABLE_MAP))
        .or_else(|| parse(ifd0, t::PROFILE_GAIN_TABLE_MAP))
}

pub(crate) fn decode(bytes: &[u8], mode: Mode, limits: &Limits) -> Result<RawImage> {
    decode_as(bytes, mode, RawFormat::Dng, limits)
}

/// Whether a TIFF that is not a DNG describes a raw image the way DNG does: a full-resolution CFA IFD
/// and DNG's own statement of the white balance (`AsShotNeutral`, in IFD0 or the raw IFD), in a coding
/// [`decode_as`] reads: uncompressed, or lossless JPEG whose scan header the lossless decoder accepts.
/// Such files (Hasselblad 3FR and FFF) carry no DNG version tag, so they are not DNGs, but nothing else
/// in them needs a maker-specific reader. Without the white-balance tag nothing in the file says how
/// to read its colour, and the file stays an undecodable raw.
pub(crate) fn is_plain_cfa_tiff(tiff: &Tiff, bytes: &[u8]) -> bool {
    let Some(raw) = raw_ifd(tiff) else { return false };
    if !raw.contains(t::AS_SHOT_NEUTRAL) && !tiff.ifds.first().is_some_and(|i| i.contains(t::AS_SHOT_NEUTRAL)) {
        return false;
    }
    let Ok(info) = raw.image() else { return false };
    if info.samples_per_pixel != 1 || !(1..=16).contains(&info.bits()) || info.sample_format == 3 {
        return false;
    }
    match info.compression {
        t::compression::NONE => true,
        t::compression::JPEG => info
            .chunks(bytes.len() as u64)
            .first()
            .and_then(|c| craft_tiff::image::chunk_bytes(bytes, c))
            .is_some_and(|src| crate::ljpeg::frame_info(src).is_ok()),
        _ => false,
    }
}

/// [`decode`] for the TIFF-based raws that follow the DNG layout without being DNGs (`format` says
/// which). They differ in one respect: when the raw IFD states no `CFAPattern`, the layout is RGGB
/// anchored at the sensor origin.
pub(crate) fn decode_as(bytes: &[u8], mode: Mode, format: RawFormat, limits: &Limits) -> Result<RawImage> {
    let tiff = Tiff::parse(bytes)?;
    let raw = raw_ifd(&tiff).ok_or_else(|| RawError::Corrupt("DNG without a raw image IFD".into()))?;
    decode_ifd(bytes, &tiff, raw, raw, mode, format, limits)
}

/// The smallest reduced-resolution copy of a DNG's LinearRaw image whose default crop still has a long edge of
/// `min_long` px: Adobe's merges keep 256, 512 and 2048 px copies beside the full image of its HDR and panorama
/// merges, so a thumbnail decodes a 512 px copy instead of a 24k × 7k image. Only a copy that the main
/// image's tags describe as well: same samples per pixel and aspect ratio (within 1 %), and a main image without
/// opcode lists, a linearization table or black-level deltas (all in full-size pixel coordinates).
fn reduced_ifd<'a>(tiff: &'a Tiff, main: &Ifd, min_long: usize) -> Option<&'a Ifd> {
    if main.u16(t::PHOTOMETRIC) != Some(photometric::LINEAR_RAW) {
        return None;
    }
    let full_size_tags =
        [t::OPCODE_LIST_1, t::OPCODE_LIST_2, t::OPCODE_LIST_3, t::LINEARIZATION_TABLE, t::BLACK_LEVEL_DELTA_H, t::BLACK_LEVEL_DELTA_V];
    if full_size_tags.iter().any(|&tag| main.contains(tag)) {
        return None;
    }
    let (mw, mh) = (main.u64(t::IMAGE_WIDTH)?, main.u64(t::IMAGE_LENGTH)?);
    if mw == 0 || mh == 0 {
        return None;
    }
    let crop_long = match main.f64s(t::DEFAULT_CROP_SIZE).as_deref() {
        Some([w, h]) if w.is_finite() && h.is_finite() && *w >= 1.0 && *h >= 1.0 => w.max(*h).min(mw.max(mh) as f64),
        _ => mw.max(mh) as f64,
    };
    let cpp = main.u16(t::SAMPLES_PER_PIXEL).unwrap_or(1);
    tiff.all_ifds()
        .into_iter()
        .filter(|i| i.u32(t::NEW_SUBFILE_TYPE) == Some(1) && i.u16(t::PHOTOMETRIC) == Some(photometric::LINEAR_RAW))
        .filter(|i| i.u16(t::SAMPLES_PER_PIXEL).unwrap_or(1) == cpp)
        .filter_map(|i| {
            let (w, h) = (i.u64(t::IMAGE_WIDTH)?, i.u64(t::IMAGE_LENGTH)?);
            let (a, b) = (w.checked_mul(mh)?, h.checked_mul(mw)?);
            let same_aspect = w > 0 && h > 0 && a.abs_diff(b).saturating_mul(100) <= a;
            let covers = crop_long * w as f64 / mw as f64 >= min_long as f64;
            (same_aspect && covers && w < mw && h < mh).then_some((i, w))
        })
        .min_by_key(|&(_, w)| w)
        .map(|(i, _)| i)
}

/// [`decode`] from the smallest reduced-resolution copy of the raw image that covers `min_long` ([`reduced_ifd`]):
/// the main image's levels, colour and metadata, its active area and default crop scaled to the copy. `None` when the
/// file keeps no such copy.
pub(crate) fn decode_reduced(bytes: &[u8], min_long: usize, limits: &Limits) -> Result<Option<RawImage>> {
    let tiff = Tiff::parse(bytes)?;
    let Some(main) = raw_ifd(&tiff) else { return Ok(None) };
    let Some(small) = reduced_ifd(&tiff, main, min_long) else { return Ok(None) };
    decode_ifd(bytes, &tiff, small, main, Mode::Full, RawFormat::Dng, limits).map(Some)
}

/// [`decode_as`] of the samples in `pixels`, described by the raw IFD `main`: the same IFD, or (see
/// [`decode_reduced`]) a reduced-resolution copy of it, whose own level tags win and whose geometry is `main`'s
/// scaled to its size.
fn decode_ifd(bytes: &[u8], tiff: &Tiff, pixels: &Ifd, main: &Ifd, mode: Mode, format: RawFormat, limits: &Limits) -> Result<RawImage> {
    let ifd0 = tiff.ifds.first().ok_or_else(|| RawError::Corrupt("TIFF without an IFD".into()))?;
    let raw = main;
    // GoPro's GPR: a DNG whose raw data is VC-5 coded, which is not decoded (yet). Named here, before
    // the generic "TIFF compression 9" of the chunk decoder. Header mode still describes the file
    // (a GPR has no embedded preview, and a library keeps the photo with this reason shown, as it
    // does for any file whose pixels fail to decode, rather than refusing to import it).
    if mode == Mode::Full
        && let Some(c) = pixels.u16(t::COMPRESSION).filter(|&c| c == t::compression::VC5)
    {
        return Err(RawError::Unsupported(format!("DNG compression {c} (GoPro VC-5) is not decoded yet")));
    }
    let info = pixels.image()?;
    let (w, h, cpp) = (info.width as usize, info.height as usize, info.samples_per_pixel as usize);
    // one sample (CFA or monochrome), RGB or four colours; two are neither and can't be developed (storytold/lightcraft#1511)
    if !matches!(cpp, 1 | 3 | 4) {
        return Err(RawError::Unsupported(format!("{cpp} samples per pixel")));
    }
    let bits = info.bits() as u32;
    // DNG 1.4 stores lossy JPEG at 8 bits per sample: the tiles are 8-bit baseline JPEGs whatever the tag says,
    // and reading them as a deeper sample would put the picture at the bottom of its range
    if info.compression == t::compression::LOSSY_JPEG && bits != 8 {
        return Err(RawError::Unsupported(format!("lossy JPEG DNG with {bits}-bit samples (DNG allows only 8)")));
    }
    let mut data = read_image_in(mode, bytes, &info, tiff.order, Packing::Msb, limits)?;

    // DNG 1.7 interleaved storage: the raster is a permutation of the mosaic, so undo it before
    // anything reads a sample as a pixel.
    if mode == Mode::Full {
        let factor = |tag| pixels.u16(tag).map(usize::from).unwrap_or(1);
        deinterleave(&mut data, w, h, cpp, factor(t::ROW_INTERLEAVE_FACTOR), factor(t::COLUMN_INTERLEAVE_FACTOR))?;
    }

    // linearization table (integer data only)
    let mut linearized = false;
    // the largest value the table can give a sample: the white level when the file states none
    let mut table_max = None;
    if let (RawData::U16(v), Some(table)) = (&mut data, pixels.u64s(t::LINEARIZATION_TABLE).filter(|t| !t.is_empty())) {
        let table: Vec<u16> = table.iter().map(|&x| x.min(65535) as u16).collect();
        let last = table.len() - 1;
        v.iter_mut().for_each(|s| *s = table[(*s as usize).min(last)]);
        linearized = true;
        table_max = table.iter().copied().max().filter(|&m| m > 0);
    }

    // a reduced copy's own levels, where it states them
    let levels = |tag| if pixels.contains(tag) { pixels } else { raw };
    let float = matches!(data, RawData::F32(_));
    // WhiteLevel is in linearized values (DNG chapter 5 linearizes before it rescales), so without the tag a
    // linearized image's white is the table's largest output, not the stored depth's (PhotoCraft's reading: an
    // 8-bit file whose table expands to 16 bits would otherwise clip at 255)
    let default_white = match table_max {
        Some(max) => f32::from(max),
        None if float => 1.0,
        None => ((1u64 << bits.min(16)) - 1) as f32,
    };
    // (a DOUBLE past `f32::MAX` casts to infinity, which would black the picture out: ignored like a zero one)
    let mut white: Vec<f32> =
        levels(t::WHITE_LEVEL).f64s(t::WHITE_LEVEL).unwrap_or_default().into_iter().map(|v| v as f32).filter(|v| v.is_finite() && *v > 0.0).collect();
    if white.is_empty() {
        white = vec![default_white];
    }
    // No sensor records NaN or infinity, so such a float sample is damage. Left in, it turns into NaN in demosaic
    // arithmetic and spreads; NaN and −∞ read as black (0), +∞ as the white level (a clipped highlight). Finite
    // samples, negative or above white included, are kept as stored.
    if let RawData::F32(v) = &mut data {
        for (i, s) in v.iter_mut().enumerate() {
            if !s.is_finite() {
                *s = if *s > 0.0 { white.get(i % cpp).or(white.first()).copied().unwrap_or(default_white) } else { 0.0 };
            }
        }
    }

    let dim = levels(t::BLACK_LEVEL).u64s(t::BLACK_LEVEL_REPEAT_DIM).unwrap_or_default();
    let (br, bc) = match dim.as_slice() {
        [r, c] if (1..=16).contains(r) && (1..=16).contains(c) => (*r as usize, *c as usize),
        _ => (1, 1),
    };
    // (a FLOAT / DOUBLE tag can hold NaN or infinity: such a level reads as 0, in its place)
    let level = |v: f64| Some(v as f32).filter(|v| v.is_finite()).unwrap_or(0.0);
    let mut values: Vec<f32> = levels(t::BLACK_LEVEL).f64s(t::BLACK_LEVEL).unwrap_or_default().into_iter().map(level).collect();
    // A TIFF that is not a DNG may state four black levels without `BlackLevelRepeatDim`: one per site of its 2 × 2
    // CFA, row-major (PhotoCraft's reading of Hasselblad files). In a DNG the count has to match the repeat.
    let (br, bc) = if format != RawFormat::Dng && cpp == 1 && values.len() == 4 && !levels(t::BLACK_LEVEL).contains(t::BLACK_LEVEL_REPEAT_DIM) {
        (2, 2)
    } else {
        (br, bc)
    };
    if values.len() != br * bc * cpp {
        values = vec![values.first().copied().unwrap_or(0.0)];
    }
    let (br, bc) = if values.len() == 1 { (1, 1) } else { (br, bc) };
    let floats = |tag| raw.f64s(tag).unwrap_or_default().into_iter().map(level).collect::<Vec<f32>>();
    let black =
        BlackLevel { repeat_rows: br, repeat_cols: bc, values, delta_h: floats(t::BLACK_LEVEL_DELTA_H), delta_v: floats(t::BLACK_LEVEL_DELTA_V) };

    // geometry in the main image's pixels (a reduced copy's is scaled below)
    let (full_w, full_h) = if std::ptr::eq(pixels, raw) {
        (w, h)
    } else {
        (raw.u64(t::IMAGE_WIDTH).unwrap_or(0) as usize, raw.u64(t::IMAGE_LENGTH).unwrap_or(0) as usize)
    };
    if full_w == 0 || full_h == 0 {
        return Err(RawError::Corrupt("raw image IFD without a size".into()));
    }
    let active_area = match raw.u64s(t::ACTIVE_AREA).as_deref() {
        Some([top, left, bottom, right]) if bottom > top && right > left && (*bottom as usize) <= full_h && (*right as usize) <= full_w => {
            Rect::new(*left as usize, *top as usize, (*right - *left) as usize, (*bottom - *top) as usize)
        }
        _ => Rect::new(0, 0, full_w, full_h),
    };
    let crop = default_crop(raw, active_area);
    let (active_area, crop) = if (full_w, full_h) == (w, h) {
        (active_area, crop)
    } else {
        let (sx, sy) = (w as f64 / full_w as f64, h as f64 / full_h as f64);
        let scale = |r: Rect| {
            let (x, y) = ((r.x as f64 * sx).round() as usize, (r.y as f64 * sy).round() as usize);
            let (x1, y1) = (((r.x + r.width) as f64 * sx).round() as usize, ((r.y + r.height) as f64 * sy).round() as usize);
            Rect::new(x, y, x1.saturating_sub(x).max(1), y1.saturating_sub(y).max(1))
        };
        let area = scale(active_area).clipped(w, h);
        (area, scale(crop).clipped(area.width, area.height))
    };

    let cfa = if info.photometric == photometric::CFA {
        if raw.u16(t::CFA_LAYOUT).unwrap_or(1) != 1 {
            return Err(RawError::Unsupported("non-rectangular CFA layout".into()));
        }
        let dim = raw.u64s(t::CFA_REPEAT_PATTERN_DIM).unwrap_or_else(|| vec![2, 2]);
        let (rows, cols) = match dim.as_slice() {
            [r, c] if (1..=16).contains(r) && (1..=16).contains(c) => (*r as usize, *c as usize),
            _ => return Err(RawError::Corrupt("bad CFARepeatPatternDim".into())),
        };
        // a TIFF that is not a DNG may leave the layout out: RGGB from the sensor origin
        let assumed = [0u8, 1, 1, 2];
        let pat = match raw.bytes(t::CFA_PATTERN_EP) {
            Some(p) => p,
            None if format != RawFormat::Dng && (rows, cols) == (2, 2) => &assumed,
            None => return Err(RawError::Corrupt("missing CFAPattern".into())),
        };
        if pat.len() != rows * cols {
            return Err(RawError::Corrupt("CFAPattern size mismatch".into()));
        }
        let planes = raw.bytes(t::CFA_PLANE_COLOR).map(|b| b.to_vec()).unwrap_or_else(|| vec![0, 1, 2]);
        let pattern: Vec<u8> = pat
            .iter()
            .map(|c| planes.iter().position(|p| p == c).map(|i| i as u8).filter(|&i| i < 3))
            .collect::<Option<_>>()
            .ok_or_else(|| RawError::Unsupported("CFA with more than three colours".into()))?;
        if cpp != 1 {
            return Err(RawError::Unsupported("CFA data with several samples per pixel".into()));
        }
        // the tag is anchored at the top-left of the ActiveArea (DNG spec); `RawImage::cfa` is anchored at the
        // sensor origin (the crop to the active area shifts it back), so move the anchor out by the area's offset
        // (DNGs that LightCraft wrote before its writer followed the spec carry the sensor-origin pattern; they are marked
        // by a bare "LightCraft" Software tag and keep their original reading)
        let at_origin = Cfa { width: cols, height: rows, pattern };
        let legacy_writer = [raw, ifd0].iter().any(|i| i.string(t::SOFTWARE).as_deref().map(str::trim) == Some("LightCraft"));
        Some(if legacy_writer { at_origin } else { at_origin.shifted(cols - active_area.x % cols, rows - active_area.y % rows) })
    } else {
        None
    };

    let opcode_list = |tag| raw.bytes(tag).map(opcodes::parse_list).unwrap_or_default();
    let opcodes =
        crate::OpcodeLists { list1: opcode_list(t::OPCODE_LIST_1), list2: opcode_list(t::OPCODE_LIST_2), list3: opcode_list(t::OPCODE_LIST_3) };

    let img = RawImage {
        format,
        width: w,
        height: h,
        cpp,
        data,
        cfa,
        bits,
        black,
        white,
        active_area,
        crop,
        orientation: Orientation::from_exif(ifd0.u16(t::ORIENTATION).unwrap_or(1)),
        color: color_data(ifd0, raw, tiff.order),
        wb_multipliers: None,
        linearized,
        pixel_aspect: default_scale_aspect(raw.f64s(t::DEFAULT_SCALE).as_deref()),
        opcodes,
        camera: Camera::from_ifd0(ifd0),
        cinema: cinema_tags(ifd0, raw),
        lens_metadata: Default::default(),
        level_sources: Default::default(),
    };
    img.validate_for(mode)?;
    Ok(img)
}

/// CinemaDNG `TimeCodes` (CinemaDNG Image Data Format Specification 1.0, tag 51043).
const TIME_CODES: u16 = 51043;
/// CinemaDNG `FrameRate` (tag 51044).
const FRAME_RATE: u16 = 51044;
/// CinemaDNG `ReelName` (tag 51081).
const REEL_NAME: u16 = 51081;

/// The CinemaDNG tags of a frame, from IFD 0 (where the specification puts them), else the raw IFD. A frame rate
/// outside 1/1000 to 1000 frames a second and a time code that is not valid BCD are ignored.
fn cinema_tags(ifd0: &Ifd, raw: &Ifd) -> CinemaTags {
    let pick = |tag: u16| if ifd0.contains(tag) { ifd0 } else { raw };
    let frame_rate = pick(FRAME_RATE).get(FRAME_RATE).and_then(|e| match &e.value {
        Value::SRational(v) => v.first().map(|&(n, d)| (i64::from(n), i64::from(d))),
        Value::Rational(v) => v.first().map(|&(n, d)| (i64::from(n), i64::from(d))),
        _ => None,
    });
    CinemaTags {
        frame_rate: frame_rate.filter(|&(n, d)| n > 0 && d > 0 && n <= 1000 * d && n * 1000 >= d),
        timecode: pick(TIME_CODES).bytes(TIME_CODES).and_then(Timecode::from_bytes),
        reel_name: pick(REEL_NAME).string(REEL_NAME).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()),
    }
}

/// The default crop, relative to the active area. Each tag has its own default (DNG 1.7.1, `DefaultCropOrigin`
/// and `DefaultCropSize`): the origin (0, 0), the size the whole area, so a file with only a size crops from the
/// area's corner. A tag that is present must hold two finite, non-negative values, and the crop must start inside
/// the area and be at least one pixel each way; otherwise the crop is the whole active area.
fn default_crop(raw: &Ifd, active_area: Rect) -> Rect {
    let whole = Rect::new(0, 0, active_area.width, active_area.height);
    let pair = |tag: u16, default: [f64; 2]| -> Option<[f64; 2]> {
        if !raw.contains(tag) {
            return Some(default);
        }
        match raw.f64s(tag).as_deref() {
            Some([a, b]) if a.is_finite() && b.is_finite() && *a >= 0.0 && *b >= 0.0 => Some([*a, *b]),
            _ => None,
        }
    };
    let (Some([x, y]), Some([cw, ch])) =
        (pair(t::DEFAULT_CROP_ORIGIN, [0.0, 0.0]), pair(t::DEFAULT_CROP_SIZE, [active_area.width as f64, active_area.height as f64]))
    else {
        return whole;
    };
    if cw < 1.0 || ch < 1.0 {
        return whole;
    }
    // (`as usize` saturates, and `clipped` holds the origin to the area)
    let crop =
        Rect::new(x.round() as usize, y.round() as usize, cw.round() as usize, ch.round() as usize).clipped(active_area.width, active_area.height);
    if crop.width == 0 || crop.height == 0 { whole } else { crop }
}

/// Pixel height over width from `DefaultScale` (horizontal, vertical): only a clear 1.5 to 4 ratio counts (the rows stretch; a wider-than-tall pixel is left alone).
fn default_scale_aspect(scale: Option<&[f64]>) -> f32 {
    match scale {
        Some([h, v]) if *h > 0.0 && *v > 0.0 && (1.5..=4.0).contains(&(v / h)) => (v / h) as f32,
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_scale_makes_tall_pixels() {
        assert_eq!(default_scale_aspect(Some(&[1.0, 2.0])), 2.0);
        assert_eq!(default_scale_aspect(Some(&[1.0, 1.0])), 1.0);
        assert_eq!(default_scale_aspect(Some(&[2.0, 1.0])), 1.0);
        assert_eq!(default_scale_aspect(Some(&[1.0, 1.2])), 1.0);
        assert_eq!(default_scale_aspect(None), 1.0);
    }

    use super::*;

    /// Store `image` the way DNG 1.7 interleaves it: row `r` of the image is written to stored row
    /// `(r % rows) * (h / rows) + r / rows`, and columns likewise. This is the inverse of
    /// [`deinterleave`]'s destination map for a raster the factor divides, i.e. what a writer
    /// produces (the short-trailing-field case is covered by the hand-written fixtures below).
    fn interleave(image: &[u16], w: usize, h: usize, rows: usize, cols: usize) -> Vec<u16> {
        let (gh, gw) = (h / rows, w / cols);
        let mut out = vec![0u16; w * h];
        for r in 0..h {
            for c in 0..w {
                out[((r % rows) * gh + r / rows) * w + (c % cols) * gw + c / cols] = image[r * w + c];
            }
        }
        out
    }

    fn samples(d: &RawData) -> Vec<u16> {
        match d {
            RawData::U16(v) => v.clone(),
            RawData::F32(_) => Vec::new(),
        }
    }

    /// The image `image` laid out in stored order, given the image line each stored row and column
    /// holds. Written out by hand from the tag's plain reading, so it does not share a formula with
    /// the code under test.
    fn store(image: &[u16], w: usize, rows_img: &[usize], cols_img: &[usize]) -> Vec<u16> {
        let mut out = vec![0u16; image.len()];
        for (sr, &ir) in rows_img.iter().enumerate() {
            for (sc, &ic) in cols_img.iter().enumerate() {
                out[sr * w + sc] = image[ir * w + ic];
            }
        }
        out
    }

    #[test]
    fn deinterleaves_an_interleaved_raster() {
        // 3×3 rather than 2×2 on purpose: the 2×2 permutation is its own inverse, so a 2×2 fixture
        // would pass even with the direction of the permutation reversed. This is the shape a DNG
        // 1.7 file caught in the wild carried (JPEG XL, `RowInterleaveFactor`/`ColumnInterleaveFactor`
        // both 2) — its real-file ground truth is that it renders bit-identical to its
        // non-interleaved twin.
        let (w, h, rows, cols) = (12, 9, 3, 3);
        let image: Vec<u16> = (0..w * h).map(|i| i as u16).collect();
        let mut data = RawData::U16(interleave(&image, w, h, rows, cols));
        assert_ne!(samples(&data), image, "the fixture has to actually be permuted");
        deinterleave(&mut data, w, h, 1, rows, cols).unwrap();
        assert_eq!(samples(&data), image);

        // 2×2 as well, the shape Adobe's converter writes.
        let (w2, h2) = (12, 8);
        let image2: Vec<u16> = (0..w2 * h2).map(|i| i as u16).collect();
        let mut two = RawData::U16(interleave(&image2, w2, h2, 2, 2));
        deinterleave(&mut two, w2, h2, 1, 2, 2).unwrap();
        assert_eq!(samples(&two), image2);
    }

    #[test]
    fn deinterleave_pins_the_field_direction() {
        // Hand-computed ground truth for the tag's plain reading ("field g holds lines g, g+n,
        // g+2n, …"). The 3×3 test above round-trips against a helper written from the same reading
        // of the tag, so it fixes consistency but not the direction; and a 2×2 fixture cannot tell
        // the two directions apart, being its own inverse. These maps are written out literally so
        // a reversed permutation fails here. 3 row fields of h = 9 start at stored rows 0, 3, 6 and
        // hold image rows [0,3,6,1,4,7,2,5,8]; 2 column fields of w = 4 give [0,2,1,3].
        let (w, h) = (4, 9);
        let image: Vec<u16> = (0..(w * h) as u16).collect();
        let mut data = RawData::U16(store(&image, w, &[0, 3, 6, 1, 4, 7, 2, 5, 8], &[0, 2, 1, 3]));
        assert_ne!(samples(&data), image, "the fixture has to actually be permuted");
        deinterleave(&mut data, w, h, 1, 3, 2).unwrap();
        assert_eq!(samples(&data), image);
    }

    #[test]
    fn deinterleaves_a_raster_the_factor_does_not_divide() {
        // The spec fixes the number of fields, not that the size is a multiple of it: h = 7 over 3
        // row fields is 3 + 2 + 2 (the round-robin split), so field 0 starts at stored row 0 holding
        // image rows 0, 3, 6; field 1 at stored row 3 holding 1, 4; field 2 at stored row 5 holding
        // 2, 5. Columns: w = 5 over 2 fields is 3 + 2 → image columns [0,2,4,1,3].
        let (w, h) = (5, 7);
        let image: Vec<u16> = (0..(w * h) as u16).collect();
        let mut data = RawData::U16(store(&image, w, &[0, 3, 6, 1, 4, 2, 5], &[0, 2, 4, 1, 3]));
        assert_ne!(samples(&data), image, "the fixture has to actually be permuted");
        deinterleave(&mut data, w, h, 1, 3, 2).unwrap();
        assert_eq!(samples(&data), image);
    }

    #[test]
    fn deinterleave_is_a_no_op_at_one_and_refuses_a_hostile_factor() {
        let image: Vec<u16> = (0..48).collect();
        let mut plain = RawData::U16(image.clone());
        deinterleave(&mut plain, 8, 6, 1, 1, 1).unwrap();
        assert_eq!(samples(&plain), image);

        // A hostile factor is an error, not a scrambled image.
        let mut hostile = RawData::U16(image);
        assert!(deinterleave(&mut hostile, 8, 6, 1, 1 << 20, 1).is_err());
    }
}
