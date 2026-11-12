//! Canon CR3 container, camera metadata and sensor geometry.
//!
//! Sources: Laurent Clévy's public CR3 format description (`CMP1`, `CDI1/IAD1`, `CMT3`),
//! <https://github.com/lclevy/canon_cr3/blob/master/readme.md>, and ExifTool's published Canon
//! tag-name tables (`SensorInfo`, `ColorData9`…`ColorData12`, `LensModel`),
//! <https://exiftool.org/TagNames/Canon.html>. No third-party decoder source was consulted.
//!
//! `IAD1` stores the valid sensor area separately from the recommended image crop. Both use inclusive
//! sensor offsets. The CFA is anchored at the full sensor's origin; cropping shifts its phase later.
//!
//! The in-camera aspect ratio (3:2, 4:3, 16:9, 1:1 ...) is not applied to the sensor data: the raw always
//! holds the whole image. The maker note's `AspectInfo` (tag `0x009a`; ExifTool names `AspectRatio`,
//! `CroppedImageWidth`, `CroppedImageHeight`, `CroppedImageLeft`, `CroppedImageTop`) holds the shot's rectangle
//! as five 32-bit values, measured from the top-left of the recommended image crop (the "Canon image", whose
//! size is ExifTool's `CanonImageWidth` x `CanonImageHeight`). That rectangle replaces the crop. Established
//! on the CC0 PowerShot SX70 HS, EOS 250D, PowerShot G5 X Mark II and EOS M6 Mark II samples shot at 3:2, 4:3,
//! 16:9 and 1:1: the rectangle is centred, has the shot's aspect, and files shot at the sensor's own aspect
//! store the whole image.
//! Unknown camera metadata versions are ignored rather than assigned a guessed white balance.

use crate::crx;
use crate::shared::{black_from_columns, white_from_data};
use crate::{BlackLevel, Camera, Cfa, ColorData, Limits, Mode, OpcodeLists, Orientation, RawData, RawError, RawFormat, RawImage, Rect, Result};
use craft_metadata::cr3::{Cr3ImageArea, Cr3Track, Cr3TrackKind, parse_cr3};
use craft_tiff::{Ifd, Tiff};

const SENSOR_INFO: u16 = 0x00e0;
const COLOR_DATA: u16 = 0x4001;
const ASPECT_INFO: u16 = 0x009a;

/// Pick the first largest raw track. Equal-sized later tracks can be the Dual Pixel delta, which
/// must never replace the main sensor image. An invalid full-size descriptor is reported, not
/// silently replaced with the reduced raw preview.
pub(crate) fn main_track(tracks: &[Cr3Track]) -> Option<&Cr3Track> {
    let mut best = None;
    let mut best_area = 0u64;
    for track in tracks {
        if let Cr3TrackKind::Raw { width, height, .. } = track.kind {
            let area = u64::from(width) * u64::from(height);
            if area > best_area {
                best = Some(track);
                best_area = area;
            }
        }
    }
    best
}

fn cfa(pattern: u8) -> Option<Cfa> {
    let name = match pattern {
        0 => "RGGB",
        1 => "GRBG",
        2 => "GBRG",
        3 => "BGGR",
        _ => return None,
    };
    Some(Cfa::bayer_static(name))
}

/// Inclusive offsets, clipped to the sample dimensions (some Canon crop-mode files record right
/// and bottom borders a few pixels beyond the encoded image).
fn inclusive_rect(bounds: [u16; 4], width: usize, height: usize) -> Option<Rect> {
    let [left, top, right, bottom] = bounds.map(usize::from);
    if left >= width || top >= height || right < left || bottom < top {
        return None;
    }
    let end_x = right.checked_add(1)?.min(width);
    let end_y = bottom.checked_add(1)?.min(height);
    Some(Rect::new(left, top, end_x.checked_sub(left)?, end_y.checked_sub(top)?))
}

fn relative_crop(crop: Rect, active: Rect) -> Option<Rect> {
    let x = crop.x.checked_sub(active.x)?;
    let y = crop.y.checked_sub(active.y)?;
    if crop.width < 2 || crop.height < 2 || x.checked_add(crop.width)? > active.width || y.checked_add(crop.height)? > active.height {
        return None;
    }
    Some(Rect::new(x, y, crop.width, crop.height))
}

fn sensor_crop(maker: Option<&Ifd>, width: usize, height: usize) -> Option<Rect> {
    let data = maker?.u64s(SENSOR_INFO)?;
    let bounds = data.get(5..9)?;
    inclusive_rect(
        [
            u16::try_from(*bounds.first()?).ok()?,
            u16::try_from(*bounds.get(1)?).ok()?,
            u16::try_from(*bounds.get(2)?).ok()?,
            u16::try_from(*bounds.get(3)?).ok()?,
        ],
        width,
        height,
    )
}

/// The shot's aspect-ratio rectangle from `AspectInfo`, placed inside `image` (the recommended crop). `None`
/// when the tag is absent or malformed, or its rectangle doesn't fit inside `image`.
fn aspect_crop(maker: Option<&Ifd>, image: Rect) -> Option<Rect> {
    let data = maker?.u64s(ASPECT_INFO)?;
    let [width, height, left, top] = [1usize, 2, 3, 4].map(|i| data.get(i).and_then(|&v| usize::try_from(v).ok()));
    let (width, height, left, top) = (width?, height?, left?, top?);
    if width < 2 || height < 2 || left.checked_add(width)? > image.width || top.checked_add(height)? > image.height {
        return None;
    }
    Some(Rect::new(image.x.checked_add(left)?, image.y.checked_add(top)?, width, height))
}

