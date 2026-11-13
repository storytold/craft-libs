//! Encoder tests against this crate's decoder: the decoded pictures must equal the encoder's
//! reconstruction exactly (any difference would drift along a GOP of P pictures). The ffmpeg
//! comparison is in `tests/mpeg2/encode.rs`.

use super::*;
use crate::mpeg2::{Decoder, probe};

/// A moving, textured test sequence: a gradient with a sharp pattern drifting by a fraction of a
/// sample per picture, and a block moving across it.
pub(crate) fn scene(w: usize, h: usize, cw: usize, ch: usize, t: usize) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let tf = t as f32;
    let mut y = vec![0u8; w * h];
    for r in 0..h {
        for c in 0..w {
            let (x, yy) = (c as f32 + tf * 1.5, r as f32 + tf * 0.75);
            let v = 60.0
                + 0.3 * x
                + 0.2 * yy
                + 40.0 * ((x * 0.21).sin() * (yy * 0.17).cos())
                + if ((x as i32 / 8) + (yy as i32 / 8)) % 2 == 0 { 25.0 } else { 0.0 };
            y[r * w + c] = v.clamp(0.0, 255.0) as u8;
        }
    }
    let (bx, by) = ((t * 3) % w.max(1), (t * 2) % h.max(1));
    for r in by..(by + 12).min(h) {
        for c in bx..(bx + 12).min(w) {
            y[r * w + c] = 230;
        }
    }
    let mut cb = vec![0u8; cw * ch];
    let mut cr = vec![0u8; cw * ch];
    for r in 0..ch {
        for c in 0..cw {
            cb[r * cw + c] = (128.0 + 50.0 * ((c as f32 + tf) * 0.13).sin()) as u8;
            cr[r * cw + c] = (128.0 + 40.0 * ((r as f32 - tf) * 0.11).cos()) as u8;
        }
    }
    (y, cb, cr)
}

/// Encode `frames` pictures of the scene; returns the stream and the reconstruction.
fn encode(cfg: EncoderConfig, frames: usize) -> (Vec<u8>, Vec<ReconFrame>, Vec<Packet>) {
    let (w, h) = (cfg.width as usize, cfg.height as usize);
    let (cw, ch) = (w / 2, if cfg.chroma == ChromaFormat::Yuv422 { h } else { h / 2 });
    let mut enc = Encoder::new(cfg).unwrap();
    enc.set_recon_capture(true);
    let mut es = Vec::new();
    let mut packets = Vec::new();
    for t in 0..frames {
        let (y, cb, cr) = scene(w, h, cw, ch, t);
        let f = YuvFrame { y: &y, cb: &cb, cr: &cr, y_stride: w, c_stride: cw };
        for p in enc.encode(&f, t as i64).unwrap() {
            es.extend_from_slice(&p.data);
            packets.push(p);
        }
    }
    for p in enc.flush() {
        es.extend_from_slice(&p.data);
        packets.push(p);
    }
    es.extend_from_slice(enc.end_of_sequence());
    (es, enc.take_recon(), packets)
}

/// Decode packets (each tagged with its presentation time); returns the pictures in display
/// order.
fn decode(packets: &[Packet]) -> Vec<crate::mpeg2::Picture> {
    let mut d = Decoder::new();
    let mut out = Vec::new();
    for p in packets {
        out.extend(d.decode(&p.data, p.pts).unwrap());
    }
    out.extend(d.flush());
    assert_eq!(d.errors(), 0, "decoder errors: {:?}", d.last_error());
    out
}

fn assert_decodes_to_recon(name: &str, cfg: EncoderConfig, frames: usize) -> Vec<Packet> {
    let (_, recon, packets) = encode(cfg, frames);
    assert!(packets.windows(2).all(|w| w[0].dts < w[1].dts) && packets.iter().all(|p| p.dts <= p.pts), "{name}: decoding times");
    let pics = decode(&packets);
    assert_eq!(pics.len(), frames, "{name}: picture count");
    for (i, (p, r)) in pics.iter().zip(&recon).enumerate() {
        assert_eq!(p.pts, i as i64, "{name}: display order");
        for (plane, (a, b)) in [("Y", (&p.y, &r.y)), ("Cb", (&p.cb, &r.cb)), ("Cr", (&p.cr, &r.cr))] {
            let first = a.iter().zip(b.iter()).position(|(x, y)| x != y);
            assert!(first.is_none() && a.len() == b.len(), "{name}: picture {i} ({:?}) {plane} differs at {first:?}", p.picture_type);
        }
    }
    packets
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let sse: f64 = a.iter().zip(b).map(|(x, y)| (f64::from(*x) - f64::from(*y)).powi(2)).sum();
    10.0 * (255.0f64.powi(2) * a.len() as f64 / sse.max(1e-9)).log10()
}

