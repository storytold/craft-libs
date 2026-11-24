//! Encoder tests against this crate's decoder: the decoded pictures must equal the encoder's
//! reconstruction exactly (any difference would drift along a GOP of P pictures). The ffmpeg
//! comparison is in `tests/mpeg2/encode.rs`.

use super::*;
use crate::mpeg2::{Decoder, probe};
use picture::{Fallback, qscale};

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

/// Uniform noise, different in every picture: as hard to code as content gets.
fn noise(w: usize, h: usize, cw: usize, ch: usize, t: usize) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut x = 0x9E37_79B9_7F4A_7C15u64 ^ (t as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93);
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x >> 24) as u8
    };
    let y = (0..w * h).map(|_| next()).collect();
    let cb = (0..cw * ch).map(|_| next()).collect();
    let cr = (0..cw * ch).map(|_| next()).collect();
    (y, cb, cr)
}

/// The scene's first picture, every time.
fn still(w: usize, h: usize, cw: usize, ch: usize, _t: usize) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    scene(w, h, cw, ch, 0)
}

type Source = fn(usize, usize, usize, usize, usize) -> (Vec<u8>, Vec<u8>, Vec<u8>);

/// Encode `frames` pictures of the scene; returns the stream and the reconstruction.
fn encode(cfg: EncoderConfig, frames: usize) -> (Vec<u8>, Vec<ReconFrame>, Vec<Packet>) {
    encode_from(cfg, frames, scene)
}

