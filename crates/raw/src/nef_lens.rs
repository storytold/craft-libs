//! Nikon's in-file distortion correction (raw SubIFD tag `0xc7d5`, NikonNEFInfo), expressed as a DNG warp for
//! the shared optics pipeline.
//!
//! Tag identity: the ExifTool Nikon tag-name documentation (NikonNEFInfo `0xc7d5`; its DistortionInfo `0x0005`
//! with DistortionCorrection and RadialDistortionCoefficient1..3). The model below (which stored value multiplies
//! which power, the normalisation, the fill-frame zoom and the optical-centre offset in entry `0x0008`) was inferred
//! independently by matching the raw against the camera's own corrected JPEG; see docs/nikon-lens-corrections.md.
//! No lens database or external decoder code.

use crate::{Opcode, Rect};
use craft_tiff::{ByteOrder, Ifd, ParseOptions, parse_ifd_at};

const NEF_INFO: u16 = 0xc7d5;
const DISTORTION_INFO: u16 = 0x0005;
const CENTRE_OFFSET: u16 = 0x0008;
/// Bodies whose files were checked against the camera's JPEG (docs/nikon-lens-corrections.md). Matching tags on
/// another body do not establish its pixel pitch or image area; add a body only after checking its files.
const VALIDATED: &[&str] = &["NIKON Z 6_2", "NIKON Z 8", "NIKON Z f"];
/// The normalisation radius every checked file stores: the FX half-diagonal in millimetres.
const FX_HALF_DIAGONAL_MM: f64 = 21.6;
const SAMPLES: usize = 256;

/// `r_src = r_out * (1 + k3 rho^2 + k2 rho^4 + k1 rho^6) / (1 + zoom)` around the optical centre, `rho` the radius
/// over the half-diagonal of the image area (`k1..k3` as ExifTool names them, in the order they are stored).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Distortion {
    k: [f64; 3],
    zoom: f64,
}

/// A signed rational at `at` (`i32 / i32`), when it is one.
fn rational(b: &[u8], at: usize, order: ByteOrder) -> Option<f64> {
    let n = order.u32(b.get(at..at + 4)?.try_into().ok()?) as i32;
    let d = order.u32(b.get(at + 4..at + 8)?.try_into().ok()?) as i32;
    (d > 0).then(|| f64::from(n) / f64::from(d)).filter(|v| v.is_finite())
}

/// The NikonNEFInfo entries: the blob is `"Nikon\0"`, a 4-byte version, then a little TIFF stream.
fn nef_info(raw: &Ifd) -> Option<(Ifd, ByteOrder)> {
    let tiff = raw.bytes(NEF_INFO)?.strip_prefix(b"Nikon\0")?.get(4..)?;
    let (order, false) = craft_tiff::Tiff::sniff(tiff)? else { return None };
    let first = order.u32(tiff.get(4..8)?.try_into().ok()?);
    let opts = ParseOptions { follow_children: false, ..Default::default() };
    let (ifd, _) = parse_ifd_at(tiff, u64::from(first), order, 0, false, &opts).ok()?;
    Some((ifd, order))
}

/// DistortionInfo: `"0100"`, the correction mode (0 none), the normalisation radius (mm) at 8, the term count (4)
/// at 16, the coefficients at 20 / 28 / 36 and the zoom at 64, each a signed rational.
fn distortion_info(b: &[u8], order: ByteOrder) -> Option<Distortion> {
    if b.get(0..4)? != b"0100" || *b.get(4)? == 0 {
        return None;
    }
    let norm = rational(b, 8, order)?;
    let terms = order.u32(b.get(16..20)?.try_into().ok()?);
    if (norm - FX_HALF_DIAGONAL_MM).abs() > 0.05 || terms != 4 {
        return None;
    }
    let k = [rational(b, 20, order)?, rational(b, 28, order)?, rational(b, 36, order)?];
    let zoom = rational(b, 64, order)?;
    // real lenses stay far inside these (the checked files: |k| < 0.13, |zoom| < 0.09)
    (k.iter().all(|v| v.abs() < 1.0) && zoom.abs() < 0.5).then_some(Distortion { k, zoom })
}