#[test]
fn intra_only_decodes_to_the_reconstruction() {
    let mut cfg = EncoderConfig::new(64, 48, 25, 1);
    cfg.gop_size = 1;
    let packets = assert_decodes_to_recon("intra", cfg, 3);
    assert!(packets.iter().all(|p| p.keyframe && p.picture_type == PictureType::I));
}

#[test]
fn long_p_gop_does_not_drift() {
    let mut cfg = EncoderConfig::new(96, 64, 25, 1);
    cfg.gop_size = 30;
    cfg.b_pictures = 0;
    let packets = assert_decodes_to_recon("long gop", cfg, 30);
    assert_eq!(packets.iter().filter(|p| p.keyframe).count(), 1);
    assert!(packets[1..].iter().all(|p| p.picture_type == PictureType::P));
}

#[test]
fn every_coding_option_decodes_to_the_reconstruction() {
    let base = EncoderConfig { gop_size: 6, ..EncoderConfig::new(80, 48, 30000, 1001) };
    let mut matrix = [0u8; 64];
    for (i, m) in matrix.iter_mut().enumerate() {
        *m = (8 + i * 2) as u8;
    }
    let cases = [
        ("422", EncoderConfig { chroma: ChromaFormat::Yuv422, ..base.clone() }),
        ("linear q", EncoderConfig { nonlinear_quant: false, rate: RateControl::Quantiser(3), ..base.clone() }),
        ("coarse", EncoderConfig { rate: RateControl::Quantiser(31), ..base.clone() }),
        ("fine", EncoderConfig { rate: RateControl::Quantiser(1), adaptive_quant: false, ..base.clone() }),
        ("dc 8 and 10", EncoderConfig { intra_dc_bits: 10, ..base.clone() }),
        ("dc 8", EncoderConfig { intra_dc_bits: 8, ..base.clone() }),
        ("matrices", EncoderConfig { intra_matrix: Some(matrix), non_intra_matrix: Some(matrix), ..base.clone() }),
        ("small range", EncoderConfig { search_range: 3, ..base.clone() }),
        ("abr", EncoderConfig { rate: RateControl::Abr { kbps: 300 }, ..base.clone() }),
        ("odd size", EncoderConfig { width: 50, height: 34, ..base.clone() }),
        ("422 odd size", EncoderConfig { width: 70, height: 30, chroma: ChromaFormat::Yuv422, ..base.clone() }),
    ];
    for (name, cfg) in cases {
        assert_decodes_to_recon(name, cfg.clone(), 8);
        assert_decodes_to_recon(name, EncoderConfig { b_pictures: 0, ..cfg }, 8);
    }
}

#[test]
fn b_pictures_decode_to_the_reconstruction() {
    let base = EncoderConfig::new(96, 64, 25, 1);
    for b in 1..=4 {
        for closed in [true, false] {
            for (gop, frames) in [(12, 30), (7, 17), (2, 6), (1, 4), (15, 16)] {
                let name = format!("b {b} closed {closed} gop {gop} frames {frames}");
                let cfg = EncoderConfig { b_pictures: b, closed_gop: closed, gop_size: gop, ..base.clone() };
                let packets = assert_decodes_to_recon(&name, cfg, frames);
                if gop > 1 {
                    assert!(packets.iter().any(|p| p.picture_type == PictureType::B), "{name}: no B pictures");
                }
            }
        }
    }
    let cases = [
        ("422", EncoderConfig { chroma: ChromaFormat::Yuv422, ..base.clone() }),
        ("abr", EncoderConfig { rate: RateControl::Abr { kbps: 400 }, ..base.clone() }),
        ("odd size", EncoderConfig { width: 50, height: 34, ..base.clone() }),
        ("small range", EncoderConfig { search_range: 2, ..base.clone() }),
        ("linear coarse", EncoderConfig { nonlinear_quant: false, rate: RateControl::Quantiser(25), ..base.clone() }),
    ];
    for (name, cfg) in cases {
        assert_decodes_to_recon(name, cfg, 14);
    }
}

