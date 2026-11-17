//! AVIF encoding helpers (8/16-bit SDR via `ravif` and 10-bit 4:4:4 BT.2020 HDR via `rav1e` + `avif-serialize`).

use crate::{CodecError, Format, Image};

/// Transfer function for [`encode_avif_hdr`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HdrTransfer {
    Pq,
    Hlg,
}

/// Absolute luminance of SDR white in an HDR file (ITU-R BT.2408 reference white), cd/m^2.
pub const HDR_REFERENCE_WHITE_NITS: f32 = 203.0;

#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
fn pq_encode(nits: f32) -> f32 {
    const M1: f32 = 2610.0 / 16384.0;
    const M2: f32 = 2523.0 / 4096.0 * 128.0;
    const C1: f32 = 3424.0 / 4096.0;
    const C2: f32 = 2413.0 / 4096.0 * 32.0;
    const C3: f32 = 2392.0 / 4096.0 * 32.0;
    let y = (nits / 10_000.0).clamp(0.0, 1.0).powf(M1);
    ((C1 + C2 * y) / (1.0 + C3 * y)).powf(M2)
}

#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
fn hlg_signal(rgb: [f32; 3]) -> [f32; 3] {
    const K: [f32; 3] = [0.2627, 0.6780, 0.0593];
    const GAMMA: f32 = 1.2;
    const A: f32 = 0.17883277;
    const B: f32 = 0.28466892;
    const C: f32 = 0.559_910_7;
    let display = rgb.map(|v| (if v.is_finite() { v.max(0.0) } else { 0.0 }) * (HDR_REFERENCE_WHITE_NITS / crate::cicp::HLG_PEAK_NITS));
    let y = K[0] * display[0] + K[1] * display[1] + K[2] * display[2];
    let inverse_ootf = if y > 0.0 { y.powf((1.0 - GAMMA) / GAMMA) } else { 0.0 };
    display.map(|v| {
        let scene = (v * inverse_ootf).clamp(0.0, 1.0);
        if scene <= 1.0 / 12.0 { (3.0 * scene).sqrt() } else { A * (12.0 * scene - B).ln() + C }
    })
}

/// Encode an 8-bit or 16-bit SDR `Image` as AVIF (`4:4:4`, 10-bit AV1) with `quality` (`1..=100`)
/// and `speed` (`1..=10`).
pub fn encode_avif_with_speed(img: &Image, quality: u8, speed: u8) -> Result<Vec<u8>, CodecError> {
    #[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
    {
        let mut enc = ravif::Encoder::new()
            .with_quality(f32::from(quality.clamp(1, 100)))
            .with_alpha_quality(f32::from(quality.clamp(1, 100)))
            .with_speed(speed.clamp(1, 10))
            .with_bit_depth(ravif::BitDepth::Ten);
        if let Some(exif) = img.meta.exif.as_deref() {
            enc = enc.with_exif(exif);
        }
        let alpha_mode = if img.layout().has_alpha() { ravif::AlphaColorMode::UnassociatedClean } else { ravif::AlphaColorMode::UnassociatedDirty };
        enc = enc.with_alpha_color_mode(alpha_mode);
        let w = img.width() as usize;
        let h = img.height() as usize;
        if let Some(s16) = img.to_u16_samples() {
            let ch = img.layout().channels();
            let mut planes = Vec::with_capacity(w * h);
            let mut a_plane = if img.layout().has_alpha() { Some(Vec::with_capacity(w * h)) } else { None };
            for px in s16.chunks_exact(ch) {
                let (r, g, b, a) = match ch {
                    1 => (px[0], px[0], px[0], u16::MAX),
                    2 => (px[0], px[0], px[0], px[1]),
                    3 => (px[0], px[1], px[2], u16::MAX),
                    _ => (px[0], px[1], px[2], px[3]),
                };
                let (rf, gf, bf) = (f32::from(r) / 65535.0, f32::from(g) / 65535.0, f32::from(b) / 65535.0);
                let y = 0.299 * rf + 0.587 * gf + 0.114 * bf;
                let cb = (bf - y) * (0.5 / (1.0 - 0.114)) + 0.5;
                let cr = (rf - y) * (0.5 / (1.0 - 0.299)) + 0.5;
                let q10 = |v: f32| (v * 1023.0).round().clamp(0.0, 1023.0) as u16;
                planes.push([q10(y), q10(cb), q10(cr)]);
                if let Some(ap) = &mut a_plane {
                    ap.push(q10(f32::from(a) / 65535.0));
                }
            }
            let alpha = a_plane.map(Vec::into_iter);
            let res = enc
                .encode_raw_planes_10_bit(w, h, planes, alpha, ravif::PixelRange::Full, ravif::MatrixCoefficients::BT601)
                .map_err(|e| CodecError::encode(Format::Avif, e))?;
            return Ok(res.avif_file);
        }
        let rgba = img.to_rgba8();
        let px: Vec<ravif::RGBA8> = rgba.as_chunks::<4>().0.iter().map(|c| ravif::RGBA8::new(c[0], c[1], c[2], c[3])).collect();
        let res = enc.encode_rgba(ravif::Img::new(&px[..], w, h)).map_err(|e| CodecError::encode(Format::Avif, e))?;
        Ok(res.avif_file)
    }
    #[cfg(not(all(feature = "avif", not(target_arch = "wasm32"))))]
    {
        let _ = (img, quality, speed);
        Err(CodecError::unsupported(Format::Avif, "AVIF encoding is not available in this build"))
    }
}

