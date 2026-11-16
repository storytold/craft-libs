//! Writing DNG files from a [`RawImage`] (uncompressed strips, lossless-JPEG tiles or Deflate tiles), used for DNG
//! export and for bit-exact decoder round-trip tests. Lossy DNG and DNGs of rendered pictures start from a developed
//! image, so the develop pipeline builds those on top of this writer. The raw image is stored in IFD0 (allowed by DNG 1.7 §"File Structure").

use crate::{Matrix3, RawData, RawError, RawImage, Result, ljpeg, opcodes};
use craft_tiff::tags::{self as t, compression, photometric};
use craft_tiff::writer::{rational, srational};
use craft_tiff::{ByteOrder, IfdBuilder, ImageData, TiffWriter, Value};
use rayon::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DngCompression {
    Uncompressed,
    /// Lossless JPEG tiles of `tile × tile` pixels (`tile` even). CFA tiles are encoded as two interleaved
    /// components of half the tile width (the common DNG layout), linear data with one component per sample.
    /// Edges above 2048 (after even alignment) are rejected before padding the tile.
    Lj92 {
        tile: u32,
    },
    /// Deflate (zlib) tiles of `tile × tile` pixels. Floating-point data is stored as 16-bit (`half`) or
    /// 32-bit IEEE floats with the floating-point predictor (TIFF predictor 3, Adobe TIFF Technical Note 3);
    /// integer data with horizontal differencing (predictor 2). 16-bit float is what HDR merges usually use.
    Deflate {
        tile: u32,
        half: bool,
    },
}

/// zlib level of Deflate tiles. On a 67 MP focus-stack merge (half floats) level 6 took 7× as long as level 1
/// (3.9 s vs 0.55 s on 4 cores) to save 3 % of the file (266 vs 275 MB); 5–12 % on raw and rendered DNGs.
const DEFLATE_LEVEL: u8 = 1;

/// Largest Deflate tile edge the writer pads to (`tile` is caller-supplied in [`DngCompression::Deflate`]):
/// square-tile buffers grow with the square, so an unclamped edge lets a request pad a tiny valid image into
/// a multi-gigabyte buffer and billions of edge-replicate iterations. 2048² samples (≤ 24 MB of f32) is far
/// above the 256 default and every real tile this writer makes.
const MAX_DEFLATE_TILE: u32 = 2048;

// Match the existing padded Deflate edge budget. JPEG's 16-bit dimension limit alone would permit
// a 65534²-sample allocation even for a tiny source image. At most four validated channels make
// this bound at most 32 MiB of u16 padding per tile, before encoder scratch/output.
const MAX_LJ92_TILE: usize = MAX_DEFLATE_TILE as usize;

/// `tile` aligned down to a multiple of 16 and clamped to [`MAX_DEFLATE_TILE`].
fn deflate_tile(tile: u32) -> usize {
    (tile.max(16) & !15).min(MAX_DEFLATE_TILE) as usize
}

/// IEEE 754 binary32 → binary16 (round to nearest even; overflow → ±inf, NaN kept).
pub fn f32_to_f16(v: f32) -> u16 {
    let b = v.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32;
    let man = b & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        // subnormal half
        let m = man | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = m >> shift;
        let rem = m & ((1 << shift) - 1);
        let mid = 1 << (shift - 1);
        let r = if rem > mid || (rem == mid && half & 1 == 1) { half + 1 } else { half };
        return sign | r as u16;
    }
    let half = ((e as u32) << 10) | (man >> 13);
    let rem = man & 0x1fff;
    let r = if rem > 0x1000 || (rem == 0x1000 && half & 1 == 1) { half + 1 } else { half };
    sign | r as u16
}

/// Apply the floating-point predictor (inverse of `unpack::undo_float_predictor`) to one row of
/// `n` big-endian samples of `bytes_per` bytes: split into byte planes (most significant first), then
/// byte-difference with `stride` samples.
fn float_predict(row: &[u8], n: usize, bytes_per: usize, stride: usize) -> Vec<u8> {
    let mut planes = vec![0u8; row.len()];
    for i in 0..n {
        for b in 0..bytes_per {
            planes[b * n + i] = row[i * bytes_per + b];
        }
    }
    for i in (stride..planes.len()).rev() {
        planes[i] = planes[i].wrapping_sub(planes[i - stride]);
    }
    planes
}

#[derive(Clone, Debug, PartialEq)]
pub struct DngWriteOptions {
    pub compression: DngCompression,
    pub order: ByteOrder,
    /// XMP packet to embed (tag 700); none when `None`.
    pub xmp: Option<String>,
    /// The `Software` tag: the writing application. It must not be the bare string `LightCraft`, which the reader
    /// takes for that app's early files (their `CFAPattern` is anchored at the sensor origin, not the ActiveArea).
    pub software: String,
}

impl Default for DngWriteOptions {
    fn default() -> Self {
        Self { compression: DngCompression::Lj92 { tile: 256 }, order: ByteOrder::Little, xmp: None, software: "craft-raw".into() }
    }
}

fn srat_mat(m: &Matrix3) -> Value {
    Value::SRational(m.iter().flatten().map(|&v| srational(v)).collect())
}

