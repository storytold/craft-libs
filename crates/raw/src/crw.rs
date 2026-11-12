//! Canon CRW (CIFF heap) through the `craft-crw` crate (`crates/crw`): the bodies before the CR2 era
//! (EOS D30/D60/10D/300D, PowerShot G-series).
//!
//! The decoder, its frozen code tables and its clean-room record live there (`crates/crw/clean-room/`). This
//! module maps its output onto [`RawImage`]: CFA phase, black and white levels, active area, orientation,
//! as-shot white balance and the make, model and capture-time records (a CRW carries no Exif block, so this
//! is where a caller learns the camera). There is one CRW decoder; this is not a second one.
//!
//! Known gaps (documented in the freeze): the PowerShot G5's entropy coding diverges from the model and is
//! refused; PowerShot bodies without CC0 samples (G2/G3/Pro1/S-series) decode through the same model
//! unverified. The file carries no colour matrices.

use crate::{BlackLevel, Camera, Cfa, ColorData, Limits, Mode, OpcodeLists, Orientation, RawData, RawError, RawFormat, RawImage, Rect, Result};

fn map_err(e: craft_crw::Error) -> RawError {
    match e {
        craft_crw::Error::NotCrw => RawError::NotRaw,
        craft_crw::Error::Corrupt(why) => RawError::Corrupt(format!("CRW: {why}")),
        craft_crw::Error::Unsupported(why) => RawError::Unsupported(format!("CRW: {why}")),
        craft_crw::Error::LimitExceeded(what) => RawError::Limit(what),
    }
}

pub(crate) fn decode(bytes: &[u8], mode: Mode, caller: &Limits) -> Result<RawImage> {
    // craft-crw's own cap (2²³ samples; the largest CRW frame is 7.4 million), or the caller's when that is
    // lower: its decode holds about three `u16` buffers of the declared size, so this crate's general default
    // (2³⁰) would let a malformed file ask for gigabytes. No sample may be filled after the
    // entropy stream ends (the crate allows one 64-sample block by default): every corpus file
    // ends exactly on its frame, so a shorter stream is a truncated file and is `Corrupt`.
    let mut limits = craft_crw::Limits::default();
    limits.max_stream_end_fill = 0;
    limits.max_samples = limits.max_samples.min(caller.samples());
    let full = mode == Mode::Full;
    let (h, samples, black, white) = if full {
        let img = craft_crw::decode(bytes, &limits).map_err(map_err)?;
        let black = BlackLevel { repeat_rows: 2, repeat_cols: 2, values: img.black.values.to_vec(), delta_h: vec![], delta_v: vec![] };
        (img.header, img.samples, black, img.white)
    } else {
        let h = craft_crw::header(bytes, &limits).map_err(map_err)?;
        let white = ((1u32 << h.bits.clamp(1, 16)) - 1) as f32;
        (h, Vec::new(), BlackLevel::uniform(0.0), white)
    };
    let active = Rect::new(h.active_area.x, h.active_area.y, h.active_area.width, h.active_area.height);
    let orientation = Orientation::from_exif(h.exif_orientation as _);
    let camera = Camera { make: h.make, model: h.model, unique_model: None, capture_unix: h.capture_unix.filter(|&t| plausible_capture_time(t)) };
    let img = RawImage {
        format: RawFormat::Crw,
        width: h.width,
        height: h.height,
        cpp: 1,
        data: RawData::U16(samples),
        cfa: Some(Cfa::bayer_static(h.cfa.name())),
        bits: h.bits,
        black,
        white: vec![white],
        active_area: active,
        crop: Rect::new(0, 0, active.width, active.height),
        orientation,
        color: ColorData::default(),
        wb_multipliers: h.wb_multipliers,
        linearized: false,
        pixel_aspect: 1.0,
        opcodes: OpcodeLists::default(),
        camera,
        cinema: Default::default(),
        lens_metadata: Default::default(),
        level_sources: Default::default(),
    };
    img.validate_for(mode)?;
    Ok(img)
}

