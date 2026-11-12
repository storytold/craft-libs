//! Canon sRAW / mRAW: reduced-resolution YCbCr "raw" frames in CR2 (raw IFD tag `0xc6c5` = 4).
//!
//! What the files contain (measured on the 31 CC0 sRAW / mRAW samples of 16 models, see the evidence below):
//!
//! - The raw strip is one lossless-JPEG frame with three components. Luma is sampled 2×1 (sRAW, 4:2:2, `0x21`) or
//!   2×2 (mRAW, 4:2:0, `0x22`), both chroma components 1×1; precision 15, predictor 1, no restart markers.
//!   The frame's width and height are only a factorisation of the sample count: they are not the image size.
//! - The decoded sample stream (MCU after MCU: `Y Y Cb Cr` or `Y Y Y Y Cb Cr`) is cut into vertical slices exactly
//!   like the CFA data (`cr2_slice = [n, w, last]`, widths counted in samples). A slice row is a whole number of
//!   MCUs, so the image is `Σw / MCU-size × 2` pixels wide, and one slice row is one pixel row (4:2:2) or two (4:2:0).
//! - Prediction: each component is a one-dimensional chain in decoding order (so the second luma row of a 4:2:0 MCU
//!   continues from the end of the first), and the first sample of every frame row starts from the first sample of
//!   the previous one ([`ljpeg::Prediction::Sequential`]). The Sony prediction (geometric) gives vertical stripes on
//!   these files.
//! - The samples are linear. Luma is the BT.601 luma (0.299, 0.587, 0.114; also the TIFF 6.0 default
//!   `YCbCrCoefficients`) of white-balanced camera RGB, about 7/8 of the black-subtracted sensor counts; chroma is
//!   centred on 16384. White balance is baked in (neutral surfaces have neutral chroma); which of the maker note's
//!   white-balance sets was used is not always the as-shot one, so it is not undone.
//! - Chroma is stored as a scaled colour difference, `R − Y = a·(Cr − 16384)` and `B − Y = b·(Cb − 16384)`, with
//!   `G` from the luma equation. `a` and `b` are about 1.0 on bodies from 2012 on, but about 5.7 and 7.4 (the TIFF
//!   6.0 YCbCr ratio 1.402 : 1.772, scaled by about 4.1) on the 2008–2010 bodies (5D Mark II, 50D, 7D, 60D, 1D Mark IV).
//!   Rather than keying a table on the model, the gains are measured per file against the 16-bit RGB image the
//!   camera stores in IFD2 for these files (see [`chroma_gains`]).
//!
//! Only these files were used as evidence: the sRAW samples themselves, the same cameras' CFA files of the same
//! scenes (raw counts against the decoded samples), and the IFD2 image. The ExifTool tag documentation was used for
//! tag names.

use super::{COLOR_BALANCE, CR2_SLICE, SENSOR_INFO, words};
use crate::{BlackLevel, Camera, ColorData, Limits, Mode, OpcodeLists, Orientation, RawData, RawError, RawFormat, RawImage, Rect, Result, ljpeg};
use craft_tiff::image::{Chunk, chunk_bytes};
use craft_tiff::{ByteOrder, Ifd, Tiff, makernote, tags as t};
use rayon::prelude::*;

/// Chroma samples are centred on this value.
const CHROMA_CENTRE: f64 = 16384.0;
/// BT.601 luma weights, the TIFF 6.0 default `YCbCrCoefficients`.
const KR: f64 = 0.299;
const KG: f64 = 0.587;
const KB: f64 = 0.114;
/// Decoded luma relative to black-subtracted sensor counts (white-balanced): 0.877, 0.877, 0.896, 0.880, 0.871, 0.851
/// and 0.846 on the 7D Mark II, 5DS R, 6D Mark II, 5DS, 5D Mark II, 1D X Mark II and 50D (CFA and sRAW files of the
/// same scene), so 7/8. It sets the white level, so that a file renders about as bright as the CFA file of the same
/// scene.
const LUMA_GAIN: f32 = 0.875;
/// Full scale of a 14-bit sensor, the white level of a CFA CR2 without a saturation plateau.
const SENSOR_FULL: f32 = 16383.0;
/// Mid-tone limit (IFD2 units) of the pixels used for the chroma fit: above about a quarter of full scale the
/// camera's own RGB image rolls off.
const FIT_LIMIT: f64 = 4096.0;