/// Encode linear BT.2020 HDR `rgb` (`width * height * 3` floats, `1.0` = `203` cd/m^2) as
/// 10-bit 4:4:4 BT.2020 PQ or HLG AVIF.
pub fn encode_avif_hdr(
    width: u32,
    height: u32,
    rgb: &[f32],
    quality: u8,
    speed: u8,
    exif: Option<&[u8]>,
    transfer: HdrTransfer,
) -> Result<Vec<u8>, CodecError> {
    let (w, h) = (width as usize, height as usize);
    let n = w.checked_mul(h).ok_or_else(|| CodecError::encode(Format::Avif, "image too large"))?;
    if n == 0 || n.checked_mul(3).is_none_or(|need| rgb.len() < need) {
        return Err(CodecError::encode(Format::Avif, "HDR AVIF: sample buffer too short"));
    }
    #[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
    {
        use rav1e::prelude::*;
        const K: [f32; 3] = [0.2627, 0.6780, 0.0593];
        let mut max_nits = 0f32;
        let mut sum_nits = 0f64;
        let planes: Vec<[u16; 3]> = rgb
            .as_chunks::<3>()
            .0
            .iter()
            .take(n)
            .map(|c| {
                let nits = [c[0], c[1], c[2]].map(|v| (if v.is_finite() { v.max(0.0) } else { 0.0 }) * HDR_REFERENCE_WHITE_NITS);
                let y_nits = K[0] * nits[0] + K[1] * nits[1] + K[2] * nits[2];
                max_nits = max_nits.max(nits[0].max(nits[1]).max(nits[2]));
                sum_nits += f64::from(y_nits);
                let [r, g, b] = match transfer {
                    HdrTransfer::Pq => nits.map(pq_encode),
                    HdrTransfer::Hlg => hlg_signal([c[0], c[1], c[2]]),
                };
                let y = K[0] * r + K[1] * g + K[2] * b;
                let cb = (b - y) / (2.0 * (1.0 - K[2])) + 0.5;
                let cr = (r - y) / (2.0 * (1.0 - K[0])) + 0.5;
                [y, cb, cr].map(|v| (v * 1023.0).round().clamp(0.0, 1023.0) as u16)
            })
            .collect();
        let q = f32::from(quality.clamp(1, 100)) / 100.0;
        let x = if q >= 0.82 {
            (1.0 - q) * 2.6
        } else if q > 0.25 {
            1.0 - 0.125 - q * 0.5
        } else {
            1.0 - q
        };
        let quantizer = (x * 255.0).round() as usize;
        let mut ec = EncoderConfig::with_speed_preset(speed.clamp(1, 10));
        ec.width = w;
        ec.height = h;
        ec.time_base = Rational::new(1, 1);
        ec.bit_depth = 10;
        ec.chroma_sampling = ChromaSampling::Cs444;
        ec.pixel_range = PixelRange::Full;
        ec.color_description = Some(ColorDescription {
            color_primaries: ColorPrimaries::BT2020,
            transfer_characteristics: match transfer {
                HdrTransfer::Pq => TransferCharacteristics::SMPTE2084,
                HdrTransfer::Hlg => TransferCharacteristics::HLG,
            },
            matrix_coefficients: MatrixCoefficients::BT2020NCL,
        });
        let cll = (max_nits.ceil().clamp(1.0, 10_000.0) as u16, (sum_nits / n as f64).ceil().clamp(1.0, 10_000.0) as u16);
        ec.content_light =
            (transfer == HdrTransfer::Pq).then_some(ContentLight { max_content_light_level: cll.0, max_frame_average_light_level: cll.1 });
        ec.still_picture = true;
        ec.quantizer = quantizer;
        ec.min_quantizer = quantizer.min(255) as u8;
        ec.speed_settings.multiref = false;
        ec.speed_settings.rdo_lookahead_frames = 1;
        ec.speed_settings.scene_detection_mode = SceneDetectionSpeed::None;
        let mut ctx: Context<u16> = Config::new().with_encoder_config(ec).new_context().map_err(|e| CodecError::encode(Format::Avif, e))?;
        let mut frame = ctx.new_frame();
        {
            let mut it = planes.iter();
            let mut fp = frame.planes.iter_mut();
            let (Some(py), Some(pu), Some(pv)) = (fp.next(), fp.next(), fp.next()) else {
                return Err(CodecError::encode(Format::Avif, "AV1: frame has no planes"));
            };
            let (mut sy, mut su, mut sv) = (py.mut_slice(Default::default()), pu.mut_slice(Default::default()), pv.mut_slice(Default::default()));
            for ((ry, ru), rv) in sy.rows_iter_mut().zip(su.rows_iter_mut()).zip(sv.rows_iter_mut()).take(h) {
                for ((y, u), v) in ry.iter_mut().zip(ru.iter_mut()).zip(rv.iter_mut()).take(w) {
                    let Some(p) = it.next() else { break };
                    (*y, *u, *v) = (p[0], p[1], p[2]);
                }
            }
        }
        ctx.send_frame(frame).map_err(|e| CodecError::encode(Format::Avif, e))?;
        ctx.flush();
        let mut av1 = Vec::new();
        loop {
            match ctx.receive_packet() {
                Ok(mut p) if p.frame_type == FrameType::KEY => av1.append(&mut p.data),
                Ok(_) => {}
                Err(EncoderStatus::Encoded | EncoderStatus::LimitReached) => break,
                Err(e) => return Err(CodecError::encode(Format::Avif, e)),
            }
        }
        use avif_serialize::constants as c;
        let mut a = avif_serialize::Aviffy::new();
        a.set_matrix_coefficients(c::MatrixCoefficients::Bt2020Ncl)
            .set_transfer_characteristics(match transfer {
                HdrTransfer::Pq => c::TransferCharacteristics::Smpte2084,
                HdrTransfer::Hlg => c::TransferCharacteristics::Hlg,
            })
            .set_color_primaries(c::ColorPrimaries::Bt2020)
            .set_full_color_range(true);
        if transfer == HdrTransfer::Pq {
            a.set_content_light_level(cll.0, cll.1);
        }
        if let Some(exif) = exif {
            a.set_exif(exif.to_vec());
        }
        Ok(a.to_vec(&av1, None, width, height, 10))
    }
    #[cfg(not(all(feature = "avif", not(target_arch = "wasm32"))))]
    {
        let _ = (width, height, rgb, quality, speed, exif, transfer);
        Err(CodecError::unsupported(Format::Avif, "AVIF encoding is not available in this build"))
    }
}