/// Whether a TimeStamp record is a capture time: before 1970 or after 2200 is a garbage clock.
fn plausible_capture_time(t: i64) -> bool {
    chrono::DateTime::from_timestamp(t, 0).is_some_and(|c| (1970..=2200).contains(&chrono::Datelike::year(&c)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 48;
    const H: usize = 20;

    /// A CIFF file with the records the decoder reads and a pseudo-random payload: enough for the
    /// header path; the entropy data decodes or is refused, never panics.
    fn synthetic_crw(model: &str) -> Vec<u8> {
        synthetic_crw_sized(model, W, H)
    }

    fn synthetic_crw_sized(model: &str, w: usize, h: usize) -> Vec<u8> {
        let payload: Vec<u8> = (0..514 + 256).map(|i| (i * 41 % 253) as u8).collect();
        synthetic_crw_with_payload(model, w, h, payload)
    }

    fn synthetic_crw_with_payload(model: &str, w: usize, h: usize, payload: Vec<u8>) -> Vec<u8> {
        let le16 = |v: u16| v.to_le_bytes();
        let le32 = |v: u32| v.to_le_bytes();
        let heap = |records: &[(u16, Vec<u8>)]| -> Vec<u8> {
            let mut b = Vec::new();
            let mut entries = Vec::new();
            for (tc, v) in records {
                entries.push((*tc, v.len() as u32, b.len() as u32));
                b.extend_from_slice(v);
                if b.len() % 2 == 1 {
                    b.push(0);
                }
            }
            let tbl = b.len() as u32;
            b.extend_from_slice(&le16(entries.len() as u16));
            for (tc, l, off) in entries {
                b.extend_from_slice(&le16(tc));
                b.extend_from_slice(&le32(l));
                b.extend_from_slice(&le32(off));
            }
            b.extend_from_slice(&le32(tbl));
            b
        };
        let sensor: Vec<u8> = [34u16, w as u16, h as u16, 0, 0, 2, 2, w as u16 - 3, h as u16 - 3].iter().flat_map(|&v| le16(v)).collect();
        let dtable: Vec<u8> = [0u32, 0, 514, 0].iter().flat_map(|&v| le32(v)).collect();
        let image_info: Vec<u8> = [w as u32, h as u32, 0, 270, 12, 12, 0].iter().flat_map(|&v| le32(v)).collect();
        let mut color_balance: Vec<u16> = vec![66];
        color_balance.extend([2000u16, 800, 800, 960]);
        color_balance.extend((0..7).flat_map(|_| [1500u16, 900, 900, 1500]));
        let color_balance: Vec<u8> = color_balance.iter().flat_map(|&v| le16(v)).collect();
        let mut make_model = b"Canon\0".to_vec();
        make_model.extend_from_slice(model.as_bytes());
        make_model.push(0);
        let props = heap(&[
            (0x1031, sensor),
            (0x1835, dtable),
            (0x080a, make_model),
            (0x1810, image_info),
            (0x180e, [1083410638u32, 0, 0].iter().flat_map(|&v| le32(v)).collect()),
            (0x10a9, color_balance),
        ]);
        let root = heap(&[(0x2005, payload), (0x300a, props)]);
        let mut f = b"II".to_vec();
        f.extend_from_slice(&le32(26));
        f.extend_from_slice(b"HEAPCCDR");
        f.extend_from_slice(&le32(0x0001_0002));
        f.extend_from_slice(&[0; 8]);
        f.extend_from_slice(&root);
        f
    }

    #[test]
    fn header_mode_maps_the_records_onto_raw_image() {
        let f = synthetic_crw("Canon EOS D60");
        let img = decode(&f, Mode::Header, &Limits::default()).expect("header decodes");
        assert_eq!((img.format, img.width, img.height, img.bits, img.cpp), (RawFormat::Crw, W, H, 10, 1));
        assert_eq!(img.data.len(), 0);
        assert_eq!(img.active_area, Rect::new(2, 2, W - 4, H - 4));
        assert_eq!(img.crop, Rect::new(0, 0, W - 4, H - 4));
        assert_eq!(img.cfa.as_ref().map(|c| c.name()).as_deref(), Some("RGGB"));
        assert_eq!(img.orientation, Orientation::from_exif(8));
        assert_eq!(img.camera.make.as_deref(), Some("Canon"));
        assert_eq!(img.camera.model.as_deref(), Some("Canon EOS D60"));
        assert_eq!(img.camera.capture_unix, Some(1083410638));
        let wb = img.wb_multipliers.expect("ColorBalance slot 0");
        assert!((wb[0] - 2.5).abs() < 1e-3 && wb[1] == 1.0 && (wb[2] - 1.2).abs() < 1e-3, "{wb:?}");
        assert_eq!(img.white, vec![1023.0]);
        assert_eq!(img.black, BlackLevel::uniform(0.0));
        // the entropy data is pseudo-random: decodes or errors, never panics
        let _ = decode(&f, Mode::Full, &Limits::default());
        let g1 = decode(&synthetic_crw("Canon PowerShot G1"), Mode::Header, &Limits::default()).expect("header decodes");
        assert_eq!(g1.cfa.as_ref().map(|c| c.name()).as_deref(), Some("GBRG"));
    }

    #[test]
    fn errors_map_onto_raw_error() {
        assert!(matches!(decode(b"II*\0\x08\0\0\0", Mode::Full, &Limits::default()), Err(RawError::NotRaw)));
        assert!(matches!(decode(b"II    HEAPCCDR  ", Mode::Full, &Limits::default()), Err(RawError::Corrupt(w)) if w.starts_with("CRW: ")));
        let Err(RawError::Unsupported(why)) = decode(&synthetic_crw("Canon PowerShot G5"), Mode::Header, &Limits::default()) else {
            panic!("expected Unsupported")
        };
        assert!(why.contains("PowerShot G5"), "{why}");
        assert_eq!(map_err(craft_crw::Error::LimitExceeded("image too large")), RawError::Limit("image too large"));
    }

    /// A file declaring more samples than any CRW camera has is refused before anything is
    /// allocated: 4096 × 4096 (16.8 million) passed when the adapter raised the crate's cap of 2²³
    /// to the raw crate's 2³⁰.
    #[test]
    fn an_oversized_crw_is_refused_by_the_crates_own_limit() {
        let f = synthetic_crw_sized("Canon EOS D60", 4096, 4096);
        for mode in [Mode::Header, Mode::Full] {
            assert_eq!(decode(&f, mode, &Limits::default()).err(), Some(RawError::Limit("image too large")), "{mode:?}");
        }
        // the largest real frame (the 10D's 3152 × 2068) is within it
        assert!(decode(&synthetic_crw_sized("Canon EOS D60", 3152, 2068), Mode::Header, &Limits::default()).is_ok());
    }

    /// A stream cut short is `Corrupt`, not a smeared image: the crate once filled every sample
    /// after the entropy stream ended with its predictor value and returned success, so a CRW
    /// truncated just past its header rendered and exported as a flat frame.
    #[test]
    fn a_truncated_entropy_stream_is_corrupt() {
        // the 514-byte header and two bytes of entropy: almost all of the 960-sample frame unread
        let f = synthetic_crw_with_payload("Canon EOS D60", W, H, vec![0; 516]);
        let Err(RawError::Corrupt(why)) = decode(&f, Mode::Full, &Limits::default()) else { panic!("a truncated stream must be refused") };
        assert!(why.starts_with("CRW: ") && why.contains("before the frame does"), "{why}");
        // the header alone still reads, so the library can list the file and show its preview
        assert!(decode(&f, Mode::Header, &Limits::default()).is_ok());
    }

    /// A stream that ends a few samples short is `Corrupt` too: the crate's default limit fills up
    /// to one 64-sample block silently, and no real CRW fills any.
    #[test]
    fn a_stream_a_few_samples_short_is_corrupt() {
        // an all-zero entropy stream of 842 bytes decodes the 960-sample frame exactly
        let payload = |len: usize| vec![0u8; 514 + len];
        let whole = synthetic_crw_with_payload("Canon EOS D60", W, H, payload(842));
        assert!(decode(&whole, Mode::Full, &Limits::default()).is_ok());
        // six bytes fewer leave 10 samples unread, within the crate's default of 64
        let short = synthetic_crw_with_payload("Canon EOS D60", W, H, payload(836));
        let filled = craft_crw::decode(&short, &craft_crw::Limits::default()).map(|i| i.stream_end_fill);
        assert_eq!(filled, Ok(10));
        let Err(RawError::Corrupt(why)) = decode(&short, Mode::Full, &Limits::default()) else { panic!("a stream 10 samples short must be refused") };
        assert!(why.contains("10 samples before the frame does"), "{why}");
    }

    #[test]
    fn a_garbage_clock_is_not_a_capture_time() {
        for t in [0i64, 951867295, 1083410638, 4102444799] {
            assert!(plausible_capture_time(t), "{t}");
        }
        assert!(!plausible_capture_time(-1));
        assert!(!plausible_capture_time(i64::MAX));
    }

    /// The caller's sample limit applies when it is lower than craft-crw's own.
    #[test]
    fn the_callers_sample_limit_applies() {
        let f = synthetic_crw("Canon EOS D60");
        let mut small = Limits { max_samples: W * H - 1, ..Limits::default() };
        for mode in [Mode::Header, Mode::Full] {
            assert_eq!(decode(&f, mode, &small).err(), Some(RawError::Limit("image too large")), "{mode:?}");
        }
        small.max_samples = W * H;
        assert!(decode(&f, Mode::Header, &small).is_ok());
    }
}