#[test]
fn pictures_are_reordered_into_decoding_order() {
    let cfg = EncoderConfig { gop_size: 12, b_pictures: 2, ..EncoderConfig::new(64, 48, 25, 1) };
    let (_, _, packets) = encode(cfg, 15);
    let order: Vec<(PictureType, i64)> = packets.iter().map(|p| (p.picture_type, p.pts)).collect();
    use PictureType::{B, I, P};
    #[rustfmt::skip]
    let want = [(I, 0), (P, 3), (B, 1), (B, 2), (P, 6), (B, 4), (B, 5), (P, 9), (B, 7), (B, 8),
        // B10 and B11 lead the second GOP; at the end the last held picture becomes a P picture
        (I, 12), (B, 10), (B, 11), (P, 14), (B, 13)];
    assert_eq!(order, want);
    // decoding times: the presentation times in order, one interval early
    let dts: Vec<i64> = packets.iter().map(|p| p.dts).collect();
    assert_eq!(dts, (-1..14).collect::<Vec<_>>());
    // temporal_reference counts display order from each GOP's first picture
    let pics = decode(&packets);
    let tr: Vec<u16> = pics.iter().map(|p| p.temporal_reference).collect();
    assert_eq!(tr, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 1, 2, 3, 4]);
}

#[test]
fn a_closed_gop_decodes_on_its_own() {
    for closed in [true, false] {
        let cfg = EncoderConfig { gop_size: 6, b_pictures: 2, closed_gop: closed, ..EncoderConfig::new(64, 48, 25, 1) };
        let (_, recon, packets) = encode(cfg, 12);
        // start decoding at the second GOP's I picture (display 6): B4 and B5 lead it
        let at = packets.iter().position(|p| p.pts == 6).unwrap();
        assert!(packets[at].keyframe);
        let mut d = Decoder::new();
        let mut pics = Vec::new();
        for p in &packets[at..] {
            pics.extend(d.decode(&p.data, p.pts).unwrap());
        }
        pics.extend(d.flush());
        let shown: Vec<i64> = pics.iter().map(|p| p.pts).collect();
        if closed {
            assert_eq!(shown, (4..12).collect::<Vec<_>>());
            for p in &pics {
                assert_eq!(p.y, recon[p.pts as usize].y, "closed GOP: picture {}", p.pts);
            }
        } else {
            // the leading B pictures need the previous GOP's last P picture
            assert_eq!(shown, (6..12).collect::<Vec<_>>());
        }
    }
}

#[test]
fn b_pictures_are_smaller_than_p_pictures() {
    let cfg = EncoderConfig { gop_size: 12, b_pictures: 2, ..EncoderConfig::new(128, 96, 25, 1) };
    let (_, _, packets) = encode(cfg, 12);
    let avg = |t: PictureType| {
        let s: Vec<usize> = packets.iter().filter(|p| p.picture_type == t).map(|p| p.data.len()).collect();
        s.iter().sum::<usize>() as f64 / s.len().max(1) as f64
    };
    let (p, b) = (avg(PictureType::P), avg(PictureType::B));
    assert!(b * 1.5 < p, "P {p:.0} bytes, B {b:.0} bytes");
    // static content: the B pictures skip every macroblock but the first and last of a slice
    let mut enc = Encoder::new(EncoderConfig { gop_size: 12, b_pictures: 2, ..EncoderConfig::new(128, 64, 25, 1) }).unwrap();
    let (y, cb, cr) = scene(128, 64, 64, 32, 0);
    let f = YuvFrame { y: &y, cb: &cb, cr: &cr, y_stride: 128, c_stride: 64 };
    let mut packets = Vec::new();
    for t in 0..7 {
        packets.extend(enc.encode(&f, t).unwrap());
    }
    let i = packets[0].data.len();
    let bs: Vec<&Packet> = packets.iter().filter(|p| p.picture_type == PictureType::B).collect();
    assert_eq!(bs.len(), 4);
    for p in bs {
        // a picture header, four slice headers and eight edge macroblocks
        assert!(p.data.len() < 64, "static B picture {} of {} bytes (I {i})", p.pts, p.data.len());
    }
}