#[cfg(test)]
mod tests {
    #[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
    #[test]
    fn sdr_avif_uses_requested_alpha_quality() {
        use crate::{ChannelLayout, Image};

        let pixels: Vec<ravif::RGBA8> = (0..1024).map(|i| ravif::RGBA8::new(100, 120, 140, ((i * 37) % 256) as u8)).collect();
        let samples: Vec<u8> = pixels.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect();
        let image = Image::from_u8(32, 32, ChannelLayout::Rgba, samples).unwrap();
        let actual = super::encode_avif_with_speed(&image, 20, 10).unwrap();
        let expected = ravif::Encoder::new()
            .with_quality(20.0)
            .with_alpha_quality(20.0)
            .with_speed(10)
            .with_bit_depth(ravif::BitDepth::Ten)
            .with_alpha_color_mode(ravif::AlphaColorMode::UnassociatedClean)
            .encode_rgba(ravif::Img::new(&pixels, 32, 32))
            .unwrap()
            .avif_file;
        assert!(actual == expected, "encoded alpha must use the requested quality");
    }

    #[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
    #[test]
    fn sdr_avif_keeps_exif_metadata() {
        use crate::{ChannelLayout, Image};

        let exif = b"II*\0shared-avif-exif".to_vec();
        let mut image = Image::from_u8(2, 1, ChannelLayout::Rgb, vec![20, 40, 60, 80, 100, 120]).unwrap();
        image.meta.exif = Some(exif.clone());

        let avif = super::encode_avif_with_speed(&image, 80, 8).unwrap();
        assert!(avif.windows(exif.len()).any(|window| window == exif.as_slice()));
    }