/// How the decoded frame maps to the image.
#[derive(Debug, PartialEq)]
pub(super) struct Layout {
    pub width: usize,
    pub height: usize,
    /// Vertical subsampling of the chroma planes: 1 (4:2:2, sRAW) or 2 (4:2:0, mRAW).
    pub vertical: usize,
    /// Slice widths in samples.
    widths: Vec<usize>,
    /// Slice height in MCU rows.
    rows: usize,
    /// Samples per MCU.
    mcu: usize,
    /// Frame size from the SOF3 header.
    frame_width: usize,
    frame_height: usize,
}

impl Layout {
    /// The layout of a `frame_width × frame_height` frame with `vertical` chroma subsampling and `cr2_slice` tag
    /// values `slices` (`[n, w, last]`).
    pub(super) fn new(frame_width: usize, frame_height: usize, vertical: usize, slices: &[u64], limits: &Limits) -> Result<Layout> {
        let &[n, w, last] = slices else { return Err(RawError::Corrupt("sRAW without slice layout".into())) };
        if n > 64 || w == 0 || last == 0 {
            return Err(RawError::Corrupt("sRAW slice layout".into()));
        }
        let mcu = 2 * vertical + 2;
        let mut widths = vec![w as usize; n as usize];
        widths.push(last as usize);
        if widths.iter().any(|w| w % mcu != 0) {
            return Err(RawError::Corrupt("sRAW slice is not a whole number of MCUs".into()));
        }
        let samples =
            frame_width.checked_mul(frame_height).and_then(|p| p.checked_add(p / vertical)).ok_or(RawError::Limit("sRAW frame too large"))?;
        let row = widths.iter().try_fold(0usize, |a, &w| a.checked_add(w)).ok_or(RawError::Limit("sRAW slice widths"))?;
        if samples == 0 || row == 0 || samples % row != 0 {
            return Err(RawError::Corrupt(format!("sRAW slices ({row} samples per row) do not divide the frame ({samples} samples)")));
        }
        let rows = samples / row;
        let (width, height) = (row / mcu * 2, rows * vertical);
        if width.checked_mul(height).and_then(|p| p.checked_mul(3)).is_none_or(|s| s > limits.samples()) {
            return Err(RawError::Limit("sRAW image too large"));
        }
        Ok(Layout { width, height, vertical, widths, rows, mcu, frame_width, frame_height })
    }
}

/// Planar YCbCr at image geometry: luma `width × height`, chroma `width/2 × height/vertical`.
struct Planes {
    width: usize,
    height: usize,
    vertical: usize,
    y: Vec<u16>,
    cb: Vec<u16>,
    cr: Vec<u16>,
}

impl Planes {
    fn chroma_width(&self) -> usize {
        self.width / 2
    }

    /// Re-chop the decoded MCU stream into image planes.
    fn arrange(frame: &ljpeg::FrameSubsampled, layout: &Layout) -> Planes {
        let (w, h, v, m) = (layout.width, layout.height, layout.vertical, layout.mcu);
        let cw = w / 2;
        let mut p = Planes { width: w, height: h, vertical: v, y: vec![0; w * h], cb: vec![0; cw * (h / v)], cr: vec![0; cw * (h / v)] };
        let mcus_per_frame_row = layout.frame_width / 2;
        let mut x0 = 0;
        for &sw in &layout.widths {
            let per_row = sw / m;
            let first = layout.rows * (x0 / m);
            for r in 0..layout.rows {
                for k in 0..per_row {
                    let q = first + r * per_row + k;
                    let (qy, qx) = (q / mcus_per_frame_row, q % mcus_per_frame_row);
                    let px = (x0 / m + k) * 2;
                    for dy in 0..v {
                        let src = (qy * v + dy) * layout.frame_width + qx * 2;
                        let dst = (r * v + dy) * w + px;
                        p.y[dst..dst + 2].copy_from_slice(&frame.planes[0][src..src + 2]);
                    }
                    p.cb[r * cw + px / 2] = frame.planes[1][qy * mcus_per_frame_row + qx];
                    p.cr[r * cw + px / 2] = frame.planes[2][qy * mcus_per_frame_row + qx];
                }
            }
            x0 += sw;
        }
        p
    }