fn rat_vec(v: &[f64]) -> Value {
    Value::Rational(v.iter().map(|&x| rational(x)).collect())
}

/// Colour tags copied by value, with potentially large profile tables borrowed.
pub struct DngColor<'a> {
    pub illuminant: [u16; 2],
    pub color_matrix: [Option<Matrix3>; 2],
    pub forward_matrix: [Option<Matrix3>; 2],
    pub camera_calibration: [Option<Matrix3>; 2],
    pub analog_balance: Option<[f64; 3]>,
    pub as_shot_neutral: Option<[f64; 3]>,
    pub as_shot_white_xy: Option<[f64; 2]>,
    pub baseline_exposure: f64,
    pub baseline_sharpness: Option<f64>,
    pub profile: &'a crate::profile::ProfileLook,
}

impl<'a> From<&'a crate::ColorData> for DngColor<'a> {
    fn from(color: &'a crate::ColorData) -> Self {
        Self {
            illuminant: color.illuminant,
            color_matrix: color.color_matrix,
            forward_matrix: color.forward_matrix,
            camera_calibration: color.camera_calibration,
            analog_balance: color.analog_balance,
            as_shot_neutral: color.as_shot_neutral,
            as_shot_white_xy: color.as_shot_white_xy,
            baseline_exposure: color.baseline_exposure,
            baseline_sharpness: color.baseline_sharpness,
            profile: &color.profile,
        }
    }
}

/// Borrowed wire-format input for callers with their own application image type.
/// Pixel buffers and large profile/opcode tables are not copied.
pub struct DngImage<'a> {
    pub width: usize,
    pub height: usize,
    pub cpp: usize,
    pub data: &'a RawData,
    pub cfa: Option<&'a crate::Cfa>,
    pub bits: u32,
    pub black: &'a crate::BlackLevel,
    pub white: &'a [f32],
    pub active_area: crate::Rect,
    pub crop: crate::Rect,
    pub orientation: crate::Orientation,
    pub color: DngColor<'a>,
    pub wb_multipliers: Option<[f32; 3]>,
    pub opcodes: &'a crate::OpcodeLists,
    pub make: Option<&'a str>,
    pub model: Option<&'a str>,
    pub unique_model: Option<&'a str>,
    pub cinema: &'a crate::CinemaTags,
    /// Optional application-supplied UniqueCameraModel fallback when make/model are absent.
    pub unknown_camera_model: Option<&'a str>,
    /// Whether `data` has already been mapped through `linearization_table` (see
    /// [`RawImage::linearized`]).
    pub linearized: bool,
    /// A LinearizationTable still to be applied by the reader; written only when `linearized` is false.
    pub linearization_table: Option<&'a [u16]>,
}

impl<'a> From<&'a RawImage> for DngImage<'a> {
    fn from(raw: &'a RawImage) -> Self {
        Self {
            width: raw.width,
            height: raw.height,
            cpp: raw.cpp,
            data: &raw.data,
            cfa: raw.cfa.as_ref(),
            bits: raw.bits,
            black: &raw.black,
            white: &raw.white,
            active_area: raw.active_area,
            crop: raw.crop,
            orientation: raw.orientation,
            color: DngColor::from(&raw.color),
            wb_multipliers: raw.wb_multipliers,
            opcodes: &raw.opcodes,
            make: raw.camera.make.as_deref(),
            model: raw.camera.model.as_deref(),
            unique_model: raw.camera.unique_model.as_deref(),
            cinema: &raw.cinema,
            unknown_camera_model: None,
            linearized: raw.linearized,
            linearization_table: raw.linearization_table.as_deref(),
        }
    }
}

impl DngImage<'_> {
    fn validate(&self) -> Result<()> {
        crate::validate_image_layout(self.width, self.height, self.cpp, Some(self.data.len()), self.cfa, self.active_area)?;
        if self.width > u32::MAX as usize || self.height > u32::MAX as usize {
            return Err(RawError::Limit("DNG dimensions exceed TIFF fields"));
        }
        Ok(())
    }

    fn white_at(&self, sample: usize) -> f32 {
        self.white.get(sample).or_else(|| self.white.first()).copied().unwrap_or(65535.0)
    }
}

/// Write an image without taking ownership of its pixels or application metadata.
pub fn write_dng_view(raw: &DngImage<'_>, opts: &DngWriteOptions) -> Result<Vec<u8>> {
    write_image(raw, opts, None)
}

/// Encode prepared 8-bit RGB codes as lossy DNG JPEG tiles. Rendering, scaling and the
/// transfer curve are application policy; `linearization` supplies exactly 256 code values.
/// The callback receives one bounded RGB tile and must return its JPEG encoding or an error.
/// It can be invoked concurrently. The callback is a trusted encoder: it owns its output
/// allocation and JPEG-validity contract; this API does not impose an aggregate output budget.
/// Only the byte order, XMP and Software fields of `opts`
/// apply here; `tile` and `quality` select the compression instead of `opts.compression`.
pub fn write_lossy_dng(
    raw: &DngImage<'_>,
    opts: &DngWriteOptions,
    tile: u32,
    quality: u8,
    linearization: &[u16],
    encode: &(impl Fn(Vec<u8>, u32, u8) -> Result<Vec<u8>> + Sync),
) -> Result<Vec<u8>> {
    write_image(raw, opts, Some(LossyTiles { tile, quality, linearization, encode }))
}