    #[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
    #[test]
    fn hlg_mapping_matches_reference_white_and_peak() {
        let white = super::hlg_signal([1.0; 3]);
        assert!(white.iter().all(|&value| (value - 0.75).abs() < 0.005), "{white:?}");

        let peak = super::hlg_signal([crate::cicp::HLG_PEAK_NITS / super::HDR_REFERENCE_WHITE_NITS; 3]);
        assert!(peak.iter().all(|&value| (value - 1.0).abs() < 0.005), "{peak:?}");

        let invalid = super::hlg_signal([f32::NAN, -1.0, f32::INFINITY]);
        assert_eq!(invalid, [0.0; 3]);
    }

    #[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
    #[test]
    fn hlg_encoded_output_signals_bt2020_hlg() {
        let avif = super::encode_avif_hdr(1, 1, &[1.0; 3], 100, 10, None, super::HdrTransfer::Hlg).unwrap();
        let nclx = avif.windows(4).position(|window| window == b"nclx").expect("the AVIF has nclx color metadata");
        let color = avif.get(nclx + 4..nclx + 11).expect("complete nclx color metadata");
        assert_eq!(&color[..2], &[0, 9], "BT.2020 primaries");
        assert_eq!(&color[2..4], &[0, 18], "BT.2100 HLG transfer");
        assert_eq!(&color[4..6], &[0, 9], "BT.2020 non-constant luminance matrix");
        assert!(!avif.windows(4).any(|window| window == b"clli"), "HLG does not signal PQ content-light metadata");
    }

    #[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
    #[test]
    fn hlg_encoded_pixels_preserve_reference_white_and_peak() {
        let mut rgb = Vec::new();
        for _ in 0..32 {
            for x in 0..64 {
                let level = if x < 32 { 1.0 } else { crate::cicp::HLG_PEAK_NITS / super::HDR_REFERENCE_WHITE_NITS };
                rgb.extend_from_slice(&[level; 3]);
            }
        }
        let avif = super::encode_avif_hdr(64, 32, &rgb, 100, 10, None, super::HdrTransfer::Hlg).unwrap();
        let mut offset = 0;
        let payload = loop {
            let header = avif.get(offset..offset + 8).expect("complete AVIF box header");
            let size = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
            assert!(size >= 8 && offset + size <= avif.len(), "valid AVIF box size");
            if &header[4..] == b"mdat" {
                break &avif[offset + 8..offset + size];
            }
            offset += size;
        };
        let mut decoder = craft_video::av1::decode::Decoder::new();
        let mut frames = decoder.decode(payload).unwrap();
        frames.extend(decoder.flush_result().unwrap());
        let luma = &frames.first().expect("encoded AV1 frame").planes[0];
        assert!((i32::from(luma[16 * 64 + 16]) - 767).abs() <= 5, "diffuse white");
        assert!((i32::from(luma[16 * 64 + 48]) - 1023).abs() <= 5, "peak white");
    }
}