    /// Mean luma / Cb / Cr over the pixel window `[x0, x1) × [y0, y1)` (non-empty, inside the image).
    fn window_mean(&self, x0: usize, x1: usize, y0: usize, y1: usize) -> (f64, f64, f64) {
        let (mut sy, mut ny) = (0f64, 0f64);
        for y in y0..y1 {
            let row = &self.y[y * self.width + x0..y * self.width + x1];
            sy += row.iter().map(|&v| f64::from(v)).sum::<f64>();
            ny += row.len() as f64;
        }
        let (cx0, cx1) = (x0 / 2, x1.div_ceil(2).max(x0 / 2 + 1));
        let (cy0, cy1) = (y0 / self.vertical, y1.div_ceil(self.vertical).max(y0 / self.vertical + 1));
        let (mut sb, mut sr, mut nc) = (0f64, 0f64, 0f64);
        for y in cy0..cy1.min(self.height / self.vertical) {
            for x in cx0..cx1.min(self.chroma_width()) {
                sb += f64::from(self.cb[y * self.chroma_width() + x]);
                sr += f64::from(self.cr[y * self.chroma_width() + x]);
                nc += 1.0;
            }
        }
        let nc = nc.max(1.0);
        (sy / ny.max(1.0), sb / nc - CHROMA_CENTRE, sr / nc - CHROMA_CENTRE)
    }

    /// Linear RGB (`width × height × 3`): `R = Y + a·Cr`, `B = Y + b·Cb`, `G` from the luma equation.
    fn to_rgb(&self, a: f64, b: f64) -> Vec<u16> {
        let (gr, gb) = ((KR / KG * a) as f32, (KB / KG * b) as f32);
        let (a, b) = (a as f32, b as f32);
        let cw = self.chroma_width();
        let centre = CHROMA_CENTRE as f32;
        let mut out = vec![0u16; self.width * self.height * 3];
        out.par_chunks_mut(self.width * 3).enumerate().for_each(|(y, row)| {
            let luma = &self.y[y * self.width..(y + 1) * self.width];
            let chroma = (y / self.vertical) * cw;
            for (x, px) in row.as_chunks_mut::<3>().0.iter_mut().enumerate() {
                let (cb, cr) = (f32::from(self.cb[chroma + x / 2]) - centre, f32::from(self.cr[chroma + x / 2]) - centre);
                let l = f32::from(luma[x]);
                px[0] = (l + a * cr).round().clamp(0.0, 65535.0) as u16;
                px[1] = (l - gr * cr - gb * cb).round().clamp(0.0, 65535.0) as u16;
                px[2] = (l + b * cb).round().clamp(0.0, 65535.0) as u16;
            }
        });
        out
    }
}

/// The 16-bit RGB image in IFD2 (`SRawType` 3): the camera's own rendering of the same frame at a fraction of the
/// size, white-balanced and linear below about a quarter of full scale.
struct Preview {
    width: usize,
    height: usize,
    rgb: Vec<u16>,
}

fn read_preview(bytes: &[u8], tiff: &Tiff) -> Option<Preview> {
    let info = tiff.ifds.get(2)?.image().ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    if info.samples_per_pixel != 3 || info.bits_per_sample != [16, 16, 16] || info.compression != 1 || info.planar != 1 || info.sample_format != 1 {
        return None;
    }
    let n = w.checked_mul(h)?.checked_mul(3)?;
    // uncompressed 16-bit samples: more than the whole file holds is refused before the buffer is reserved
    if n == 0 || n > 1 << 24 || n * 2 > bytes.len() {
        return None;
    }
    let mut raw: Vec<u8> = Vec::with_capacity(n * 2);
    for c in info.chunks(bytes.len() as u64) {
        raw.extend_from_slice(chunk_bytes(bytes, &c)?);
        if raw.len() >= n * 2 {
            break;
        }
    }
    if raw.len() < n * 2 {
        return None;
    }
    let order: ByteOrder = tiff.order;
    let rgb = raw.as_chunks::<2>().0.iter().take(n).map(|&c| order.u16(c)).collect();
    Some(Preview { width: w, height: h, rgb })
}

#[derive(Default)]
struct Regression {
    n: f64,
    sx: f64,
    st: f64,
    sxx: f64,
    sxt: f64,
    stt: f64,
}

impl Regression {
    fn add(&mut self, x: f64, t: f64) {
        self.n += 1.0;
        self.sx += x;
        self.st += t;
        self.sxx += x * x;
        self.sxt += x * t;
        self.stt += t * t;
    }