fn geometry(area: Option<&Cr3ImageArea>, maker: Option<&Ifd>, width: usize, height: usize) -> (Rect, Rect) {
    let area = area.filter(|a| usize::from(a.width) == width && usize::from(a.height) == height);
    let sensor = sensor_crop(maker, width, height);
    let mut active = area.and_then(|a| a.active).and_then(|b| inclusive_rect(b, width, height)).or(sensor).unwrap_or(Rect::new(0, 0, width, height));
    // The recommended crop. `AspectInfo` is measured from its origin, so it is applied on top of it.
    let stated = area.and_then(|a| inclusive_rect(a.crop, width, height)).or(sensor);
    let mut recommended = stated.and_then(|c| relative_crop(c, active));
    // Some crop-mode files (EOS R5 Mark II APS-C, 5376 x 3574) state an active area that starts in the dark
    // columns and stops short of the image, while their recommended crop lies on the image (measured: columns
    // 132..~256 sit at the black level, the image runs to the right edge). The recommended crop is then the
    // valid area.
    if recommended.is_none()
        && let Some(c) = stated.filter(|c| c.width >= 2 && c.height >= 2)
    {
        active = c;
        recommended = Some(Rect::new(0, 0, c.width, c.height));
    }
    let crop = recommended.map(|c| aspect_crop(maker, c).unwrap_or(c)).unwrap_or(Rect::new(0, 0, active.width, active.height));
    (active, crop)
}

#[derive(Default)]
struct CameraLevels {
    wb: Option<[f32; 3]>,
    black: Option<[f32; 4]>,
    white: Option<f32>,
}

/// Canon's versioned word arrays. Offsets are word indices from the published tag tables, not
/// byte offsets. The four as-shot WB values are R, G, G, B, independent of the CFA layout.
/// A version without a white-level offset takes its white level from the data (see `docs/cr3.md`).
fn camera_levels(maker: Option<&Ifd>, bits: u8) -> CameraLevels {
    let Some(values) = maker.and_then(|m| m.u64s(COLOR_DATA)) else { return CameraLevels::default() };
    let offsets = match values.first().copied() {
        Some(16..=19) => (71, 329, Some(797)),
        Some(32 | 33) => (85, 343, Some(811)),
        Some(34 | 48) => (105, 363, Some(641)),
        Some(64 | 65) => (105, 383, Some(661)),
        Some(66) => (105, 383, None),
        _ => return CameraLevels::default(),
    };
    let full = ((1u32 << u32::from(bits.clamp(1, 16))) - 1) as f32;
    let quadruple = |at: usize| -> Option<[f32; 4]> {
        let v = values.get(at..at + 4)?;
        Some([*v.first()? as f32, *v.get(1)? as f32, *v.get(2)? as f32, *v.get(3)? as f32])
    };
    let wb = quadruple(offsets.0).and_then(|[r, g1, g2, b]| {
        let green = (g1 + g2) * 0.5;
        if !(64.0..=32767.0).contains(&green) || (g1 - g2).abs() > green * 0.1 {
            return None;
        }
        let red = r / green;
        let blue = b / green;
        ((0.1..8.0).contains(&red) && (0.1..8.0).contains(&blue)).then_some([red, 1.0, blue])
    });
    let black = quadruple(offsets.1).filter(|v| v.iter().any(|&b| b > 0.0) && v.iter().all(|&b| b >= 0.0 && b < full * 0.5));
    let max_black = black.map(|v| v.into_iter().fold(0.0f32, f32::max)).unwrap_or(0.0);
    let white = offsets.2.and_then(|at| values.get(at)).map(|&v| v as f32).filter(|&v| v > max_black && v >= full * 0.5 && v <= full);
    CameraLevels { wb, black, white }
}

fn black_at_active(levels: [f32; 4], pattern: u8, active: Rect) -> BlackLevel {
    // Convert Canon's channel order to full-sensor site order, then shift to the active origin.
    let sites = match pattern {
        1 => [1, 0, 3, 2],
        2 => [2, 3, 0, 1],
        3 => [3, 2, 1, 0],
        _ => [0, 1, 2, 3],
    };
    let values = (0..4)
        .map(|i| {
            let x = (active.x + i % 2) % 2;
            let y = (active.y + i / 2) % 2;
            levels.get(*sites.get(y * 2 + x).unwrap_or(&0)).copied().unwrap_or(0.0)
        })
        .collect();
    BlackLevel { repeat_rows: 2, repeat_cols: 2, values, delta_h: vec![], delta_v: vec![] }
}