type JpegTileEncoder<'a> = dyn Fn(Vec<u8>, u32, u8) -> Result<Vec<u8>> + Sync + 'a;
struct LossyTiles<'a> {
    tile: u32,
    quality: u8,
    linearization: &'a [u16],
    encode: &'a JpegTileEncoder<'a>,
}

fn lossy_tile_edge(tile: u32) -> usize {
    (tile.clamp(16, 4096) & !15) as usize
}

fn set_lossy_tiles(ifd: &mut IfdBuilder, raw: &DngImage<'_>, options: LossyTiles<'_>) -> Result<()> {
    let RawData::U16(samples) = raw.data else { return Err(RawError::Unsupported("lossy DNG requires prepared 8-bit RGB codes".into())) };
    if raw.cpp != 3 || raw.cfa.is_some() || options.linearization.len() != 256 || samples.iter().any(|&v| v > 255) {
        return Err(RawError::Unsupported("lossy DNG requires prepared 8-bit RGB codes and a 256-entry linearization table".into()));
    }
    // Preserve the existing profile-export limit: <=4096² RGB bytes per worker, never
    // a caller-supplied unbounded square. Existing codec output/worker budgets still apply.
    let tile = lossy_tile_edge(options.tile);
    let (across, down) = (raw.width.div_ceil(tile), raw.height.div_ceil(tile));
    let count = across.checked_mul(down).ok_or(RawError::Limit("too many DNG tiles"))?;
    let mut tiles = Vec::new();
    tiles.try_reserve_exact(count).map_err(|_| RawError::Limit("DNG tile table allocation failed"))?;
    tiles.resize_with(count, Vec::new);
    tiles.par_iter_mut().enumerate().try_for_each(|(i, output)| -> Result<()> {
        let (tx, ty) = (i % across * tile, i / across * tile);
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(tile * tile * 3).map_err(|_| RawError::Limit("DNG tile allocation failed"))?;
        for y in 0..tile {
            let sy = (ty + y).min(raw.height - 1);
            for x in 0..tile {
                let sx = (tx + x).min(raw.width - 1);
                pixels.extend(samples[(sy * raw.width + sx) * 3..][..3].iter().map(|&v| v as u8));
            }
        }
        *output = (options.encode)(pixels, tile as u32, options.quality.clamp(1, 100))?;
        Ok(())
    })?;
    ifd.set(t::BITS_PER_SAMPLE, Value::Short(vec![8; 3]));
    ifd.set(t::COMPRESSION, Value::Short(vec![compression::LOSSY_JPEG]));
    ifd.set(t::LINEARIZATION_TABLE, Value::Short(options.linearization.to_vec()));
    ifd.set_image(ImageData::Tiles { tile_width: tile as u32, tile_height: tile as u32, tiles });
    Ok(())
}

/// Serialise `raw` as a DNG.
pub fn write_dng(raw: &RawImage, opts: &DngWriteOptions) -> Result<Vec<u8>> {
    write_dng_view(&DngImage::from(raw), opts)
}