    /// Least-squares slope of `t` on `x` (with intercept) and its residual sum of squares.
    fn slope(&self) -> Option<(f64, f64)> {
        let var = self.n * self.sxx - self.sx * self.sx;
        if self.n < 500.0 || var <= 1e-9 * self.n * self.n {
            return None;
        }
        let slope = (self.n * self.sxt - self.sx * self.st) / var;
        let icpt = (self.st - slope * self.sx) / self.n;
        let rss =
            self.stt - 2.0 * slope * self.sxt - 2.0 * icpt * self.st + slope * slope * self.sxx + 2.0 * slope * icpt * self.sx + self.n * icpt * icpt;
        Some((slope, rss.max(0.0)))
    }
}

/// How the IFD2 image sits on the full-size planes: its pixel `(i, j)` covers the window starting at
/// `(x0 + i·sx, y0 + j·sy)` and `sx × sy` pixels large.
struct Grid {
    x0: f64,
    y0: f64,
    sx: f64,
    sy: f64,
}

/// The chroma gains `(a, b)` of this file (`R − Y = a·Cr`, `B − Y = b·Cb`), by least squares of the IFD2 image
/// against the planes averaged to its size. Where IFD2 sits on the frame depends on the model (the whole frame,
/// the sensor's active area, or a whole or half-integer reduction of the frame that overhangs it by a few pixels),
/// so these are tried and the best fit is kept. `None` when the file has no usable IFD2 or the fit is not well
/// conditioned.
fn chroma_gains(planes: &Planes, preview: &Preview, active: Rect) -> Option<(f64, f64)> {
    let (pw, ph) = (preview.width as f64, preview.height as f64);
    let (w, h) = (planes.width as f64, planes.height as f64);
    let grids = [
        Grid { x0: 0.0, y0: 0.0, sx: w / pw, sy: h / ph },
        Grid { x0: active.x as f64, y0: active.y as f64, sx: active.width as f64 / pw, sy: active.height as f64 / ph },
        Grid { x0: 0.0, y0: 0.0, sx: (2.0 * w / pw).round() / 2.0, sy: (2.0 * h / ph).round() / 2.0 },
    ];
    let mut best: Option<(f64, f64, f64)> = None;
    for g in &grids {
        if !(g.sx >= 1.0 && g.sy >= 1.0) {
            continue;
        }
        let (mut fr, mut fb) = (Regression::default(), Regression::default());
        for j in 0..preview.height {
            let y0 = (g.y0 + j as f64 * g.sy) as usize;
            let y1 = ((g.y0 + (j + 1) as f64 * g.sy) as usize).min(planes.height);
            for i in 0..preview.width {
                let p = &preview.rgb[(j * preview.width + i) * 3..][..3];
                let [r, gr, b] = [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])];
                if r.max(gr).max(b) >= FIT_LIMIT || r.min(gr).min(b) <= 0.0 {
                    continue;
                }
                let x0 = (g.x0 + i as f64 * g.sx) as usize;
                let x1 = ((g.x0 + (i + 1) as f64 * g.sx) as usize).min(planes.width);
                if x1 <= x0 || y1 <= y0 {
                    continue;
                }
                let (y, cb, cr) = planes.window_mean(x0, x1, y0, y1);
                if y < 150.0 {
                    continue;
                }
                fr.add(cr, r - y);
                fb.add(cb, b - y);
            }
        }
        if let (Some((a, ra)), Some((b, rb))) = (fr.slope(), fb.slope()) {
            let cost = (ra + rb) / (fr.n + fb.n);
            if a.is_finite() && b.is_finite() && best.is_none_or(|(_, _, c)| cost < c) {
                best = Some((a, b, cost));
            }
        }
    }
    best.map(|(a, b, _)| (a, b)).filter(|(a, b)| (0.25..=16.0).contains(a) && (0.25..=16.0).contains(b))
}

/// Word index of `AverageBlackLevel` (four equal-ish values) in the maker note's `ColorData` by `ColorDataVersion`
/// (word 0), located by searching the CC0 samples for the values ExifTool reports.
fn average_black_at(version: i16) -> Option<usize> {
    Some(match version {
        6 | 7 | 9 => 231,
        10 | 11 => 276,
        12 | 13 | 15 => 326,
        _ => return None,
    })
}

/// The sensor's black level in raw counts from `ColorData` (`None` for versions whose position is not known, and
/// when the four values are not plausible and near-equal).
fn colour_data_black(words: &[u64]) -> Option<f32> {
    let version = words.first().map(|&v| v as u16 as i16)?;
    let q = words.get(average_black_at(version)?..)?.get(..4)?;
    let (lo, hi) = (q.iter().min()?, q.iter().max()?);
    ((256..=4096).contains(lo) && hi - lo <= 8).then(|| q.iter().sum::<u64>() as f32 / 4.0)
}