pub(crate) fn decode(bytes: &[u8], mode: Mode, limits: &Limits) -> Result<RawImage> {
    let container = parse_cr3(bytes).ok_or(RawError::NotRaw)?;
    let track = main_track(&container.tracks).ok_or_else(|| RawError::Unsupported("CR3 without a Bayer raw track".into()))?;
    let coding = track.compression(bytes).ok_or_else(|| RawError::Corrupt("CR3 raw track has an invalid CMP1 descriptor".into()))?;
    if coding.planes != 4 || coding.encoding != 0 {
        return Err(RawError::Unsupported(format!("Canon CR3 encoding {} with {} planes", coding.encoding, coding.planes)));
    }
    let (width, height) = (coding.width as usize, coding.height as usize);
    let samples = width.checked_mul(height).filter(|&n| n > 0 && n <= limits.samples()).ok_or(RawError::Limit("CR3 sensor dimensions"))?;
    let cfa = cfa(coding.cfa_pattern).ok_or_else(|| RawError::Unsupported(format!("Canon CR3 CFA {}", coding.cfa_pattern)))?;
    let (offset, len) = track.data.ok_or_else(|| RawError::Corrupt("CR3 raw sample outside file".into()))?;
    let sample =
        offset.checked_add(len).and_then(|end| bytes.get(offset..end)).ok_or_else(|| RawError::Corrupt("CR3 raw sample outside file".into()))?;
    if coding.header_size as usize > sample.len() {
        return Err(RawError::Corrupt("CR3 coding header outside sample".into()));
    }
    crx::validate(sample, &coding)?;
    let data = match mode {
        Mode::Full => {
            let data = crx::decode(sample, &coding)?;
            if data.len() != samples {
                return Err(RawError::Corrupt("CR3 decoded sample count does not match CMP1".into()));
            }
            data
        }
        Mode::Header => vec![],
    };
    let static_maker = container.cmt.get(2).and_then(|b| *b).and_then(|b| Tiff::parse(b).ok());
    let maker = static_maker.as_ref().and_then(|m| m.ifds.first());
    // Retain only the first useful timed maker note: repeated CTMD records must not accumulate
    // independent copies of every bounded TIFF value in memory.
    // Records aliasing the same bytes are parsed once, and all parses share a budget of twice the file (at least 1 MiB);
    // a record past what is left is skipped, a later smaller one still parsed.
    let mut seen = std::collections::HashSet::new();
    let mut left = bytes.len().saturating_mul(2).max(1 << 20);
    let timed_maker = container
        .timed_exif(bytes)
        .into_iter()
        .filter(|b| b.tag == 0x927c)
        .filter(|b| seen.insert((b.data.as_ptr() as usize, b.data.len())))
        .filter(|b| match left.checked_sub(b.data.len()) {
            Some(rest) => {
                left = rest;
                true
            }
            None => false,
        })
        .filter_map(|b| Tiff::parse(b.data).ok())
        .find(|t| t.ifds.first().is_some_and(|m| m.contains(COLOR_DATA)));
    let color_maker = timed_maker.as_ref().and_then(|t| t.ifds.first()).or(maker);
    let area = track.image_area(bytes);
    let (active, crop) = geometry(area.as_ref(), maker, width, height);
    let levels = camera_levels(color_maker, coding.bit_depth);
    let black = match levels.black {
        Some(levels) => black_at_active(levels, coding.cfa_pattern, active),
        None if active.x >= 8 => black_from_columns(&data, width, 2..active.x - 2, active.y..active.y + active.height, active),
        None => BlackLevel::uniform(0.0),
    };
    let white = levels.white.unwrap_or_else(|| white_from_data(&data, u32::from(coding.bit_depth)));
    // CMT1 is the file's IFD0: orientation, make and model
    let cmt1 = container.cmt.first().and_then(|b| *b).and_then(|b| Tiff::parse(b).ok());
    let ifd0 = cmt1.as_ref().and_then(|t| t.ifds.first());
    let mut image = RawImage {
        format: RawFormat::Cr3,
        width,
        height,
        cpp: 1,
        data: RawData::U16(data),
        cfa: Some(cfa),
        bits: u32::from(coding.bit_depth),
        black,
        white: vec![white],
        active_area: active,
        crop,
        orientation: Orientation::from_exif(ifd0.and_then(|i| i.u16(craft_tiff::tags::ORIENTATION)).unwrap_or(1)),
        color: ColorData::default(),
        wb_multipliers: levels.wb,
        linearized: false,
        pixel_aspect: 1.0,
        opcodes: OpcodeLists::default(),
        camera: ifd0.map(Camera::from_ifd0).unwrap_or_default(),
        cinema: Default::default(),
        lens_metadata: Default::default(),
        level_sources: Default::default(),
    };
    // Canon's stated white level can sit below the sensor's true saturation (EOS RP: 14558
    // stated, 16383 real). Extend it to a saturation pile-up above it, so highlight data
    // between the stated level and the real clip point is not treated as clipped. No-op when
    // no such pile-up exists, and for header-only decodes (no samples).
    extend_white(&mut image);
    image.validate_for(mode)?;
    Ok(image)
}

// The white extension below is LightCraft's `saturation.rs` (CR3 part).
/// A saturation plateau must be at least this many times as populated as the average value just below it.
const MIN_SPIKE: f64 = 16.0;
/// At most this share of the plateau may lie above it (a real clip point is the top of the data).
const MAX_ABOVE: f64 = 0.02;
// CR3 gain-normalized saturation can occupy several nearby codes, rather than one exact value.
const CR3_BINS: usize = 256;
const CR3_PLATEAU_BINS: usize = 8;
const CR3_MIN_SAMPLES: u64 = 64;