fn write_image(raw: &DngImage<'_>, opts: &DngWriteOptions, lossy: Option<LossyTiles<'_>>) -> Result<Vec<u8>> {
    raw.validate()?;
    if opts.software.trim() == "LightCraft" {
        return Err(RawError::Unsupported("the Software tag `LightCraft` marks files with a sensor-anchored CFAPattern".into()));
    }
    let (w, h, cpp) = (raw.width, raw.height, raw.cpp);
    let mut ifd = IfdBuilder::new();
    ifd.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![0]));
    ifd.set(t::IMAGE_WIDTH, Value::Long(vec![w as u32]));
    ifd.set(t::IMAGE_LENGTH, Value::Long(vec![h as u32]));
    ifd.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![cpp as u16]));
    ifd.set(t::PLANAR_CONFIGURATION, Value::Short(vec![1]));
    ifd.set(t::PHOTOMETRIC, Value::Short(vec![if raw.cfa.is_some() { photometric::CFA } else { photometric::LINEAR_RAW }]));
    ifd.set(t::DNG_VERSION, Value::Byte(vec![1, 4, 0, 0]));
    ifd.set(t::DNG_BACKWARD_VERSION, Value::Byte(vec![1, 3, 0, 0]));
    write_identity_tags(&mut ifd, raw, opts);

    if let Some(cfa) = raw.cfa {
        ifd.set(t::CFA_REPEAT_PATTERN_DIM, Value::Short(vec![cfa.height as u16, cfa.width as u16]));
        // the tag is anchored at the ActiveArea's top-left, `raw.cfa` at the sensor origin
        let a = raw.active_area;
        ifd.set(t::CFA_PATTERN_EP, Value::Byte(cfa.shifted(a.x, a.y).pattern));
        ifd.set(t::CFA_PLANE_COLOR, Value::Byte(vec![0, 1, 2]));
        ifd.set(t::CFA_LAYOUT, Value::Short(vec![1]));
    }
    let b = raw.black;
    ifd.set(t::BLACK_LEVEL_REPEAT_DIM, Value::Short(vec![b.repeat_rows as u16, b.repeat_cols as u16]));
    let mut bl: Vec<f64> = b.values.iter().map(|&v| v as f64).collect();
    if bl.len() == 1 && cpp > 1 {
        bl = vec![bl[0]; cpp];
    }
    ifd.set(t::BLACK_LEVEL, rat_vec(&bl));
    if !b.delta_h.is_empty() {
        ifd.set(t::BLACK_LEVEL_DELTA_H, Value::SRational(b.delta_h.iter().map(|&v| srational(v as f64)).collect()));
    }
    if !b.delta_v.is_empty() {
        ifd.set(t::BLACK_LEVEL_DELTA_V, Value::SRational(b.delta_v.iter().map(|&v| srational(v as f64)).collect()));
    }
    let white: Vec<u32> = (0..cpp).map(|s| raw.white_at(s).round().clamp(0.0, u32::MAX as f32) as u32).collect();
    let max_white = white.iter().copied().max().unwrap_or(0);
    if matches!(raw.data, RawData::F32(_)) {
        ifd.set(t::WHITE_LEVEL, rat_vec(&(0..cpp).map(|s| raw.white_at(s) as f64).collect::<Vec<_>>()));
    } else {
        ifd.set(t::WHITE_LEVEL, Value::Long(white));
    }
    // A table still to be applied (OpcodeList1 has to run on the stored codes first) is written
    // with the samples. Once the samples have been mapped, writing the tag would apply it twice
    // on the next read of a file without list 1.
    if !raw.linearized
        && let Some(table) = raw.linearization_table.filter(|t| !t.is_empty())
    {
        ifd.set(t::LINEARIZATION_TABLE, Value::Short(table.to_vec()));
    }
    let a = raw.active_area;
    ifd.set(t::ACTIVE_AREA, Value::Long(vec![a.y as u32, a.x as u32, (a.y + a.height) as u32, (a.x + a.width) as u32]));
    ifd.set(t::DEFAULT_CROP_ORIGIN, Value::Long(vec![raw.crop.x as u32, raw.crop.y as u32]));
    ifd.set(t::DEFAULT_CROP_SIZE, Value::Long(vec![raw.crop.width as u32, raw.crop.height as u32]));

    write_color_tags(&mut ifd, raw);
    write_profile_tags(&mut ifd, raw, opts.order);
    write_opcode_tags(&mut ifd, raw);

    if let Some(lossy) = lossy {
        set_lossy_tiles(&mut ifd, raw, lossy)?;
    } else {
        match (raw.data, opts.compression) {
            (RawData::F32(v), DngCompression::Deflate { tile, half }) => {
                let tile = deflate_tile(tile);
                let bp = if half { 2 } else { 4 };
                ifd.set(t::BITS_PER_SAMPLE, Value::Short(vec![(bp * 8) as u16; cpp]));
                ifd.set(t::SAMPLE_FORMAT, Value::Short(vec![3; cpp]));
                ifd.set(t::COMPRESSION, Value::Short(vec![compression::ADOBE_DEFLATE]));
                ifd.set(t::PREDICTOR, Value::Short(vec![3]));
                let (ta, td) = (w.div_ceil(tile), h.div_ceil(tile));
                let tiles: Vec<Vec<u8>> = (0..ta * td)
                    .into_par_iter()
                    .map(|i| {
                        let (tx, ty) = (i % ta * tile, i / ta * tile);
                        let mut buf = Vec::with_capacity(tile * tile * cpp * bp);
                        let mut row = Vec::with_capacity(tile * cpp * bp);
                        for y in 0..tile {
                            let sy = (ty + y).min(h - 1);
                            row.clear();
                            for x in 0..tile {
                                let sx = (tx + x).min(w - 1);
                                for s in 0..cpp {
                                    let f = v[(sy * w + sx) * cpp + s];
                                    if half {
                                        row.extend_from_slice(&f32_to_f16(f).to_be_bytes());
                                    } else {
                                        row.extend_from_slice(&f.to_bits().to_be_bytes());
                                    }
                                }
                            }
                            buf.extend_from_slice(&float_predict(&row, tile * cpp, bp, cpp));
                        }
                        miniz_oxide::deflate::compress_to_vec_zlib(&buf, DEFLATE_LEVEL)
                    })
                    .collect();
                ifd.set_image(ImageData::Tiles { tile_width: tile as u32, tile_height: tile as u32, tiles });
            }
            (RawData::F32(v), _) => {
                ifd.set(t::BITS_PER_SAMPLE, Value::Short(vec![32; cpp]));
                ifd.set(t::SAMPLE_FORMAT, Value::Short(vec![3; cpp]));
                ifd.set(t::COMPRESSION, Value::Short(vec![compression::NONE]));
                let mut bytes = Vec::with_capacity(v.len() * 4);
                for &x in v {
                    let mut tmp = Vec::with_capacity(4);
                    opts.order.put_u32(&mut tmp, x.to_bits());
                    bytes.extend_from_slice(&tmp);
                }
                ifd.set_image(ImageData::Strips { rows_per_strip: h as u32, strips: vec![bytes] });
            }
            (RawData::U16(v), _) => {
                // Dark frames may never reach the sensor's white level, which must still fit.
                let maxv = u32::from(v.iter().copied().max().unwrap_or(0)).max(max_white);
                let bits = (32 - maxv.leading_zeros()).max(raw.bits).clamp(2, 16) as u8;
                ifd.set(
                    t::BITS_PER_SAMPLE,
                    Value::Short(vec![if matches!(opts.compression, DngCompression::Uncompressed) { 16 } else { bits as u16 }; cpp]),
                );
                match opts.compression {
                    DngCompression::Uncompressed => {
                        ifd.set(t::COMPRESSION, Value::Short(vec![compression::NONE]));
                        let rows = (65536 / (w * cpp * 2)).clamp(1, h);
                        let strips = v
                            .chunks(rows * w * cpp)
                            .map(|s| {
                                let mut b = Vec::with_capacity(s.len() * 2);
                                s.iter().for_each(|&x| opts.order.put_u16(&mut b, x));
                                b
                            })
                            .collect();
                        ifd.set_image(ImageData::Strips { rows_per_strip: rows as u32, strips });
                    }
                    DngCompression::Deflate { tile, .. } => {
                        let tile = deflate_tile(tile);
                        ifd.set(t::BITS_PER_SAMPLE, Value::Short(vec![16; cpp]));
                        ifd.set(t::COMPRESSION, Value::Short(vec![compression::ADOBE_DEFLATE]));
                        ifd.set(t::PREDICTOR, Value::Short(vec![2]));
                        let (ta, td) = (w.div_ceil(tile), h.div_ceil(tile));
                        let tiles: Vec<Vec<u8>> = (0..ta * td)
                            .into_par_iter()
                            .map(|i| {
                                let (tx, ty) = (i % ta * tile, i / ta * tile);
                                let mut buf = Vec::with_capacity(tile * tile * cpp * 2);
                                let mut row = vec![0u16; tile * cpp];
                                for y in 0..tile {
                                    let sy = (ty + y).min(h - 1);
                                    for x in 0..tile {
                                        let sx = (tx + x).min(w - 1);
                                        for s in 0..cpp {
                                            row[x * cpp + s] = v[(sy * w + sx) * cpp + s];
                                        }
                                    }
                                    for i in (cpp..row.len()).rev() {
                                        row[i] = row[i].wrapping_sub(row[i - cpp]);
                                    }
                                    row.iter().for_each(|&x| opts.order.put_u16(&mut buf, x));
                                }
                                miniz_oxide::deflate::compress_to_vec_zlib(&buf, DEFLATE_LEVEL)
                            })
                            .collect();
                        ifd.set_image(ImageData::Tiles { tile_width: tile as u32, tile_height: tile as u32, tiles });
                    }
                    DngCompression::Lj92 { tile } => {
                        let tile = (tile.max(2) & !1) as usize;
                        if tile > MAX_LJ92_TILE {
                            return Err(RawError::Limit("tile too large"));
                        }
                        ifd.set(t::COMPRESSION, Value::Short(vec![compression::JPEG]));
                        let (ta, td) = (w.div_ceil(tile), h.div_ceil(tile));
                        let tiles: Vec<Vec<u8>> = (0..ta * td)
                            .into_par_iter()
                            .map(|i| {
                                let (tx, ty) = (i % ta * tile, i / ta * tile);
                                let mut buf = Vec::with_capacity(tile * tile * cpp);
                                for y in 0..tile {
                                    let sy = (ty + y).min(h - 1);
                                    for x in 0..tile {
                                        let sx = (tx + x).min(w - 1);
                                        for s in 0..cpp {
                                            buf.push(v[(sy * w + sx) * cpp + s]);
                                        }
                                    }
                                }
                                if cpp == 1 {
                                    ljpeg::encode(&buf, tile / 2, tile, 2, bits, 1, 0)
                                } else {
                                    ljpeg::encode(&buf, tile, tile, cpp, bits, 1, 0)
                                }
                            })
                            .collect();
                        ifd.set_image(ImageData::Tiles { tile_width: tile as u32, tile_height: tile as u32, tiles });
                    }
                }
            }
        }
    }
    Ok(TiffWriter::new(opts.order, false).write(&[ifd])?)
}