fn encode_from(cfg: EncoderConfig, frames: usize, source: Source) -> (Vec<u8>, Vec<ReconFrame>, Vec<Packet>) {
    let (w, h) = (cfg.width as usize, cfg.height as usize);
    let (cw, ch) = (w / 2, if cfg.chroma == ChromaFormat::Yuv422 { h } else { h / 2 });
    let mut enc = Encoder::new(cfg).unwrap();
    enc.set_recon_capture(true);
    let mut es = Vec::new();
    let mut packets = Vec::new();
    for t in 0..frames {
        let (y, cb, cr) = source(w, h, cw, ch, t);
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
    assert_decodes_to_recon_from(name, cfg, frames, scene)
}

fn assert_decodes_to_recon_from(name: &str, cfg: EncoderConfig, frames: usize, source: Source) -> Vec<Packet> {
    let (_, recon, packets) = encode_from(cfg, frames, source);
    assert!(packets.windows(2).all(|w| w[0].dts < w[1].dts) && packets.iter().all(|p| p.dts <= p.pts), "{name}: decoding times");
    let pics = decode(&packets);
    assert_eq!(pics.len(), frames, "{name}: picture count");
    assert_eq!(recon.len(), frames, "{name}: reconstruction count");
    for (i, (p, r)) in pics.iter().zip(&recon).enumerate() {
        assert_eq!(p.pts, i as i64, "{name}: display order");
        for (plane, (a, b)) in [("Y", (&p.y, &r.y)), ("Cb", (&p.cb, &r.cb)), ("Cr", (&p.cr, &r.cr))] {
            let first = a.iter().zip(b.iter()).position(|(x, y)| x != y);
            assert!(first.is_none() && a.len() == b.len(), "{name}: picture {i} ({:?}) {plane} differs at {first:?}", p.picture_type);
        }
    }
    packets
}

/// What [`check_vbv`] found.
#[derive(Debug)]
struct VbvRun {
    cbr: bool,
    /// Bit rate and buffer size from the sequence header.
    rate: u64,
    size: u64,
    /// Bits of the whole stream.
    bits: u64,
}

/// Start codes in `data`: (offset of the 00 00 01, code).
fn start_codes(data: &[u8]) -> Vec<(usize, u8)> {
    (0..data.len().saturating_sub(3)).filter(|&i| data[i..i + 3] == [0, 0, 1]).map(|i| (i, data[i + 3])).collect()
}

/// Check a stream (packets in decoding order) against the VBV buffer of H.262 Annex C, using
/// nothing but its bytes: the sequence header's bit rate and buffer size, each picture's size
/// and `vbv_delay`. With a constant bitrate, data arrives at the bit rate from the first byte and
/// picture n is decoded `vbv_delay` after its start code arrives: those times must be one
/// picture interval apart, each picture complete by its time, and the buffer never over full.
/// With a variable bitrate the buffer fills at the bit rate until full and each picture must be
/// in it at its decoding time.
fn check_vbv(name: &str, packets: &[Packet], (fps_num, fps_den): (u32, u32)) -> VbvRun {
    use crate::mpeg2::headers::{Extension, PictureHeader, SequenceHeader};
    let first = &packets[0].data;
    let codes = start_codes(first);
    let at = |code: u8| codes.iter().find(|c| c.1 == code).map(|c| c.0 + 4).unwrap();
    let seq = SequenceHeader::parse(&first[at(0xB3)..]).unwrap();
    let Extension::Sequence(ext) = Extension::parse(&first[at(0xB5)..]).unwrap() else { panic!("{name}: no sequence extension") };
    let rate = (u64::from(seq.bit_rate_value) | u64::from(ext.bit_rate_extension) << 18) * 400;
    let size = (u64::from(seq.vbv_buffer_size_value) | u64::from(ext.vbv_buffer_size_extension) << 10) * 16384;
    let (num, den) = (u128::from(fps_num), u128::from(fps_den));
    let (r, s) = (u128::from(rate), u128::from(size));
    let pics: Vec<(u64, u64, u16)> = packets
        .iter()
        .map(|p| {
            let pic = start_codes(&p.data).into_iter().find(|c| c.1 == 0).unwrap().0 + 4;
            (p.data.len() as u64 * 8, pic as u64 * 8, PictureHeader::parse(&p.data[pic..]).unwrap().vbv_delay)
        })
        .collect();
    let cbr = pics[0].2 != 0xFFFF;
    assert!(pics.iter().all(|p| (p.2 != 0xFFFF) == cbr), "{name}: some vbv_delay are 0xFFFF and some not");
    if cbr {
        // in bit·ticks (bits × 90000): picture n decodes at T = (bits up to its start code) ×
        // 90000 + vbv_delay × rate; one interval is rate × 90000 × den / num of them
        let mut before = 0u128;
        let mut last: Option<u128> = None;
        for (i, &(bits, head, delay)) in pics.iter().enumerate() {
            let (bits, head) = (u128::from(bits), u128::from(head));
            let t = (before + head) * 90_000 + u128::from(delay) * r;
            if let Some(l) = last {
                // each vbv_delay is rounded down to a tick
                let step = (t - l) * num;
                let want = r * 90_000 * den;
                assert!(step.abs_diff(want) <= 2 * r * num, "{name}: picture {i} decodes {step} after the one before, not {want}");
            }
            let arrived = t + r; // within the tick the delay was rounded down by
            assert!((before + bits) * 90_000 <= arrived, "{name}: picture {i} underflows ({bits} bits)");
            assert!(t <= (before + s) * 90_000, "{name}: the buffer overflows before picture {i}");
            before += bits;
            last = Some(t);
        }
    } else {
        // in 1/num bits: the buffer starts full and gains rate × den a picture, up to full
        let mut full = s * num;
        for (i, &(bits, _, _)) in pics.iter().enumerate() {
            let b = u128::from(bits) * num;
            assert!(b <= full, "{name}: picture {i} underflows ({bits} bits, {} in the buffer)", full / num);
            full = (full - b + r * den).min(s * num);
        }
    }
    VbvRun { cbr, rate, size, bits: pics.iter().map(|p| p.0).sum() }
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

#[test]
fn decoding_times_increase_after_a_lone_picture_is_flushed() {
    let mut enc = Encoder::new(EncoderConfig::new(32, 32, 25, 1)).unwrap();
    let (y, c) = (vec![0u8; 32 * 32], vec![0u8; 16 * 16]);
    let f = YuvFrame { y: &y, cb: &c, cr: &c, y_stride: 32, c_stride: 16 };
    // one picture, flushed before a second sets the step: it decodes one unit early
    assert!(enc.encode(&f, 100).unwrap().is_empty());
    let mut packets = enc.flush();
    assert_eq!(packets.iter().map(|p| (p.pts, p.dts)).collect::<Vec<_>>(), [(100, 99)]);
    // encoding goes on: the next picture decodes at the first one's presentation time
    for pts in [105, 110] {
        packets.extend(enc.encode(&f, pts).unwrap());
    }
    packets.extend(enc.flush());
    assert_eq!(packets.iter().map(|p| (p.pts, p.dts)).collect::<Vec<_>>(), [(100, 99), (105, 100), (110, 105)]);
    assert!(packets[1].keyframe);
}

#[test]
fn presentation_times_without_a_decoding_time_are_refused() {
    let (y, c) = (vec![0u8; 32 * 32], vec![0u8; 16 * 16]);
    let f = YuvFrame { y: &y, cb: &c, cr: &c, y_stride: 32, c_stride: 16 };
    // the decoding times are one step earlier: they must fit in an i64
    for (first, second) in [(i64::MIN + 1, i64::MIN + 3), (i64::MIN + 5, i64::MIN + 20)] {
        let mut enc = Encoder::new(EncoderConfig::new(32, 32, 25, 1)).unwrap();
        assert!(matches!(enc.encode(&f, i64::MIN), Err(EncodeError::BadTimestamp(_))));
        assert!(enc.encode(&f, first).unwrap().is_empty());
        assert!(matches!(enc.encode(&f, second), Err(EncodeError::BadTimestamp(_))), "{second} after {first}");
        // a refused time leaves the encoder usable
        assert_eq!(enc.flush().iter().map(|p| (p.pts, p.dts)).collect::<Vec<_>>(), [(first, first - 1)]);
    }
    // the lowest times that have one
    let mut enc = Encoder::new(EncoderConfig::new(32, 32, 25, 1)).unwrap();
    let mut packets = enc.encode(&f, i64::MIN + 1).unwrap();
    packets.extend(enc.encode(&f, i64::MIN + 2).unwrap());
    packets.extend(enc.flush());
    assert_eq!(packets.iter().map(|p| p.dts).collect::<Vec<_>>(), [i64::MIN, i64::MIN + 1]);
    // without B pictures every time passes through
    let mut enc = Encoder::new(EncoderConfig { b_pictures: 0, ..EncoderConfig::new(32, 32, 25, 1) }).unwrap();
    assert_eq!(enc.encode(&f, i64::MIN).unwrap()[0].dts, i64::MIN);
}

/// Average bitrate (bit/s) of the scene coded in groups of `group` pictures, flushed after each.
fn flushed_bitrate(cfg: EncoderConfig, group: usize, groups: usize) -> f64 {
    let (w, h) = (cfg.width as usize, cfg.height as usize);
    let fps = f64::from(cfg.fps_num) / f64::from(cfg.fps_den);
    let mut enc = Encoder::new(cfg).unwrap();
    let mut bytes = 0;
    for t in 0..group * groups {
        let (y, cb, cr) = scene(w, h, w / 2, h / 2, t);
        let f = YuvFrame { y: &y, cb: &cb, cr: &cr, y_stride: w, c_stride: w / 2 };
        bytes += enc.encode(&f, t as i64).unwrap().iter().map(|p| p.data.len()).sum::<usize>();
        if (t + 1) % group == 0 {
            bytes += enc.flush().iter().map(|p| p.data.len()).sum::<usize>();
        }
    }
    bytes as f64 * 8.0 * fps / (group * groups) as f64
}

#[test]
fn flushing_short_gops_keeps_the_average_bitrate() {
    // every flush ends a GOP of 12 after 4 pictures: the 8 that never came must not leave their
    // bits to the next GOP
    for b in [0, 2] {
        let cfg = EncoderConfig { gop_size: 12, b_pictures: b, rate: RateControl::Abr { kbps: 600 }, ..EncoderConfig::new(176, 144, 25, 1) };
        let got = flushed_bitrate(cfg, 4, 15);
        assert!((got / 600e3 - 1.0).abs() < 0.15, "b {b}: {got:.0} bit/s for 600000");
    }
}

#[test]
fn constant_bitrate_streams_meet_the_vbv_buffer() {
    let qcif = EncoderConfig { gop_size: 12, ..EncoderConfig::new(176, 144, 25, 1) };
    let cases: [(&str, EncoderConfig, Source); 9] = [
        ("scene", EncoderConfig { rate: RateControl::Cbr { kbps: 400 }, ..qcif.clone() }, scene),
        ("scene ip", EncoderConfig { rate: RateControl::Cbr { kbps: 400 }, b_pictures: 0, ..qcif.clone() }, scene),
        ("scene open b4", EncoderConfig { rate: RateControl::Cbr { kbps: 400 }, b_pictures: 4, closed_gop: false, ..qcif.clone() }, scene),
        ("intra", EncoderConfig { rate: RateControl::Cbr { kbps: 1500 }, gop_size: 1, ..qcif.clone() }, scene),
        ("small buffer", EncoderConfig { rate: RateControl::Cbr { kbps: 300 }, vbv_buffer_bits: Some(16384 * 3), ..qcif.clone() }, scene),
        ("noise", EncoderConfig { rate: RateControl::Cbr { kbps: 600 }, ..qcif.clone() }, noise),
        ("noise intra", EncoderConfig { rate: RateControl::Cbr { kbps: 600 }, gop_size: 1, ..qcif.clone() }, noise),
        ("still", EncoderConfig { rate: RateControl::Cbr { kbps: 1000 }, ..qcif.clone() }, still),
        (
            "422 29.97",
            EncoderConfig { rate: RateControl::Cbr { kbps: 900 }, chroma: ChromaFormat::Yuv422, fps_num: 30000, fps_den: 1001, ..qcif.clone() },
            scene,
        ),
    ];
    for (name, cfg, source) in cases {
        let (fps, frames) = ((cfg.fps_num, cfg.fps_den), 40);
        // every case is a constant bitrate (0 would fail the rate check below)
        let kbps = if let RateControl::Cbr { kbps } = cfg.rate { kbps } else { 0 };
        let packets = assert_decodes_to_recon_from(name, cfg, frames, source);
        let run = check_vbv(name, &packets, fps);
        assert!(run.cbr && run.rate == u64::from(kbps) * 1000, "{name}: {run:?}");
        // a constant bitrate: the stream is the bit rate's worth of its duration, give or take
        // what the buffer held at the start and holds at the end
        let want = run.rate as f64 * frames as f64 * f64::from(fps.1) / f64::from(fps.0);
        assert!((run.bits as f64 - want).abs() <= run.size as f64, "{name}: {} bits for {want:.0}", run.bits);
    }
}

#[test]
fn constant_bitrate_pads_pictures_that_would_overflow_the_buffer() {
    // a still picture costs next to nothing after the first: the rest must be stuffing
    let cfg = EncoderConfig { rate: RateControl::Cbr { kbps: 2000 }, gop_size: 12, ..EncoderConfig::new(176, 144, 25, 1) };
    let (_, _, packets) = encode_from(cfg, 24, still);
    let padded = packets.iter().filter(|p| p.data.ends_with(&[0; 64])).count();
    assert!(padded >= 12, "{padded} of {} pictures padded", packets.len());
    let run = check_vbv("still", &packets, (25, 1));
    assert!((run.bits as f64 / (2e6 * 24.0 / 25.0) - 1.0).abs() < 0.2, "{run:?}");
}

#[test]
fn variable_bitrate_streams_meet_the_vbv_buffer() {
    let qcif = EncoderConfig { gop_size: 12, ..EncoderConfig::new(176, 144, 25, 1) };
    let small = Some(16384 * 4);
    let cases: [(&str, EncoderConfig, Source); 7] = [
        ("vbr", EncoderConfig { rate: RateControl::Vbr { kbps: 300, max_kbps: 600 }, ..qcif.clone() }, scene),
        ("vbr noise", EncoderConfig { rate: RateControl::Vbr { kbps: 300, max_kbps: 600 }, vbv_buffer_bits: small, ..qcif.clone() }, noise),
        ("abr small buffer", EncoderConfig { rate: RateControl::Abr { kbps: 2000 }, vbv_buffer_bits: small, ..qcif.clone() }, noise),
        ("quantiser 1", EncoderConfig { rate: RateControl::Quantiser(1), vbv_buffer_bits: small, ..qcif.clone() }, noise),
        ("quantiser 1 intra", EncoderConfig { rate: RateControl::Quantiser(1), gop_size: 1, vbv_buffer_bits: small, ..qcif.clone() }, noise),
        ("quantiser 1 b4", EncoderConfig { rate: RateControl::Quantiser(1), b_pictures: 4, vbv_buffer_bits: small, ..qcif.clone() }, noise),
        ("default", qcif.clone(), scene),
    ];
    for (name, cfg, source) in cases {
        let packets = assert_decodes_to_recon_from(name, cfg, 30, source);
        let run = check_vbv(name, &packets, (25, 1));
        assert!(!run.cbr, "{name}: {run:?}");
    }
}

#[test]
fn pictures_too_large_for_the_buffer_are_coded_coarser() {
    // noise at quantiser 1 is far beyond a 64 kbit buffer: every picture must give way, down to
    // the coarsest quantiser or the fallbacks, and still decode to the reconstruction
    let cfg = EncoderConfig { rate: RateControl::Quantiser(1), vbv_buffer_bits: Some(16384 * 4), ..EncoderConfig::new(176, 144, 25, 1) };
    let q_max = qscale(cfg.nonlinear_quant, 31) as f32;
    let packets = assert_decodes_to_recon_from("noise", cfg, 24, noise);
    assert!(packets.iter().all(|p| p.quantiser_scale > 8.0), "{:?}", packets.iter().map(|p| p.quantiser_scale).collect::<Vec<_>>());
    // adaptive quantisation spreads the macroblocks of the coarsest pictures just under it
    assert!(packets.iter().any(|p| p.quantiser_scale > 0.95 * q_max));
    check_vbv("noise", &packets, (25, 1));
}

#[test]
fn the_smallest_pictures_do_not_depend_on_the_source() {
    let enc = Encoder::new(EncoderConfig { b_pictures: 2, ..EncoderConfig::new(176, 144, 25, 1) }).unwrap();
    let smallest = enc.smallest_picture();
    let frame = |t| {
        let (y, cb, cr) = noise(176, 144, 88, 72, t);
        enc.load(&YuvFrame { y: &y, cb: &cb, cr: &cr, y_stride: 176, c_stride: 88 }).unwrap()
    };
    let (a, b) = (frame(1), frame(2));
    let h = Held { src: frame(3), pts: 0, n: 0 };
    let sizes: Vec<u64> =
        [(PictureType::I, None, None), (PictureType::P, Some(&a), None), (PictureType::B, Some(&a), Some(&b)), (PictureType::B, None, Some(&b))]
            .into_iter()
            .map(|(t, fwd, bwd)| {
                let c = enc.code_picture(&h, t, (fwd, bwd), [&[], &[]], 30.0, Fallback::Minimal);
                // the minimal picture repeats its reference (grey for an I picture)
                let want = bwd.or(fwd).map_or(128, |r| r.planes[0][5]);
                assert_eq!(c.recon.planes[0][5], want, "{t:?}");
                c.bits()
            })
            .collect();
    assert_eq!(sizes.iter().max().copied(), Some(smallest), "{sizes:?}");
    // the same sizes for a grey source and references
    let mut grey = frame(0);
    for p in &mut grey.planes {
        p.fill(128);
    }
    let g = Held { src: grey.clone(), pts: 0, n: 0 };
    let i = enc.code_picture(&g, PictureType::I, (None, None), [&[], &[]], 30.0, Fallback::Minimal).bits();
    assert_eq!(i, sizes[0]);
}

#[test]
fn the_sequence_header_carries_the_rate_and_buffer() {
    let base = EncoderConfig::new(176, 144, 25, 1);
    for (rate, vbv, want_rate, want_vbv) in [
        (RateControl::Cbr { kbps: 1000 }, None, 1_000_000, 475_136 / 16384 * 16384),
        // rounded up to 400 bit/s
        (RateControl::Cbr { kbps: 1001 }, None, 1_001_200, 475_136 / 16384 * 16384),
        (RateControl::Vbr { kbps: 500, max_kbps: 1500 }, Some(200_000), 1_500_000, 196_608),
        // the level's maximum
        (RateControl::Abr { kbps: 500 }, None, 4_000_000, 475_136 / 16384 * 16384),
        (RateControl::Quantiser(4), None, 4_000_000, 475_136 / 16384 * 16384),
    ] {
        let enc = Encoder::new(EncoderConfig { rate, vbv_buffer_bits: vbv, ..base.clone() }).unwrap();
        let info = probe(enc.sequence_header()).unwrap();
        assert_eq!(info.bit_rate, want_rate, "{rate:?}");
        assert_eq!(enc.vbv_value * 16384, want_vbv, "{rate:?}");
    }
    // a larger buffer than the level holds moves to the next level
    let enc = Encoder::new(EncoderConfig { vbv_buffer_bits: Some(1_000_000), ..base.clone() }).unwrap();
    assert_eq!(enc.profile_and_level(), 0x48);
}

#[test]
fn bitrates_the_buffer_cannot_carry_are_refused() {
    let base = EncoderConfig::new(352, 288, 25, 1);
    let bad = [
        // 2000 bits a picture: less than the smallest CIF picture
        EncoderConfig { rate: RateControl::Cbr { kbps: 50 }, ..base.clone() },
        EncoderConfig { rate: RateControl::Vbr { kbps: 40, max_kbps: 50 }, ..base.clone() },
        // a buffer of less than two pictures
        EncoderConfig { rate: RateControl::Cbr { kbps: 4000 }, vbv_buffer_bits: Some(16384 * 9), ..base.clone() },
        // a buffer smaller than the smallest CIF picture
        EncoderConfig { rate: RateControl::Quantiser(4), vbv_buffer_bits: Some(16384), ..base.clone() },
        EncoderConfig { vbv_buffer_bits: Some(16000), ..base.clone() },
        // beyond every level
        EncoderConfig { vbv_buffer_bits: Some(10_000_000), ..base.clone() },
        EncoderConfig { rate: RateControl::Cbr { kbps: 0 }, ..base.clone() },
        EncoderConfig { rate: RateControl::Vbr { kbps: 900, max_kbps: 800 }, ..base.clone() },
        EncoderConfig { rate: RateControl::Cbr { kbps: 90_000 }, ..base.clone() },
    ];
    for cfg in bad {
        let desc = format!("{cfg:?}");
        assert!(matches!(Encoder::new(cfg), Err(EncodeError::Config(_))), "{desc}");
    }
    // the lowest bitrate that is accepted gives a stream that meets the buffer, noise or not
    let lowest = (1..4000).find(|&kbps| Encoder::new(EncoderConfig { rate: RateControl::Cbr { kbps }, ..base.clone() }).is_ok()).unwrap();
    let cfg = EncoderConfig { rate: RateControl::Cbr { kbps: lowest }, ..base.clone() };
    let packets = assert_decodes_to_recon_from("lowest", cfg, 14, noise);
    check_vbv("lowest", &packets, (25, 1));
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