/// The lower edge of a terminal pile-up, ignoring a few brighter outliers. A continuous
/// highlight distribution has comparable density below it and therefore has no plateau.
fn cr3_plateau(hist: &[[u64; 3]; CR3_BINS], first: &[[u16; 3]; CR3_BINS], colour: usize, from: usize, white: f32) -> Option<f32> {
    let (start, end, count) = (from + 1..=CR3_BINS)
        .map(|end| {
            let start = end.saturating_sub(CR3_PLATEAU_BINS).max(from);
            let count: u64 = hist[start..end].iter().map(|bin| bin[colour]).sum();
            (start, end, count)
        })
        .max_by_key(|&(_, end, count)| (count, end))?;
    let below_start = start.saturating_sub(CR3_PLATEAU_BINS);
    let below: u64 = hist[below_start..start].iter().map(|bin| bin[colour]).sum();
    let above: u64 = hist[end..].iter().map(|bin| bin[colour]).sum();
    let density = count as f64 / (end - start) as f64;
    let below_density = below as f64 / (start - below_start).max(1) as f64;
    if count < CR3_MIN_SAMPLES || (below > 0 && density < MIN_SPIKE * below_density) || above as f64 > MAX_ABOVE * count as f64 {
        return None;
    }
    let threshold = (density / MIN_SPIKE).max(MIN_SPIKE * below_density).max(1.0);
    (start..end).find(|&bin| hist[bin][colour] as f64 >= threshold).map(|bin| first[bin][colour] as f32).filter(|&point| point > white)
}