/// Entry `0x0008`: `"0100"`, then the optical centre's offset from the image-area centre in pixels (x right,
/// y down), two signed rationals. Zero when the lens stabilises (the sensor stays centred).
fn centre_offset(b: &[u8], order: ByteOrder) -> Option<(f64, f64)> {
    (b.get(0..4)? == b"0100").then_some(())?;
    Some((rational(b, 4, order)?, rational(b, 12, order)?))
}

/// The correction of a `model` NEF whose raw IFD is `raw`, for the image area `crop` inside `active` (both in
/// sensor pixels), as an `OpcodeList3` warp in active-area coordinates. `None` for unvalidated bodies, other
/// image areas, missing or implausible data, and maps that fold.
pub(super) fn distortion(model: &str, raw: &Ifd, active: Rect, crop: Rect) -> Option<Opcode> {
    if !VALIDATED.contains(&model.trim()) {
        return None;
    }
    let crop = crop.clipped(active.width, active.height);
    if active.width < 2 || active.height < 2 || crop.width < 2 || crop.height < 2 {
        return None;
    }
    // the stored radius is the FX half-diagonal: only the full 3:2 image area (not a DX or other crop of the
    // sensor) is normalised by it
    if (crop.width as f64 / crop.height as f64 - 1.5).abs() > 0.005 || (crop.width as f64) < active.width as f64 * 0.95 {
        return None;
    }
    let (info, order) = nef_info(raw)?;
    let d = distortion_info(info.bytes(DISTORTION_INFO)?, order)?;
    let (ox, oy) = info.bytes(CENTRE_OFFSET).and_then(|b| centre_offset(b, order)).unwrap_or((0.0, 0.0));
    let (cw, ch) = (crop.width as f64 - 1.0, crop.height as f64 - 1.0);
    if ox.abs() > cw * 0.05 || oy.abs() > ch * 0.05 {
        return None;
    }
    let half_diagonal = cw.hypot(ch) / 2.0;
    let poly = |rho: f64| {
        let x = rho * rho;
        1.0 + x * (d.k[2] + x * (d.k[1] + x * d.k[0]))
    };
    // DNG normalises by the farthest active-area corner from the centre; the image area reaches at most there
    let (ax, ay) = (active.width as f64 - 1.0, active.height as f64 - 1.0);
    let (cx, cy) = (crop.x as f64 + cw / 2.0 + ox, crop.y as f64 + ch / 2.0 + oy);
    let farthest = cx.max(ax - cx).hypot(cy.max(ay - cy));
    let a = farthest / half_diagonal;
    // the source radius must grow with the output radius out to the farthest corner, or the map folds
    let mut previous = -1.0;
    for j in 0..=SAMPLES {
        let rho = a * j as f64 / SAMPLES as f64;
        let source = rho * poly(rho) / (1.0 + d.zoom);
        if !source.is_finite() || source <= previous {
            return None;
        }
        previous = source;
    }
    let s = 1.0 / (1.0 + d.zoom);
    let a2 = a * a;
    let planes = vec![[s, d.k[2] * a2 * s, d.k[1] * a2 * a2 * s, d.k[0] * a2 * a2 * a2 * s, 0.0, 0.0]];
    Some(Opcode::WarpRectilinear { planes, center: [cx / ax, cy / ay] })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use craft_tiff::{Entry, IfdBuilder, TiffWriter, Value};

    const AREA: Rect = Rect { x: 0, y: 0, width: 6064, height: 4040 };
    const CROP: Rect = Rect { x: 8, y: 8, width: 6048, height: 4024 };

    fn rat(v: f64, den: i32) -> [u8; 8] {
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&((v * f64::from(den)).round() as i32).to_le_bytes());
        b[4..].copy_from_slice(&den.to_le_bytes());
        b
    }

    /// DistortionInfo as the checked files lay it out (84 bytes, little-endian).
    pub(crate) fn distortion_block(mode: u8, norm: f64, terms: u32, k: [f64; 3], zoom: f64) -> Vec<u8> {
        let mut b = vec![0u8; 84];
        b[..4].copy_from_slice(b"0100");
        b[4] = mode;
        b[5] = 1;
        b[8..16].copy_from_slice(&rat(norm, 4096));
        b[16..20].copy_from_slice(&terms.to_le_bytes());
        for (i, v) in k.iter().enumerate() {
            b[20 + 8 * i..28 + 8 * i].copy_from_slice(&rat(*v, 1 << 20));
        }
        b[44..52].copy_from_slice(&rat(0.0, 1));
        b[52..60].copy_from_slice(&rat(0.0, 1));
        b[64..72].copy_from_slice(&rat(zoom, 1 << 20));
        b
    }

    pub(crate) fn offset_block(x: f64, y: f64) -> Vec<u8> {
        let mut b = b"0100".to_vec();
        b.extend_from_slice(&rat(x, 256));
        b.extend_from_slice(&rat(y, 256));
        b.extend_from_slice(&[0, 0, 0, 0]);
        b
    }

    /// A NikonNEFInfo (`0xc7d5`) blob with the given entries.
    pub(crate) fn nef_info_blob(entries: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let mut info = IfdBuilder::new();
        for (tag, data) in entries {
            info.set(*tag, Value::Undefined(data.clone()));
        }
        let tiff = TiffWriter::new(ByteOrder::Little, false).write(&[info]).unwrap();
        let mut blob = b"Nikon\0\x01\x03\0\0".to_vec();
        blob.extend_from_slice(&tiff);
        blob
    }

    /// A raw IFD carrying a NikonNEFInfo blob with the given entries.
    pub(super) fn raw_with(entries: &[(u16, Vec<u8>)]) -> Ifd {
        Ifd { entries: vec![Entry { tag: NEF_INFO, value: Value::Undefined(nef_info_blob(entries)), offset: 0 }], ..Default::default() }
    }

    // a Z 24-70 f/4 S at 24 mm and at 70 mm, as stored in two of the checked files
    const WIDE: ([f64; 3], f64) = ([0.039895, 0.019211, -0.127218], -0.025043);
    const TELE: ([f64; 3], f64) = ([0.01681, -0.010601, 0.056758], 0.088301);

    fn warp(model: &str, raw: &Ifd) -> Option<([f64; 6], [f64; 2])> {
        match distortion(model, raw, AREA, CROP)? {
            Opcode::WarpRectilinear { planes, center } => Some((planes[0], center)),
            _ => None,
        }
    }

    /// The source radius the warp gives for an output radius `rho` (in image-area half-diagonals) about the centre.
    fn source_rho(p: [f64; 6], centre: [f64; 2], rho: f64) -> f64 {
        let (ax, ay) = (AREA.width as f64 - 1.0, AREA.height as f64 - 1.0);
        let (cx, cy) = (centre[0] * ax, centre[1] * ay);
        let m = cx.max(ax - cx).hypot(cy.max(ay - cy));
        let hd = (CROP.width as f64 - 1.0).hypot(CROP.height as f64 - 1.0) / 2.0;
        let t2 = (rho * hd / m).powi(2);
        rho * (p[0] + t2 * (p[1] + t2 * (p[2] + t2 * p[3])))
    }

    #[test]
    fn the_warp_is_the_stored_polynomial_over_the_zoom() {
        for (k, zoom) in [WIDE, TELE] {
            let raw = raw_with(&[(DISTORTION_INFO, distortion_block(3, 21.6, 4, k, zoom))]);
            let (p, c) = warp("NIKON Z 6_2", &raw).unwrap();
            for rho in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let x = rho * rho;
                let want = rho * (1.0 + k[2] * x + k[1] * x * x + k[0] * x * x * x) / (1.0 + zoom);
                assert!((source_rho(p, c, rho) - want).abs() < 2e-6, "{k:?} at {rho}");
            }
            // centred on the image area, which sits 8 px in from the sensor's top-left
            assert!((c[0] - (8.0 + 6047.0 / 2.0) / 6063.0).abs() < 1e-12 && (c[1] - (8.0 + 4023.0 / 2.0) / 4039.0).abs() < 1e-12);
        }
    }

    #[test]
    fn the_optical_centre_follows_the_sensor_shift() {
        let raw = raw_with(&[(DISTORTION_INFO, distortion_block(3, 21.6, 4, TELE.0, TELE.1)), (CENTRE_OFFSET, offset_block(-90.1, 18.59))]);
        let (_, c) = warp("NIKON Z 6_2", &raw).unwrap();
        assert!((c[0] * 6063.0 - (8.0 + 6047.0 / 2.0 - 90.1)).abs() < 0.01, "{c:?}");
        assert!((c[1] * 4039.0 - (8.0 + 4023.0 / 2.0 + 18.59)).abs() < 0.01, "{c:?}");
    }

    #[test]
    fn refuses_unvalidated_bodies_and_unknown_layouts() {
        let good = distortion_block(3, 21.6, 4, WIDE.0, WIDE.1);
        assert!(warp("NIKON Z 6_2", &raw_with(&[(DISTORTION_INFO, good.clone())])).is_some());
        assert!(warp("NIKON Z f", &raw_with(&[(DISTORTION_INFO, good.clone())])).is_some());
        assert!(warp("NIKON Z 8", &raw_with(&[(DISTORTION_INFO, good.clone())])).is_some());
        for model in ["", "NIKON Z 6", "NIKON Z 6_3", "NIKON Z 9", "NIKON D850", "NIKON Z 6_2 x"] {
            assert!(warp(model, &raw_with(&[(DISTORTION_INFO, good.clone())])).is_none(), "{model}");
        }
        // correction off, a DX radius, another term count, a wild coefficient or zoom, no NEFInfo at all
        for bad in [
            distortion_block(0, 21.6, 4, WIDE.0, WIDE.1),
            distortion_block(3, 14.2, 4, WIDE.0, WIDE.1),
            distortion_block(3, 21.6, 3, WIDE.0, WIDE.1),
            distortion_block(3, 21.6, 4, [0.0, 0.0, 1.5], 0.0),
            distortion_block(3, 21.6, 4, WIDE.0, 0.9),
            b"0200".iter().copied().chain([0u8; 80]).collect(),
            vec![0u8; 12],
        ] {
            assert!(warp("NIKON Z 6_2", &raw_with(&[(DISTORTION_INFO, bad)])).is_none());
        }
        assert!(warp("NIKON Z 6_2", &Ifd::default()).is_none());
        // a map that folds (the source radius turns back) is refused
        assert!(warp("NIKON Z 6_2", &raw_with(&[(DISTORTION_INFO, distortion_block(3, 21.6, 4, [-0.9, 0.0, 0.0], 0.0))])).is_none());
    }

    #[test]
    fn only_the_full_3_2_image_area_is_corrected() {
        let raw = raw_with(&[(DISTORTION_INFO, distortion_block(3, 21.6, 4, WIDE.0, WIDE.1))]);
        // a DX or 1:1 crop of the sensor would need another normalisation
        assert!(distortion("NIKON Z 6_2", &raw, AREA, Rect::new(1016, 676, 4032, 2688)).is_none(), "DX");
        assert!(distortion("NIKON Z 6_2", &raw, AREA, Rect::new(1016, 8, 4024, 4024)).is_none());
        assert!(distortion("NIKON Z 6_2", &raw, AREA, Rect::new(0, 0, 1, 1)).is_none());
        // a sensor-shift offset beyond 5 % of the frame is not trusted
        let far = raw_with(&[(DISTORTION_INFO, distortion_block(3, 21.6, 4, WIDE.0, WIDE.1)), (CENTRE_OFFSET, offset_block(900.0, 0.0))]);
        assert!(warp("NIKON Z 6_2", &far).is_none());
    }
}