/// Make, Model, UniqueCameraModel, Orientation, Software and the XMP packet.
fn write_identity_tags(ifd: &mut IfdBuilder, raw: &DngImage<'_>, opts: &DngWriteOptions) {
    let make = raw.make.unwrap_or_default();
    let model = raw.model.unwrap_or_default();
    if !make.is_empty() {
        ifd.set(t::MAKE, Value::Ascii(make.to_owned()));
    }
    if !model.is_empty() {
        ifd.set(t::MODEL, Value::Ascii(model.to_owned()));
    }
    let model_includes_make = !make.is_empty()
        && model.get(..make.len()).is_some_and(|prefix| prefix.eq_ignore_ascii_case(make))
        && model.get(make.len()..).is_some_and(|suffix| suffix.chars().next().is_none_or(|c| !c.is_alphanumeric() && c != '_'));
    let unique = if let Some(unique) = raw.unique_model.filter(|u| !u.is_empty()) {
        unique.to_owned()
    } else if model_includes_make || make.is_empty() {
        model.to_owned()
    } else if model.is_empty() {
        make.to_owned()
    } else {
        format!("{make} {model}")
    };
    ifd.set(t::UNIQUE_CAMERA_MODEL, Value::Ascii(if unique.is_empty() { raw.unknown_camera_model.unwrap_or("Unknown").into() } else { unique }));
    ifd.set(t::ORIENTATION, Value::Short(vec![raw.orientation.to_exif()]));
    ifd.set(t::SOFTWARE, Value::Ascii(opts.software.clone()));
    if let Some(xmp) = &opts.xmp {
        ifd.set(t::XMP, Value::Byte(xmp.clone().into_bytes()));
    }
}