/// Recover real CR3 samples beyond a conservatively stated common white level. Search
/// coarse bins so gain-spread saturation (e.g. R100) is found too. With enough above-white
/// data but no clear pile-up, the bit-depth ceiling keeps all that unclipped headroom.
/// One shared range and its compensating baseline EV preserve lower tones and colour ratios.
fn extend_white(image: &mut RawImage) -> bool {
    if image.format != crate::RawFormat::Cr3 || !image.color.baseline_exposure.is_finite() || !(1..=16).contains(&image.bits) {
        return false;
    }
    let (RawData::U16(data), Some(cfa)) = (&image.data, &image.cfa) else { return false };
    let (old, black) = (image.white_at(0), image.black.mean());
    let ceiling = ((1u32 << image.bits) - 1) as f32;
    if image.cpp != 1
        || !cfa.valid()
        || image.width.checked_mul(image.height) != Some(data.len())
        || !black.is_finite()
        || !old.is_finite()
        || black < 0.0
        || old <= black
        || old >= ceiling
        || image.white.is_empty()
        || image.white.iter().any(|w| *w != old)
    {
        return false;
    }
    let a = image.active_area;
    let fits = |start: usize, len: usize, max: usize| start.checked_add(len).is_some_and(|end| end <= max);
    if a.width == 0 || a.height == 0 || !fits(a.x, a.width, image.width) || !fits(a.y, a.height, image.height) {
        return false;
    }
    let step = ((ceiling - black) / CR3_BINS as f32).ceil().max(1.0);
    let mut hist = [[0u64; 3]; CR3_BINS];
    let mut first = [[u16::MAX; 3]; CR3_BINS];
    let mut over = [0u64; 3];
    for y in a.y..a.y + a.height {
        for (x, &v) in data[y * image.width + a.x..][..a.width].iter().enumerate() {
            if v as f32 > ceiling {
                return false;
            }
            let c = cfa.color_at(a.x + x, y).min(2) as usize;
            let bin = (((v as f32 - black).max(0.0) / step) as usize).min(CR3_BINS - 1);
            hist[bin][c] += 1;
            first[bin][c] = first[bin][c].min(v);
            over[c] += u64::from(v as f32 > old);
        }
    }
    let above: u64 = over.iter().sum();
    let pixels = a.width * a.height;
    if above < CR3_MIN_SAMPLES || (above as f64) < pixels as f64 * 1e-5 {
        return false;
    }
    let from = (((old - black) / step) as usize).min(CR3_BINS - 1);
    let new = (0..3)
        .map(|c| if over[c] < CR3_MIN_SAMPLES { ceiling } else { cr3_plateau(&hist, &first, c, from, old).unwrap_or(ceiling) })
        .fold(ceiling, f32::min);
    let ev = ((new - black) as f64 / (old - black) as f64).log2();
    if new <= old || !ev.is_finite() {
        return false;
    }
    image.white.fill(new);
    image.color.baseline_exposure += ev;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_tiff::{IfdBuilder, Value};

    fn maker(version: u16, wb_at: usize, black_at: usize, white_at: usize) -> Ifd {
        let mut words = vec![0u16; white_at + 1];
        words[0] = version;
        words[wb_at..wb_at + 4].copy_from_slice(&[2048, 1024, 1024, 1536]);
        words[black_at..black_at + 4].copy_from_slice(&[100, 101, 102, 103]);
        words[white_at] = 15000;
        let b = IfdBuilder::new().with(COLOR_DATA, Value::Short(words));
        let bytes = craft_tiff::TiffWriter::new(craft_tiff::ByteOrder::Little, false).write(&[b]).unwrap();
        Tiff::parse(&bytes).unwrap().ifds.remove(0)
    }

    fn box_bytes(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(kind);
        b.extend_from_slice(body);
        b
    }

    fn track(entry: &[u8], offset: u64, size: u32) -> Vec<u8> {
        let mut stsd = vec![0u8; 4];
        stsd.extend(1u32.to_be_bytes());
        stsd.extend(entry);
        let mut stsz = vec![0u8; 4];
        stsz.extend(size.to_be_bytes());
        stsz.extend(1u32.to_be_bytes());
        let mut co64 = vec![0u8; 4];
        co64.extend(1u32.to_be_bytes());
        co64.extend(offset.to_be_bytes());
        let tables = [box_bytes(b"stsd", &stsd), box_bytes(b"stsz", &stsz), box_bytes(b"co64", &co64)].concat();
        box_bytes(b"trak", &box_bytes(b"mdia", &box_bytes(b"minf", &box_bytes(b"stbl", &tables))))
    }

    fn synthetic_header() -> Vec<u8> {
        synthetic_header_with(None)
    }

    fn synthetic_header_with(aspect: Option<[u32; 5]>) -> Vec<u8> {
        use craft_tiff::{ByteOrder, TiffWriter, tags as t};
        let tiff = |b: IfdBuilder| TiffWriter::new(ByteOrder::Little, false).write(&[b]).unwrap();
        let cmt1 = tiff(
            IfdBuilder::new()
                .with(t::MAKE, Value::Ascii("Canon".into()))
                .with(t::MODEL, Value::Ascii("EOS Test".into()))
                .with(t::ORIENTATION, Value::Short(vec![6])),
        );
        let cmt2 = tiff(IfdBuilder::new().with(t::ISO_SPEED, Value::Short(vec![800])));
        let mut cmt3 = IfdBuilder::new()
            .with(SENSOR_INFO, Value::Short(vec![34, 64, 48, 1, 1, 6, 4, 61, 45]))
            .with(0x0095, Value::Ascii("RF-S18-45mm F4.5-6.3 IS STM".into()));
        if let Some(a) = aspect {
            cmt3 = cmt3.with(ASPECT_INFO, Value::Long(a.to_vec()));
        }
        let cmt3 = tiff(cmt3);
        let mut canon = vec![0x85, 0xc0, 0xb6, 0x87, 0x82, 0x0f, 0x11, 0xe0, 0x81, 0x11, 0xf4, 0xce, 0x46, 0x2b, 0x6a, 0x48];
        canon.extend([box_bytes(b"CMT1", &cmt1), box_bytes(b"CMT2", &cmt2), box_bytes(b"CMT3", &cmt3)].concat());
        let mut coding = vec![0u8; 52];
        coding[0..2].copy_from_slice(&0xff00u16.to_be_bytes());
        coding[2..4].copy_from_slice(&48u16.to_be_bytes());
        coding[4..6].copy_from_slice(&0x100u16.to_be_bytes());
        for (at, value) in [(8, 64u32), (12, 48), (16, 32), (20, 48), (28, 216)] {
            coding[at..at + 4].copy_from_slice(&value.to_be_bytes());
        }
        coding[24] = 14;
        coding[25] = 0x40;
        let mut area = vec![0u8; 48];
        for (at, value) in [
            (4, 64u16),
            (6, 48),
            (10, 2),
            (16, 6),
            (18, 4),
            (20, 61),
            (22, 45),
            (28, 3),
            (30, 47),
            (32, 4),
            (36, 63),
            (38, 1),
            (40, 4),
            (42, 2),
            (44, 63),
            (46, 47),
        ] {
            area[at..at + 2].copy_from_slice(&value.to_be_bytes());
        }
        let mut craw = vec![0u8; 82];
        craw[24..26].copy_from_slice(&64u16.to_be_bytes());
        craw[26..28].copy_from_slice(&48u16.to_be_bytes());
        craw.extend(box_bytes(b"CMP1", &coding));
        craw.extend(box_bytes(b"CDI1", &[vec![0u8; 4], box_bytes(b"IAD1", &area)].concat()));
        let levels = maker(48, 105, 363, 641).value(COLOR_DATA).unwrap().clone();
        let dynamic_maker = tiff(IfdBuilder::new().with(COLOR_DATA, levels));
        let mut item = ((dynamic_maker.len() + 8) as u32).to_le_bytes().to_vec();
        item.extend(0x927cu32.to_le_bytes());
        item.extend(dynamic_maker);
        let mut timed = vec![0u8; 12];
        timed[0..4].copy_from_slice(&((12 + item.len()) as u32).to_le_bytes());
        timed[4..6].copy_from_slice(&8u16.to_le_bytes());
        timed.extend(item);
        // Two tiles, each with four one-byte lossless plane streams. Header
        // probing checks the complete marker layout without decoding entropy.
        let mut sample = Vec::new();
        let marker = |out: &mut Vec<u8>, code: u16, size: u32, flags: u32| {
            out.extend(code.to_be_bytes());
            out.extend(8u16.to_be_bytes());
            out.extend(size.to_be_bytes());
            out.extend(flags.to_be_bytes());
        };
        for tile in 0..2 {
            marker(&mut sample, 0xff01, 4, tile << 16);
            for plane in 0..4 {
                marker(&mut sample, 0xff02, 1, (plane << 28) | 0x0800_0000);
                marker(&mut sample, 0xff03, 1, 0x0020_0000);
            }
        }
        assert_eq!(sample.len(), 216);
        sample.extend([0x1e; 8]);
        let moov =
            [box_bytes(b"uuid", &canon), track(&box_bytes(b"CRAW", &craw), 4096, 224), track(&box_bytes(b"CTMD", &[]), 4320, timed.len() as u32)]
                .concat();
        let mut file = box_bytes(b"ftyp", b"crx \0\0\0\x01crx isom");
        file.extend(box_bytes(b"moov", &moov));
        assert!(file.len() < 4096);
        file.resize(4096, 0);
        file.extend(sample);
        file.extend(timed);
        file
    }

    #[test]
    fn header_reads_timed_white_balance_and_distinct_geometry() {
        let bytes = synthetic_header();
        let image = decode(&bytes, Mode::Header, &Limits::default()).unwrap();
        assert_eq!((image.width, image.height, image.bits), (64, 48, 14));
        assert_eq!(image.active_area, Rect::new(4, 2, 60, 46));
        assert_eq!(image.crop, Rect::new(2, 2, 56, 42));
        assert_eq!(image.wb_multipliers, Some([2.0, 1.0, 1.5]));
        assert_eq!(image.black.values, vec![100.0, 101.0, 102.0, 103.0]);
        assert_eq!(image.white, vec![15000.0]);
        assert_eq!(image.info().developed_size(), (56, 42));
        assert_eq!((image.camera.make.as_deref(), image.camera.model.as_deref()), (Some("Canon"), Some("EOS Test")));
        assert_eq!(image.orientation, crate::Orientation::Rotate90);
        assert!(image.data.is_empty());
        assert!(decode(&bytes[..4200], Mode::Header, &Limits::default()).is_err());
    }

    #[test]
    fn aspect_info_replaces_the_crop() {
        // 16:9 inside the 56 x 42 recommended crop at (2, 2): 56 x 30, 6 rows down.
        let bytes = synthetic_header_with(Some([7, 56, 30, 0, 6]));
        let image = decode(&bytes, Mode::Header, &Limits::default()).unwrap();
        assert_eq!(image.active_area, Rect::new(4, 2, 60, 46));
        assert_eq!(image.crop, Rect::new(2, 8, 56, 30));
        assert_eq!(image.info().developed_size(), (56, 30));
        // 1:1 with a left offset.
        let image = decode(&synthetic_header_with(Some([1, 42, 42, 7, 0])), Mode::Header, &Limits::default()).unwrap();
        assert_eq!(image.crop, Rect::new(9, 2, 42, 42));
        // The whole image, as stored for a shot at the sensor's own aspect, changes nothing.
        let image = decode(&synthetic_header_with(Some([0, 56, 42, 0, 0])), Mode::Header, &Limits::default()).unwrap();
        assert_eq!(image.crop, Rect::new(2, 2, 56, 42));
    }

    #[test]
    fn aspect_info_that_does_not_fit_is_ignored() {
        for bad in [[7, 57, 30, 0, 6], [7, 56, 30, 0, 13], [7, 56, 30, 1, 6], [7, 1, 30, 0, 0], [7, 56, 0, 0, 0], [7, u32::MAX, 30, 0, 0]] {
            let image = decode(&synthetic_header_with(Some(bad)), Mode::Header, &Limits::default()).unwrap();
            assert_eq!(image.crop, Rect::new(2, 2, 56, 42), "{bad:?}");
        }
    }

    /// Some crop-mode files (EOS R5 Mark II 7883) record a recommended crop that reaches past their stated active
    /// area, which starts in dark columns. The recommended crop is then the valid area (no black band on the left),
    /// and `AspectInfo` applies from its origin.
    #[test]
    fn aspect_info_needs_a_consistent_recommended_crop() {
        let aspect = craft_tiff::IfdBuilder::new().with(ASPECT_INFO, Value::Long(vec![13, 5088, 3392, 0, 0]));
        let bytes = craft_tiff::TiffWriter::new(craft_tiff::ByteOrder::Little, false).write(&[aspect]).unwrap();
        let maker = Tiff::parse(&bytes).unwrap().ifds.remove(0);
        let area =
            |crop| Cr3ImageArea { width: 5376, height: 3574, crop, active: Some([132, 160, 5243, 3567]), masked_left: [0; 4], masked_top: None };
        // crop right edge 5359 > active right edge 5243: the recommended crop becomes the valid area
        assert_eq!(
            geometry(Some(&area([272, 172, 5359, 3563])), Some(&maker), 5376, 3574),
            (Rect::new(272, 172, 5088, 3392), Rect::new(0, 0, 5088, 3392))
        );
        // consistent crop: AspectInfo applies from its origin
        assert_eq!(geometry(Some(&area([140, 172, 5227, 3563])), Some(&maker), 5376, 3574).1, Rect::new(8, 12, 5088, 3392));
    }

    /// `AspectInfo` as the SX70 HS 16:9 sample stores it, byte for byte (little-endian LONG x 5), read through
    /// the TIFF parser: AspectRatio 7, 5184 x 2912 at (0, 488).
    #[test]
    fn aspect_info_known_bytes() {
        let mut tiff = b"II*     ".to_vec();
        tiff.extend_from_slice(&[0x9a, 0x00, 0x04, 0x00, 0x05, 0x00, 0x00, 0x00, 0x1a, 0x00, 0x00, 0x00]);
        tiff.extend_from_slice(&[0, 0, 0, 0]);
        tiff.extend_from_slice(&[0x07, 0, 0, 0, 0x40, 0x14, 0, 0, 0x60, 0x0b, 0, 0, 0, 0, 0, 0, 0xe8, 0x01, 0, 0]);
        let parsed = Tiff::parse(&tiff).unwrap();
        let maker = parsed.ifds.first();
        assert_eq!(aspect_crop(maker, Rect::new(132, 40, 5184, 3888)), Some(Rect::new(132, 528, 5184, 2912)));
        assert_eq!(aspect_crop(maker, Rect::new(0, 0, 5184, 2000)), None);
        assert_eq!(aspect_crop(None, Rect::new(0, 0, 5184, 3888)), None);
    }

    fn prepend_tracks(bytes: &[u8], tracks: &[u8]) -> Vec<u8> {
        let ftyp_end = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
        let moov_len = u32::from_be_bytes(bytes[ftyp_end..ftyp_end + 4].try_into().unwrap()) as usize;
        assert_eq!(&bytes[ftyp_end + 4..ftyp_end + 8], b"moov");
        let moov = [tracks, &bytes[ftyp_end + 8..ftyp_end + moov_len]].concat();
        let mut out = [bytes[..ftyp_end].to_vec(), box_bytes(b"moov", &moov)].concat();
        assert!(out.len() < 4096);
        out.resize(4096, 0);
        out.extend_from_slice(&bytes[4096..]);
        out
    }

    #[test]
    fn hevc_preview_with_equal_dimensions_does_not_replace_the_sensor_track() {
        let bytes = synthetic_header();
        let mut preview = vec![0u8; 82];
        preview[24..26].copy_from_slice(&64u16.to_be_bytes());
        preview[26..28].copy_from_slice(&48u16.to_be_bytes());
        preview.extend(box_bytes(b"HEVC", &[0; 4]));
        let bytes = prepend_tracks(&bytes, &track(&box_bytes(b"CRAW", &preview), 4096, 224));
        let container = parse_cr3(&bytes).unwrap();
        assert_eq!(container.tracks[0].kind, Cr3TrackKind::Other(*b"HEVC"));
        let image = decode(&bytes, Mode::Header, &Limits::default()).unwrap();
        assert_eq!((image.width, image.height, image.bits), (64, 48, 14));
        assert_eq!(image.active_area, Rect::new(4, 2, 60, 46));
    }

    #[test]
    fn corrupt_main_descriptor_is_not_replaced_by_the_reduced_raw() {
        let bytes = synthetic_header();
        let container = parse_cr3(&bytes).unwrap();
        let Cr3TrackKind::Raw { cmp1: Some((at, len)), .. } = main_track(&container.tracks).unwrap().kind else { panic!("raw fixture") };
        let mut reduced = vec![0u8; 82];
        reduced[24..26].copy_from_slice(&32u16.to_be_bytes());
        reduced[26..28].copy_from_slice(&24u16.to_be_bytes());
        reduced.extend(box_bytes(b"CMP1", &bytes[at..at + len]));
        let bytes = prepend_tracks(&bytes, &track(&box_bytes(b"CRAW", &reduced), 4096, 224));
        let container = parse_cr3(&bytes).unwrap();
        let Cr3TrackKind::Raw { cmp1: Some((at, len)), .. } = main_track(&container.tracks).unwrap().kind else { panic!("main raw fixture") };
        let mut malformed = bytes.clone();
        malformed[at..at + len].fill(0);
        assert!(matches!(decode(&malformed, Mode::Header, &Limits::default()), Err(RawError::Corrupt(_))));
        let mut missing = bytes;
        missing[at - 4..at].copy_from_slice(b"free");
        assert!(matches!(decode(&missing, Mode::Header, &Limits::default()), Err(RawError::Corrupt(_))));
    }

    #[test]
    fn color_data_uses_versioned_as_shot_offsets() {
        for (version, wb_at, black_at, white_at) in
            [(16, 71, 329, 797), (19, 71, 329, 797), (33, 85, 343, 811), (48, 105, 363, 641), (64, 105, 383, 661)]
        {
            let m = maker(version, wb_at, black_at, white_at);
            let levels = camera_levels(Some(&m), 14);
            assert_eq!(levels.wb, Some([2.0, 1.0, 1.5]));
            assert_eq!(levels.black, Some([100.0, 101.0, 102.0, 103.0]));
            assert_eq!(levels.white, Some(15000.0));
        }
        // A version without a white-level offset leaves the white level to the data.
        let levels = camera_levels(Some(&maker(66, 105, 383, 661)), 14);
        assert_eq!(levels.wb, Some([2.0, 1.0, 1.5]));
        assert_eq!(levels.black, Some([100.0, 101.0, 102.0, 103.0]));
        assert_eq!(levels.white, None);
        assert!(camera_levels(Some(&maker(255, 71, 329, 797)), 14).wb.is_none());
        assert!(camera_levels(None, 14).black.is_none());
    }

    #[test]
    fn crop_is_relative_to_the_valid_sensor_area() {
        let area = Cr3ImageArea {
            width: 6888,
            height: 4546,
            crop: [156, 158, 6875, 4537],
            active: Some([144, 46, 6887, 4545]),
            masked_left: [0, 0, 143, 4545],
            masked_top: None,
        };
        assert_eq!(geometry(Some(&area), None, 6888, 4546), (Rect::new(144, 46, 6744, 4500), Rect::new(12, 112, 6720, 4380)));
        assert_eq!(inclusive_rect([152, 46, 4353, 2851], 4352, 2850), Some(Rect::new(152, 46, 4200, 2804)));
        assert_eq!(inclusive_rect([100, 100, 50, 50], 200, 200), None);
        assert_eq!(relative_crop(Rect::new(0, 0, 100, 100), Rect::new(4, 4, 92, 92)), None);
    }

    #[test]
    fn first_full_size_track_wins_over_dual_pixel_delta() {
        let raw = |w, h, offset| Cr3Track { kind: Cr3TrackKind::Raw { width: w, height: h, cmp1: None, iad1: None }, data: Some((offset, 100)) };
        let tracks = [raw(1624, 1080, 1), raw(6888, 4546, 2), raw(6888, 4546, 3)];
        assert_eq!(main_track(&tracks).unwrap().data, Some((2, 100)));
    }

    #[test]
    fn cfa_and_black_follow_the_full_sensor_origin() {
        for (code, name) in [(0, "RGGB"), (1, "GRBG"), (2, "GBRG"), (3, "BGGR")] {
            assert_eq!(cfa(code).unwrap().name(), name);
        }
        let b = black_at_active([100.0, 101.0, 102.0, 103.0], 0, Rect::new(3, 1, 32, 20));
        assert_eq!(b.values, vec![103.0, 102.0, 101.0, 100.0]);
    }

    /// A 64 x 64 RGGB CR3 image with a gradient (black 528, 14 bits) stated to clip at `white`.
    fn levels_image(white: f32) -> RawImage {
        let data = (0..64 * 64).map(|i| 528 + ((i % 64) * 60 + (i / 64) * 7) as u16 % 4000).collect();
        RawImage {
            format: RawFormat::Cr3,
            width: 64,
            height: 64,
            cpp: 1,
            data: RawData::U16(data),
            cfa: Cfa::bayer("RGGB"),
            bits: 14,
            black: BlackLevel::uniform(528.0),
            white: vec![white],
            active_area: Rect::new(0, 0, 64, 64),
            crop: Rect::new(0, 0, 64, 64),
            orientation: Orientation::Normal,
            color: ColorData::default(),
            wb_multipliers: None,
            linearized: false,
            pixel_aspect: 1.0,
            opcodes: OpcodeLists::default(),
            camera: Camera::default(),
            cinema: Default::default(),
            lens_metadata: Default::default(),
            level_sources: Default::default(),
        }
    }

    /// Canon's stated white (EOS RP: 14558) below a pile-up at 16383 is extended to it, the samples unchanged and
    /// the BaselineExposure raised by the stops that keep unclipped tones where they were; once only.
    #[test]
    fn white_extends_to_a_saturation_plateau_above_the_stated_level() {
        let mut r = levels_image(14558.0);
        if let RawData::U16(d) = &mut r.data {
            d[..2048].fill(16383);
        }
        r.color.baseline_exposure = 0.5;
        let samples = r.data.clone();
        assert!(extend_white(&mut r));
        assert_eq!(r.white_at(0), 16383.0);
        assert!((r.color.baseline_exposure - (0.5 + ((16383.0f64 - 528.0) / (14558.0 - 528.0)).log2())).abs() < 1e-6);
        assert_eq!(r.data, samples);
        let ev = r.color.baseline_exposure;
        assert!(!extend_white(&mut r), "idempotent");
        assert_eq!(r.color.baseline_exposure, ev);
    }

    #[test]
    fn white_extension_leaves_other_formats_and_unequal_levels_alone() {
        let mut r = levels_image(14558.0);
        if let RawData::U16(d) = &mut r.data {
            d[..2048].fill(16383);
        }
        r.format = RawFormat::Dng;
        assert!(!extend_white(&mut r));
        assert_eq!(r.white_at(0), 14558.0);
        r.format = RawFormat::Cr3;
        r.white = vec![14558.0, 15000.0, 14558.0];
        assert!(!extend_white(&mut r));
        assert_eq!(r.color.baseline_exposure, 0.0);
    }

    #[test]
    fn white_extension_finds_a_spread_plateau() {
        let mut r = levels_image(10439.0);
        if let RawData::U16(d) = &mut r.data {
            for (i, v) in d.iter_mut().take(3072).enumerate() {
                *v = 13300 + (i * 37 % 401) as u16;
            }
        }
        assert!(extend_white(&mut r), "the R100-style plateau spans hundreds of codes");
        assert!((13300.0..=13364.0).contains(&r.white_at(0)), "{}", r.white_at(0));
    }

    #[test]
    fn white_extension_keeps_unclipped_data_past_the_stated_level() {
        let mut r = levels_image(14558.0);
        if let RawData::U16(d) = &mut r.data {
            for (i, v) in d.iter_mut().enumerate() {
                *v = 528 + (i * 37 % 15855) as u16;
            }
        }
        assert!(extend_white(&mut r), "unclipped data past the stated white is still real light");
        assert_eq!(r.white_at(0), 16383.0);
        let mut r = levels_image(14558.0);
        let cfa = r.cfa.clone().unwrap();
        if let RawData::U16(d) = &mut r.data {
            for (i, v) in d.iter_mut().enumerate() {
                if cfa.color_at(i % 64, i / 64) == 0 {
                    *v = 16383;
                }
            }
        }
        assert!(extend_white(&mut r), "a coloured highlight need not clip all three channels");
        assert_eq!(r.white_at(0), 16383.0);
    }
}