#[test]
fn presentation_times_must_step_evenly_with_b_pictures() {
    let mut enc = Encoder::new(EncoderConfig::new(32, 32, 25, 1)).unwrap();
    let (y, c) = (vec![0u8; 32 * 32], vec![0u8; 16 * 16]);
    let f = YuvFrame { y: &y, cb: &c, cr: &c, y_stride: 32, c_stride: 16 };
    // the I picture waits for the second time, which sets the step
    assert!(enc.encode(&f, 10).unwrap().is_empty());
    let mut packets = enc.encode(&f, 12).unwrap();
    for bad in [12, 11, 15] {
        assert!(matches!(enc.encode(&f, bad), Err(EncodeError::BadTimestamp(_))), "{bad}");
    }
    packets.extend(enc.encode(&f, 14).unwrap());
    packets.extend(enc.flush());
    assert_eq!(packets.iter().map(|p| (p.pts, p.dts)).collect::<Vec<_>>(), [(10, 8), (14, 10), (12, 12)]);
    // without B pictures any increasing (or not) times pass through
    let mut enc = Encoder::new(EncoderConfig { b_pictures: 0, ..EncoderConfig::new(32, 32, 25, 1) }).unwrap();
    for pts in [5, 3, 9] {
        let p = enc.encode(&f, pts).unwrap().remove(0);
        assert_eq!((p.pts, p.dts), (pts, pts));
    }
}

#[cfg(feature = "threads")]
#[test]
fn threaded_slices_give_the_same_stream() {
    let cfg = EncoderConfig { gop_size: 5, threads: false, ..EncoderConfig::new(96, 80, 25, 1) };
    let (a, _, _) = encode(cfg.clone(), 6);
    let (b, _, _) = encode(EncoderConfig { threads: true, ..cfg }, 6);
    assert_eq!(a, b);
}

#[test]
fn quality_follows_the_quantiser() {
    let mut last = f64::MAX;
    let mut last_size = usize::MAX;
    for q in [1u8, 4, 12, 31] {
        let cfg = EncoderConfig { rate: RateControl::Quantiser(q), gop_size: 4, ..EncoderConfig::new(96, 64, 25, 1) };
        let (es, recon, _) = encode(cfg, 4);
        let p: f64 = recon.iter().enumerate().map(|(t, r)| psnr(&scene(96, 64, 48, 32, t).0, &r.y)).sum::<f64>() / recon.len() as f64;
        assert!(p < last && es.len() < last_size, "q {q}: {p:.2} dB, {} bytes", es.len());
        if q == 1 {
            assert!(p > 45.0, "q 1: {p:.2} dB");
        }
        if q == 4 {
            assert!(p > 36.0, "q 4: {p:.2} dB");
        }
        (last, last_size) = (p, es.len());
    }
}

#[test]
fn p_pictures_are_smaller_and_static_content_skips() {
    let cfg = EncoderConfig { gop_size: 8, b_pictures: 0, ..EncoderConfig::new(128, 64, 25, 1) };
    let mut enc = Encoder::new(cfg).unwrap();
    let (y, cb, cr) = scene(128, 64, 64, 32, 0);
    let f = YuvFrame { y: &y, cb: &cb, cr: &cr, y_stride: 128, c_stride: 64 };
    let i = enc.encode(&f, 0).unwrap().remove(0);
    let p1 = enc.encode(&f, 1).unwrap().remove(0);
    let p2 = enc.encode(&f, 2).unwrap().remove(0);
    assert_eq!(p1.picture_type, PictureType::P);
    // the first P picture refines the I picture's reconstruction; the next repeat has almost
    // nothing left to code and skips (every macroblock but the first and last of a slice)
    assert!(p1.data.len() * 5 < i.data.len(), "I {} bytes, P {} bytes", i.data.len(), p1.data.len());
    assert!(p2.data.len() * 20 < i.data.len(), "I {} bytes, second P {} bytes", i.data.len(), p2.data.len());
}