/// The colour tags (matrices, white balance, baseline exposure and sharpness) and the CinemaDNG frame tags.
fn write_color_tags(ifd: &mut IfdBuilder, raw: &DngImage<'_>) {
    let c = &raw.color;
    for (i, tag) in [t::CALIBRATION_ILLUMINANT_1, t::CALIBRATION_ILLUMINANT_2].into_iter().enumerate() {
        if c.color_matrix[i].is_some() {
            ifd.set(tag, Value::Short(vec![c.illuminant[i]]));
        }
    }
    for (pair, tags) in [
        (&c.color_matrix, [t::COLOR_MATRIX_1, t::COLOR_MATRIX_2]),
        (&c.forward_matrix, [t::FORWARD_MATRIX_1, t::FORWARD_MATRIX_2]),
        (&c.camera_calibration, [t::CAMERA_CALIBRATION_1, t::CAMERA_CALIBRATION_2]),
    ] {
        for i in 0..2 {
            if let Some(m) = &pair[i] {
                ifd.set(tags[i], srat_mat(m));
            }
        }
    }
    if let Some(v) = c.analog_balance {
        ifd.set(t::ANALOG_BALANCE, rat_vec(&v));
    }
    if let Some(v) = c.as_shot_neutral {
        ifd.set(t::AS_SHOT_NEUTRAL, rat_vec(&v));
    } else if let Some(xy) = c.as_shot_white_xy {
        ifd.set(t::AS_SHOT_WHITE_XY, rat_vec(&xy));
    } else if let Some(m) = raw.wb_multipliers.filter(|m| m.iter().all(|v| *v > 0.0 && v.is_finite())) {
        // vendor raws (NEF, ARW…) carry white balance as multipliers: the DNG neutral is their reciprocal
        ifd.set(t::AS_SHOT_NEUTRAL, rat_vec(&m.map(|v| 1.0 / v as f64)));
    }
    ifd.set(t::BASELINE_EXPOSURE, Value::SRational(vec![srational(c.baseline_exposure)]));
    if let Some(v) = c.baseline_sharpness {
        ifd.set(t::BASELINE_SHARPNESS, rat_vec(&[v]));
    }
    if let Some((n, d)) = raw.cinema.frame_rate.and_then(|(n, d)| Some((i32::try_from(n).ok()?, i32::try_from(d).ok()?))) {
        ifd.set(51044, Value::SRational(vec![(n, d)]));
    }
    if let Some(tc) = raw.cinema.timecode {
        ifd.set(51043, Value::Byte(tc.to_bytes().to_vec()));
    }
    if let Some(reel) = raw.cinema.reel_name.as_deref().filter(|r| !r.is_empty()) {
        ifd.set(51081, Value::Ascii(reel.to_owned()));
    }
}

fn write_profile_tags(ifd: &mut IfdBuilder, raw: &DngImage<'_>, order: ByteOrder) {
    // The source's own profile look travels with its data (conversions, smart previews, merges).
    let profile = raw.color.profile;
    let table_tags = |ifd: &mut IfdBuilder, table: &crate::profile::HsvTable, dims: u16, enc: u16| {
        ifd.set(dims, Value::Long(vec![table.hue_divisions as u32, table.sat_divisions as u32, table.val_divisions as u32]));
        if table.srgb_value {
            ifd.set(enc, Value::Long(vec![1]));
        }
    };
    let flat = |table: &crate::profile::HsvTable| Value::Float(table.data.iter().flatten().copied().collect());
    match &profile.hue_sat_map {
        [Some(first), second] => {
            table_tags(ifd, first, t::PROFILE_HUE_SAT_MAP_DIMS, t::PROFILE_HUE_SAT_MAP_ENCODING);
            ifd.set(t::PROFILE_HUE_SAT_MAP_DATA_1, flat(first));
            if let Some(second) = second.as_ref().filter(|second| {
                (second.hue_divisions, second.sat_divisions, second.val_divisions) == (first.hue_divisions, first.sat_divisions, first.val_divisions)
            }) {
                ifd.set(t::PROFILE_HUE_SAT_MAP_DATA_2, flat(second));
            }
        }
        [None, Some(table)] => {
            table_tags(ifd, table, t::PROFILE_HUE_SAT_MAP_DIMS, t::PROFILE_HUE_SAT_MAP_ENCODING);
            ifd.set(t::PROFILE_HUE_SAT_MAP_DATA_1, flat(table));
        }
        [None, None] => {}
    }
    if let Some(table) = &profile.look_table {
        table_tags(ifd, table, t::PROFILE_LOOK_TABLE_DIMS, t::PROFILE_LOOK_TABLE_ENCODING);
        ifd.set(t::PROFILE_LOOK_TABLE_DATA, flat(table));
    }
    if let Some(curve) = &profile.tone_curve {
        ifd.set(t::PROFILE_TONE_CURVE, Value::Float(curve.points.iter().flatten().copied().collect()));
    }
    if let Some(map) = &profile.gain_table_map {
        // The raw IFD is IFD 0 here, where both tag versions are valid.
        let (version2, bytes) = map.to_bytes(order);
        ifd.set(if version2 { t::PROFILE_GAIN_TABLE_MAP_2 } else { t::PROFILE_GAIN_TABLE_MAP }, Value::Undefined(bytes));
    }
}