/// Decode a Canon sRAW / mRAW CR2 into linear white-balanced camera RGB (`cpp` 3).
pub(super) fn decode(bytes: &[u8], tiff: &Tiff, raw: &Ifd, mode: Mode, limits: &Limits) -> Result<RawImage> {
    let ifd0 = tiff.ifds.first().ok_or_else(|| RawError::Corrupt("TIFF without IFDs".into()))?;
    let off = raw.u64(t::STRIP_OFFSETS).ok_or(RawError::Tiff(craft_tiff::TiffError::MissingTag(t::STRIP_OFFSETS)))?;
    let len = raw.u64(t::STRIP_BYTE_COUNTS).unwrap_or(bytes.len() as u64 - off.min(bytes.len() as u64));
    let chunk = Chunk { index: 0, x: 0, y: 0, width: 0, height: 0, plane: 0, offset: off, len };
    let src = chunk_bytes(bytes, &chunk).ok_or_else(|| RawError::Corrupt("raw strip outside file".into()))?;
    let (fw, fh, vertical) = ljpeg::frame_info_subsampled(src)?;
    let slices = raw.u64s(CR2_SLICE).unwrap_or_default();
    let layout = Layout::new(fw, fh, vertical, &slices, limits)?;

    let make = ifd0.string(t::MAKE).unwrap_or_default();
    let mn =
        tiff.exif().and_then(|e| e.get(t::MAKER_NOTE)).and_then(|e| makernote::parse_makernote(bytes, e.offset, e.count() as u64, tiff.order, &make));
    let (w, h) = (layout.width, layout.height);
    let mut active = Rect::new(0, 0, w, h);
    if let Some(si) = mn.as_ref().and_then(|m| m.ifd.u64s(SENSOR_INFO)).filter(|v| v.len() >= 9) {
        let (l, tp, r, b) = (si[5] as usize, si[6] as usize, si[7] as usize, si[8] as usize);
        if r > l && b > tp && r < w && b < h {
            active = Rect::new(l, tp, r - l + 1, b - tp + 1);
        }
    }
    let colour_data = mn.as_ref().and_then(|m| m.ifd.value(COLOR_BALANCE).map(|v| words(v, m.order)));
    let black_counts = colour_data.as_deref().and_then(colour_data_black);
    // 1.0 = a CFA CR2 of the same body at full 14-bit scale; see LUMA_GAIN
    let white = LUMA_GAIN * (SENSOR_FULL - black_counts.unwrap_or(0.0));

    let data = match mode {
        Mode::Header => Vec::new(),
        Mode::Full => {
            let samples = fw * fh + fw * fh / vertical;
            let frame = ljpeg::decode_subsampled(src, samples.min(limits.samples()), ljpeg::Prediction::Sequential)?;
            let planes = Planes::arrange(&frame, &layout);
            drop(frame);
            let (a, b) = read_preview(bytes, tiff).and_then(|p| chroma_gains(&planes, &p, active)).unwrap_or((1.0, 1.0));
            planes.to_rgb(a, b)
        }
    };

    let img = RawImage {
        format: RawFormat::Cr2,
        width: w,
        height: h,
        cpp: 3,
        data: RawData::U16(data),
        cfa: None,
        bits: 14,
        black: BlackLevel::uniform(0.0),
        white: vec![white],
        active_area: active,
        crop: Rect::new(0, 0, active.width, active.height),
        orientation: Orientation::from_exif(ifd0.u16(t::ORIENTATION).unwrap_or(1)),
        color: ColorData::default(),
        // the camera's white balance is already applied, as in Sony's linear YCbCr
        wb_multipliers: Some([1.0; 3]),
        linearized: false,
        pixel_aspect: 1.0,
        opcodes: OpcodeLists::default(),
        camera: tiff.ifds.first().map(Camera::from_ifd0).unwrap_or_default(),
        cinema: Default::default(),
        lens_metadata: Default::default(),
        level_sources: Default::default(),
    };
    img.validate_for(mode)?;
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_tiff::{IfdBuilder, ImageData, TiffWriter, Value};

    const SRAW_TYPE_TAG: u16 = 0xc6c5;

    /// The 4:2:2 stream of `ljpeg`'s hand-coded fixture (4 × 2 frame, two MCUs per frame row) decodes to luma
    /// 1000 1001 / 1002 1003 / 1100 1101 / 1102 1103 in MCU order, Cb 16384 16385 16484 16485, Cr 16484 16486
    /// 16584 16586.
    fn stream_422() -> Vec<u8> {
        ljpeg::tests::fixture_subsampled(4, 2, 0x21, &[-31768, 1, -16384, -16284, 1, 1, 1, 2, 100, 1, 100, 100, 1, 1, 1, 2])
    }

    /// A CR2 whose raw IFD holds `stream` cut into the slices `slices` (IFD2 = a 2 × 2 16-bit RGB image when
    /// `preview` is given).
    fn sraw_file(stream: Vec<u8>, slices: [u16; 3], sraw_type: u32, preview: Option<[u16; 12]>) -> Vec<u8> {
        let mut ifd0 = IfdBuilder::new();
        ifd0.set(t::MAKE, Value::Ascii("Canon".into()));
        ifd0.set(t::MODEL, Value::Ascii("Canon EOS Test".into()));
        let mut ifd2 = IfdBuilder::new();
        if let Some(p) = preview {
            ifd2.set(t::COMPRESSION, Value::Short(vec![1]));
            ifd2.set(t::PHOTOMETRIC, Value::Short(vec![2]));
            ifd2.set(t::IMAGE_WIDTH, Value::Long(vec![2]));
            ifd2.set(t::IMAGE_LENGTH, Value::Long(vec![2]));
            ifd2.set(t::BITS_PER_SAMPLE, Value::Short(vec![16, 16, 16]));
            ifd2.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![3]));
            ifd2.set_image(ImageData::Strips { rows_per_strip: 2, strips: vec![p.iter().flat_map(|v| v.to_le_bytes()).collect()] });
        } else {
            ifd2.set(1, Value::Short(vec![0]));
        }
        let mut ifd3 = IfdBuilder::new();
        ifd3.set(t::COMPRESSION, Value::Short(vec![6]));
        ifd3.set(SRAW_TYPE_TAG, Value::Long(vec![sraw_type]));
        ifd3.set(CR2_SLICE, Value::Short(slices.to_vec()));
        ifd3.set_image(ImageData::Strips { rows_per_strip: 2, strips: vec![stream] });
        TiffWriter::new(ByteOrder::Little, false).write(&[ifd0, IfdBuilder::new().with(1, Value::Short(vec![0])), ifd2, ifd3]).unwrap()
    }

    #[test]
    fn slices_are_cut_back_into_image_columns_and_converted() {
        // slices [1, 4, 4]: two slices of one MCU per row; slice 0 holds MCUs 0 and 1 (image rows 0 and 1), slice 1
        // holds MCUs 2 and 3, so the image is 4 × 2 with luma 1000 1001 1100 1101 / 1002 1003 1102 1103.
        let bytes = sraw_file(stream_422(), [1, 4, 4], 4, None);
        let img = crate::decode(&bytes).unwrap();
        assert_eq!((img.width, img.height, img.cpp), (4, 2, 3));
        assert!(img.cfa.is_none());
        assert_eq!(img.wb_multipliers, Some([1.0; 3]));
        let RawData::U16(px) = &img.data else { panic!("integer data") };
        // no IFD2: chroma gains 1; R = Y + Cr', B = Y + Cb', G from the BT.601 luma
        let expect = |y: f64, cb: f64, cr: f64| {
            let (r, b) = (y + cr, y + cb);
            [r, (y - KR * r - KB * b) / KG, b].map(|v| v.round() as i32)
        };
        let cases = [
            ((0, 0), expect(1000.0, 0.0, 100.0)),
            ((1, 0), expect(1001.0, 0.0, 100.0)),
            ((2, 0), expect(1100.0, 100.0, 200.0)),
            ((3, 0), expect(1101.0, 100.0, 200.0)),
            ((0, 1), expect(1002.0, 1.0, 102.0)),
            ((3, 1), expect(1103.0, 101.0, 202.0)),
        ];
        for ((x, y), want) in cases {
            let got: Vec<i32> = px[(y * 4 + x) * 3..][..3].iter().map(|&v| i32::from(v)).collect();
            assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1), "({x},{y}): {got:?} vs {want:?}");
        }
        // header-only decoding reports the same image
        assert_eq!(crate::probe_info(&bytes).unwrap(), img.info());
    }

    #[test]
    fn a_preview_too_small_to_fit_leaves_the_gains_at_one() {
        // IFD2 of 2 × 2 pixels cannot support a fit (it needs hundreds of samples): the file still decodes
        let with = crate::decode(&sraw_file(stream_422(), [1, 4, 4], 4, Some([900; 12]))).unwrap();
        let without = crate::decode(&sraw_file(stream_422(), [1, 4, 4], 4, None)).unwrap();
        assert_eq!(with.data, without.data);
    }

    /// storytold/lightcraft#2561: IFD2's 16-bit RGB preview is read when its samples fit in the file, and refused before its buffer is
    /// reserved when they cannot: 64 × 2000 pixels (768000 bytes) whose one-row strips all point at the same bytes of
    /// a file a few tens of KiB long.
    #[test]
    fn a_preview_larger_than_the_file_is_refused() {
        let bytes = sraw_file(stream_422(), [1, 4, 4], 4, Some([900; 12]));
        let p = read_preview(&bytes, &Tiff::parse(&bytes).unwrap()).expect("a 2 x 2 preview that fits");
        assert_eq!((p.width, p.height, p.rgb), (2, 2, vec![900; 12]));
        let (w, rows) = (64u32, 2000u32);
        let mut ifd2 = IfdBuilder::new();
        ifd2.set(t::COMPRESSION, Value::Short(vec![1]));
        ifd2.set(t::PHOTOMETRIC, Value::Short(vec![2]));
        ifd2.set(t::IMAGE_WIDTH, Value::Long(vec![w]));
        ifd2.set(t::IMAGE_LENGTH, Value::Long(vec![rows]));
        ifd2.set(t::BITS_PER_SAMPLE, Value::Short(vec![16, 16, 16]));
        ifd2.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![3]));
        ifd2.set(t::ROWS_PER_STRIP, Value::Long(vec![1]));
        ifd2.set(t::STRIP_OFFSETS, Value::Long(vec![0; rows as usize]));
        ifd2.set(t::STRIP_BYTE_COUNTS, Value::Long(vec![w * 6; rows as usize]));
        let one = || IfdBuilder::new().with(1, Value::Short(vec![0]));
        let big = TiffWriter::new(ByteOrder::Little, false).write(&[one(), one(), ifd2]).unwrap();
        assert!(big.len() > (w * 6) as usize && big.len() < (w * rows * 6) as usize, "{} bytes", big.len());
        assert!(read_preview(&big, &Tiff::parse(&big).unwrap()).is_none());
    }

    #[test]
    fn only_ycbcr_frames_take_this_path() {
        // other raw types (3 is the preview image's) stay unsupported, as before
        for kind in [0, 2, 3, 5] {
            let bytes = sraw_file(stream_422(), [1, 4, 4], kind, None);
            assert!(matches!(crate::decode(&bytes), Err(RawError::Unsupported(_))), "type {kind}");
        }
    }

    #[test]
    fn slice_layouts_must_fit_the_frame() {
        let ok = Layout::new(4, 2, 1, &[1, 4, 4], &Limits::default()).unwrap();
        assert_eq!((ok.width, ok.height), (4, 2));
        // 4:2:0: six samples per MCU, one slice row is two image rows
        let l = Layout::new(8, 4, 2, &[1, 12, 12], &Limits::default()).unwrap();
        assert_eq!((l.width, l.height), (8, 4));
        assert!(Layout::new(4, 2, 1, &[1, 3, 5], &Limits::default()).is_err()); // not whole MCUs
        assert!(Layout::new(4, 2, 1, &[1, 4, 8], &Limits::default()).is_err()); // rows do not divide the frame
        assert!(Layout::new(4, 2, 1, &[1, 0, 4], &Limits::default()).is_err());
        assert!(Layout::new(4, 2, 1, &[4, 4], &Limits::default()).is_err()); // not three values
        assert!(Layout::new(4, 2, 1, &[65, 4, 4], &Limits::default()).is_err());
        // slice widths whose sum overflows (or wraps to zero) are rejected, not divided by
        assert!(Layout::new(4, 2, 1, &[3, 1 << 62, 1 << 62], &Limits::default()).is_err());
        assert!(Layout::new(4, 2, 1, &[1, u64::MAX - 3, 4], &Limits::default()).is_err());
        let bytes = sraw_file(stream_422(), [1, 4, 8], 4, None);
        assert!(crate::decode(&bytes).is_err());
    }

    /// Textured luma and smooth chroma on `w × h` planes.
    fn test_planes(w: usize, h: usize, vertical: usize) -> Planes {
        let y: Vec<u16> = (0..w * h).map(|i| (600 + ((i % w) * 13 + (i / w) * 29) % 1500) as u16).collect();
        let cw = w / 2;
        let ch = h / vertical;
        let cb = (0..cw * ch).map(|i| (16384 + (i % cw) as i32 * 7 - (i / cw) as i32 * 3 - 150) as u16).collect();
        let cr = (0..cw * ch).map(|i| (16384 + (i / cw) as i32 * 5 + (i % cw) as i32 * 2 - 120) as u16).collect();
        Planes { width: w, height: h, vertical, y, cb, cr }
    }

    /// An IFD2 image of `pw × ph` pixels at `scale` per pixel, built the way the files relate it to the planes.
    fn preview_of(p: &Planes, pw: usize, ph: usize, scale: usize, a: f64, b: f64, pedestal: f64) -> Preview {
        let mut rgb = Vec::new();
        for j in 0..ph {
            for i in 0..pw {
                let (y, cb, cr) = p.window_mean(i * scale, (i + 1) * scale, j * scale, (j + 1) * scale);
                let (r, bl) = (y + a * cr, y + b * cb);
                let g = (y - KR * r - KB * bl) / KG;
                rgb.extend([r, g, bl].map(|v| (v + pedestal).round().clamp(0.0, 65535.0) as u16));
            }
        }
        Preview { width: pw, height: ph, rgb }
    }

    #[test]
    fn chroma_gains_are_measured_against_the_preview() {
        for vertical in [1, 2] {
            for (a, b) in [(1.0, 1.0), (5.7, 7.4), (1.13, 1.12)] {
                let planes = test_planes(160, 96, vertical);
                // IFD2 is a plain 1/4 reduction, optionally with a black pedestal (some bodies add one)
                for pedestal in [0.0, 520.0] {
                    let preview = preview_of(&planes, 40, 24, 4, a, b, pedestal);
                    let (ga, gb) =
                        chroma_gains(&planes, &preview, Rect::new(0, 0, 160, 96)).unwrap_or_else(|| panic!("no fit for {a}, {b}, {vertical}"));
                    assert!((ga / a - 1.0).abs() < 0.02 && (gb / b - 1.0).abs() < 0.02, "vertical {vertical}: ({a}, {b}) measured as ({ga}, {gb})");
                }
            }
        }
        // the preview overhangs the frame by a few pixels: the whole-frame scale (4.1) is wrong, 4 is right
        let planes = test_planes(164, 96, 1);
        let mut preview = preview_of(&planes, 40, 24, 4, 5.7, 7.4, 0.0);
        let (ga, gb) = chroma_gains(&planes, &preview, Rect::new(0, 0, 164, 96)).unwrap();
        assert!((ga / 5.7 - 1.0).abs() < 0.02 && (gb / 7.4 - 1.0).abs() < 0.02, "({ga}, {gb})");
        // a preview of something else gives no usable fit rather than a wild gain
        preview.rgb.iter_mut().enumerate().for_each(|(i, v)| *v = (1000 + (i * 7919) % 3000) as u16);
        assert!(
            chroma_gains(&planes, &preview, Rect::new(0, 0, 164, 96)).is_none_or(|(a, b)| (0.25..=16.0).contains(&a) && (0.25..=16.0).contains(&b))
        );
    }

    #[test]
    fn colour_data_black_by_version() {
        let mut w = vec![0u64; 400];
        w[0] = 7;
        w[231..235].copy_from_slice(&[2048, 2047, 2048, 2048]);
        assert_eq!(colour_data_black(&w), Some(2047.75));
        w[0] = 12;
        assert_eq!(colour_data_black(&w), None); // the black level of another version lives elsewhere
        w[326..330].copy_from_slice(&[512, 512, 511, 512]);
        assert_eq!(colour_data_black(&w), Some(511.75));
        w[326..330].copy_from_slice(&[512, 512, 100, 512]); // implausible
        assert_eq!(colour_data_black(&w), None);
        w[0] = 3;
        assert_eq!(colour_data_black(&w), None); // no position known: black 0
        w[0] = 65533; // -3
        assert_eq!(colour_data_black(&w), None);
        assert_eq!(colour_data_black(&[]), None);
        assert_eq!(colour_data_black(&[7, 1, 2]), None);
    }
}