#[test]
fn the_sequence_header_describes_the_stream() {
    let cases = [
        (EncoderConfig::new(352, 288, 25, 1), 0x4A, "Main@Low"),
        (EncoderConfig::new(720, 576, 25, 1), 0x48, "Main@Main"),
        (EncoderConfig::new(1440, 1080, 25, 1), 0x46, "Main@High-1440"),
        (EncoderConfig::new(1920, 1080, 30000, 1001), 0x44, "Main@High"),
        (EncoderConfig { chroma: ChromaFormat::Yuv422, ..EncoderConfig::new(720, 608, 25, 1) }, 0x85, "4:2:2@Main"),
        (EncoderConfig { chroma: ChromaFormat::Yuv422, ..EncoderConfig::new(1920, 1080, 25, 1) }, 0x82, "4:2:2@High"),
        (EncoderConfig { rate: RateControl::Abr { kbps: 20_000 }, ..EncoderConfig::new(720, 576, 25, 1) }, 0x46, "Main@High-1440"),
    ];
    for (cfg, pli, name) in cases {
        let (w, h, aspect) = (cfg.width, cfg.height, cfg.aspect);
        let enc = Encoder::new(cfg).unwrap();
        assert_eq!(enc.profile_and_level(), pli, "{name}");
        let info = probe(enc.sequence_header()).unwrap();
        assert_eq!((info.width, info.height, info.mpeg2, info.progressive_sequence, info.low_delay), (w, h, true, true, false));
        assert_eq!(info.codec_name(), format!("MPEG-2 Video ({name})"));
        assert_eq!(aspect, AspectRatio::Square);
        assert_eq!(info.sar, (1, 1));
    }
    let enc = Encoder::new(EncoderConfig { aspect: AspectRatio::Dar16x9, ..EncoderConfig::new(720, 576, 30000, 1001) }).unwrap();
    let info = probe(enc.sequence_header()).unwrap();
    assert_eq!((info.dar, info.frame_rate), (Some((16, 9)), Some((30000, 1001))));
    // without B pictures the stream is low delay
    let enc = Encoder::new(EncoderConfig { b_pictures: 0, ..EncoderConfig::new(720, 576, 25, 1) }).unwrap();
    assert!(probe(enc.sequence_header()).unwrap().low_delay);
}

#[test]
fn bad_configurations_are_refused() {
    let ok = EncoderConfig::new(64, 64, 25, 1);
    let bad = [
        EncoderConfig { width: 63, ..ok.clone() },
        EncoderConfig { height: 0, ..ok.clone() },
        EncoderConfig { width: 3840, height: 2160, ..ok.clone() },
        EncoderConfig { chroma: ChromaFormat::Yuv444, ..ok.clone() },
        EncoderConfig { chroma: ChromaFormat::Yuv422, width: 1920, height: 1152, ..ok.clone() },
        EncoderConfig { fps_num: 26, ..ok.clone() },
        EncoderConfig { fps_den: 0, ..ok.clone() },
        EncoderConfig { gop_size: 0, ..ok.clone() },
        EncoderConfig { gop_size: 1024, ..ok.clone() },
        EncoderConfig { b_pictures: 5, ..ok.clone() },
        EncoderConfig { gop_size: 1023, b_pictures: 2, ..ok.clone() },
        EncoderConfig { rate: RateControl::Quantiser(0), ..ok.clone() },
        EncoderConfig { rate: RateControl::Quantiser(32), ..ok.clone() },
        EncoderConfig { rate: RateControl::Abr { kbps: 0 }, ..ok.clone() },
        EncoderConfig { rate: RateControl::Abr { kbps: 90_000 }, ..ok.clone() },
        EncoderConfig { intra_dc_bits: 11, ..ok.clone() },
        EncoderConfig { intra_matrix: Some([0; 64]), ..ok.clone() },
        EncoderConfig { search_range: 0, ..ok.clone() },
        EncoderConfig { search_range: 64, ..ok.clone() },
    ];
    for cfg in bad {
        let desc = format!("{cfg:?}");
        assert!(matches!(Encoder::new(cfg), Err(EncodeError::Config(_))), "{desc}");
    }
}

#[test]
fn short_or_inconsistent_planes_are_refused() {
    let mut enc = Encoder::new(EncoderConfig { b_pictures: 0, ..EncoderConfig::new(32, 32, 25, 1) }).unwrap();
    let (y, c) = (vec![0u8; 32 * 32], vec![0u8; 16 * 16]);
    let ok = YuvFrame { y: &y, cb: &c, cr: &c, y_stride: 32, c_stride: 16 };
    assert!(enc.encode(&ok, 0).is_ok());
    let bad = [
        YuvFrame { y: &y[..32 * 31], ..ok },
        YuvFrame { cb: &c[..10], ..ok },
        YuvFrame { y_stride: 31, ..ok },
        YuvFrame { c_stride: usize::MAX, ..ok },
        YuvFrame { y_stride: usize::MAX / 2, ..ok },
    ];
    for f in bad {
        assert!(matches!(enc.encode(&f, 1), Err(EncodeError::BadFrame(_))));
    }
    // a refused frame leaves the encoder usable
    assert_eq!(enc.encode(&ok, 1).unwrap()[0].picture_type, PictureType::P);
}