fn write_opcode_tags(ifd: &mut IfdBuilder, raw: &DngImage<'_>) {
    for (list, tag) in [(&raw.opcodes.list1, t::OPCODE_LIST_1), (&raw.opcodes.list2, t::OPCODE_LIST_2), (&raw.opcodes.list3, t::OPCODE_LIST_3)] {
        if !list.is_empty() {
            ifd.set(tag, Value::Undefined(opcodes::write_list(list)));
        }
    }
}

#[cfg(test)]
#[path = "dngwrite_legacy_tests.rs"]
mod legacy;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlackLevel, OpcodeLists, RawData, RawFormat, RawImage, Rect};

    fn tiny_raw() -> RawImage {
        let (w, h) = (8usize, 8usize);
        let full = Rect::new(0, 0, w, h);
        RawImage {
            format: RawFormat::Dng,
            width: w,
            height: h,
            cpp: 1,
            data: RawData::U16((0..64).map(|i| i as u16 * 129).collect()),
            cfa: None,
            bits: 14,
            black: BlackLevel::uniform(0.0),
            white: vec![16383.0],
            active_area: full,
            crop: full,
            orientation: crate::Orientation::Normal,
            color: crate::ColorData::default(),
            wb_multipliers: None,
            linearized: false,
            linearization_table: None,
            pixel_aspect: 1.0,
            opcodes: OpcodeLists::default(),
            camera: Default::default(),
            cinema: Default::default(),
            lens_metadata: Default::default(),
            level_sources: Default::default(),
            notices: crate::RawNotices::default(),
        }
    }

    #[test]
    fn borrowed_writer_preserves_owned_output() {
        let mut raw = tiny_raw();
        raw.camera.unique_model = Some("Exact camera model".into());
        for floating in [false, true] {
            if floating {
                raw.data = RawData::F32((0..64).map(|v| v as f32 / 64.0).collect());
                raw.white = vec![1.0];
            }
            for order in [ByteOrder::Little, ByteOrder::Big] {
                for (compression, old_compression) in [
                    (DngCompression::Uncompressed, legacy::DngCompression::Uncompressed),
                    (DngCompression::Lj92 { tile: 16 }, legacy::DngCompression::Lj92 { tile: 16 }),
                    (DngCompression::Deflate { tile: 16, half: false }, legacy::DngCompression::Deflate { tile: 16, half: false }),
                    (DngCompression::Deflate { tile: 16, half: true }, legacy::DngCompression::Deflate { tile: 16, half: true }),
                ] {
                    let xmp = Some("<xmp>retained</xmp>".to_string());
                    let options = DngWriteOptions { compression, order, xmp: xmp.clone(), software: "Test writer".into() };
                    let old_options = legacy::DngWriteOptions { compression: old_compression, order, xmp, software: "Test writer".into() };
                    let expected = legacy::write_dng(&raw, &old_options).unwrap();
                    assert_eq!(expected, write_dng_view(&DngImage::from(&raw), &options).unwrap());
                    assert_eq!(expected, write_dng(&raw, &options).unwrap());
                }
            }
        }
    }

    #[test]
    fn lossy_tiles_preserve_prepared_codes_padding_and_tags() {
        let mut raw = tiny_raw();
        raw.width = 2;
        raw.height = 1;
        raw.cpp = 3;
        raw.bits = 8;
        raw.data = RawData::U16(vec![1, 2, 3, 250, 251, 252]);
        raw.active_area = Rect::new(0, 0, 2, 1);
        raw.crop = raw.active_area;
        let linearization: Vec<u16> = (0..256).map(|v| v * 257).collect();
        let called = std::sync::atomic::AtomicUsize::new(0);
        let encoder = |pixels: Vec<u8>, tile: u32, quality: u8| {
            called.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            assert_eq!((tile, quality), (16, 100));
            for row in pixels.as_chunks::<{ 16 * 3 }>().0 {
                assert_eq!(&row[..3], &[1, 2, 3]);
                assert!(row[3..].as_chunks::<3>().0.iter().all(|p| *p == [250, 251, 252]));
            }
            let mut output = Vec::new();
            jpeg_encoder::Encoder::new(&mut output, quality)
                .encode(&pixels, tile as u16, tile as u16, jpeg_encoder::ColorType::Rgb)
                .map_err(|e| RawError::Corrupt(e.to_string()))?;
            Ok(output)
        };
        let bytes = write_lossy_dng(&DngImage::from(&raw), &DngWriteOptions::default(), 17, 255, &linearization, &encoder).unwrap();
        assert_eq!(called.load(std::sync::atomic::Ordering::Relaxed), 1);
        let tiff = craft_tiff::Tiff::parse(&bytes).unwrap();
        let ifd = tiff.all_ifds()[0];
        assert_eq!(ifd.u16(t::COMPRESSION), Some(compression::LOSSY_JPEG));
        assert_eq!(ifd.u64s(t::LINEARIZATION_TABLE).unwrap(), linearization.iter().map(|&v| u64::from(v)).collect::<Vec<_>>());
        assert_eq!((crate::decode(&bytes).unwrap().width, crate::decode(&bytes).unwrap().height), (2, 1));
        assert_eq!(lossy_tile_edge(u32::MAX), 4096);
        assert_eq!(lossy_tile_edge(0), 16);
        let failing = |_: Vec<u8>, _: u32, _: u8| Err(RawError::Limit("test encoder failure"));
        assert!(matches!(
            write_lossy_dng(&DngImage::from(&raw), &DngWriteOptions::default(), 16, 90, &linearization, &failing),
            Err(RawError::Limit("test encoder failure"))
        ));
        let never = |_: Vec<u8>, _: u32, _: u8| -> Result<Vec<u8>> { panic!("malformed input reached encoder") };
        assert!(write_lossy_dng(&DngImage::from(&raw), &DngWriteOptions::default(), 16, 90, &linearization[..255], &never).is_err());
        raw.data = RawData::U16(vec![256; 6]);
        assert!(write_lossy_dng(&DngImage::from(&raw), &DngWriteOptions::default(), 16, 90, &linearization, &never).is_err());
        raw.data = RawData::U16(vec![1; 5]);
        assert!(write_lossy_dng(&DngImage::from(&raw), &DngWriteOptions::default(), 16, 90, &linearization, &never).is_err());
    }

    #[test]
    fn oversized_lj92_padding_is_rejected() {
        let raw = tiny_raw();
        // The near-boundary case previously completed with an oversized padded tile; keep it first
        // so reverting the guard fails without trying the multi-gigabyte cases.
        for tile in [2050, 65534, u32::MAX] {
            let opts = DngWriteOptions { compression: DngCompression::Lj92 { tile }, ..Default::default() };
            assert!(matches!(write_dng(&raw, &opts), Err(RawError::Limit("tile too large"))));
        }
    }

    #[test]
    fn default_lj92_tiles_preserve_samples() {
        let raw = tiny_raw();
        let bytes = write_dng(&raw, &DngWriteOptions::default()).expect("normal tiles write");
        let back = crate::decode(&bytes).expect("normal tiles decode");
        assert_eq!((back.width, back.height), (raw.width, raw.height));
        match (&back.data, &raw.data) {
            (RawData::U16(actual), RawData::U16(expected)) => assert_eq!(actual, expected),
            _ => panic!("integer DNG changed sample type"),
        }
    }

    /// Deflate tiles of `tile × tile` pad into a square buffer that grows with the square: a
    /// caller-supplied edge of `u32::MAX` would make the writer pad an 8 × 8 image into billions of
    /// edge-replicate iterations and a multi-gigabyte buffer. The clamp keeps the bounded shape — same
    /// file as requesting the cap exactly, and a DNG either way.
    #[test]
    fn a_hostile_deflate_tile_edge_is_clamped() {
        let (w, h) = (8usize, 8usize);
        let full = Rect::new(0, 0, w, h);
        let raw = RawImage {
            format: RawFormat::Dng,
            width: w,
            height: h,
            cpp: 1,
            data: RawData::U16((0..64).map(|i| i as u16 * 129).collect()),
            cfa: None,
            bits: 12,
            black: BlackLevel::uniform(0.0),
            white: vec![4095.0],
            active_area: full,
            crop: full,
            orientation: crate::Orientation::Normal,
            color: crate::ColorData::default(),
            wb_multipliers: None,
            linearized: false,
            linearization_table: None,
            pixel_aspect: 1.0,
            opcodes: OpcodeLists::default(),
            camera: Default::default(),
            cinema: Default::default(),
            lens_metadata: Default::default(),
            level_sources: Default::default(),
            notices: crate::RawNotices::default(),
        };
        let opts = |tile: u32| DngWriteOptions { compression: DngCompression::Deflate { tile, half: false }, ..DngWriteOptions::default() };
        let hostile = write_dng(&raw, &opts(u32::MAX)).expect("the clamped tile writes");
        assert_eq!(hostile, write_dng(&raw, &opts(2048)).expect("the cap itself writes"), "u32::MAX is clamped to the cap");
        assert_ne!(hostile, write_dng(&raw, &opts(256)).expect("a normal tile writes"), "the clamp is not a no-op");
        assert_eq!(crate::probe(&hostile), Some(RawFormat::Dng), "the clamped output is still a DNG");
        // the writer's own padded tile (2048 × 2048 around an 8 × 8 image) reads back
        let back = crate::decode(&hostile).expect("a tile this writer pads to decodes");
        assert_eq!((back.width, back.height), (w, h));
    }
}
