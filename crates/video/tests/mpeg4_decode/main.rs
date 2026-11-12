//! The MPEG-4 Part 2 decoder against ffmpeg as the writer of test streams and as the reference
//! decoder (skipped when ffmpeg is absent), plus checks that need no oracle: hand-built streams,
//! packed B-VOPs, a round trip through this crate's encoder and corrupted input.

use std::path::{Path, PathBuf};
use std::process::Command;

use craft_media::bitstream::BitWriter;
use craft_video::mpeg4::{DecodeError, Decoder, Picture, VopType};

fn ffmpeg() -> Option<PathBuf> {
    craft_media_testkit::ffmpeg_or_skip("mpeg4 decoder oracle")
}

fn out_dir(name: &str) -> PathBuf {
    let dir = craft_media_testkit::fixtures_dir(&format!("mpeg4dec/{name}"));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    dir
}

/// The moving test pattern the oracle streams encode: deterministic, with noise so that every
/// block has coefficients. A size ending in `c` (`"352x288c"`) is the clean pattern, whose still
/// background gives skipped and uncoded macroblocks.
fn source_filter(size: &str) -> String {
    match size.strip_suffix('c') {
        Some(size) => format!("testsrc2=size={size}:rate=25,format=yuv420p"),
        None => format!("testsrc2=size={size}:rate=25,noise=alls=12:allf=t,format=yuv420p"),
    }
}

fn run(cmd: &mut Command, what: &str) -> Vec<u8> {
    let out = cmd.output().expect("run ffmpeg");
    assert!(out.status.success(), "{what} failed: {}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

/// Encode `frames` frames of the test pattern with ffmpeg's `mpeg4` encoder and `args`.
fn encode(ff: &Path, name: &str, size: &str, frames: u32, args: &[&str]) -> Vec<u8> {
    let path = out_dir(name).join("stream.m4v");
    run(
        Command::new(ff)
            .args(["-hide_banner", "-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i", &source_filter(size), "-frames:v"])
            .arg(frames.to_string())
            .args(["-c:v", "mpeg4"])
            .args(args)
            .args(["-f", "m4v"])
            .arg(&path),
        "ffmpeg encode",
    );
    std::fs::read(&path).expect("read stream")
}

/// The uncompressed test pattern, yuv420p frames back to back.
fn source(ff: &Path, size: &str, frames: u32) -> Vec<u8> {
    run(
        Command::new(ff)
            .args(["-hide_banner", "-nostdin", "-v", "error", "-f", "lavfi", "-i", &source_filter(size), "-frames:v"])
            .arg(frames.to_string())
            .args(["-f", "rawvideo", "-pix_fmt", "yuv420p", "-"]),
        "ffmpeg source",
    )
}

/// ffmpeg's decode of a raw stream, yuv420p frames back to back.
fn reference(ff: &Path, stream: &[u8], name: &str) -> Vec<u8> {
    let path = out_dir(name).join("reference-input.m4v");
    std::fs::write(&path, stream).expect("write");
    run(
        Command::new(ff)
            .args(["-hide_banner", "-nostdin", "-v", "error", "-f", "m4v", "-i"])
            .arg(&path)
            .args(["-f", "rawvideo", "-pix_fmt", "yuv420p", "-"]),
        "ffmpeg decode",
    )
}

fn is_start(s: &[u8], i: usize) -> bool {
    s.len() > i + 3 && s[i] == 0 && s[i + 1] == 0 && s[i + 2] == 1
}

/// Split a raw stream into chunks that each end with one VOP; headers go with the next VOP.
fn chunks(stream: &[u8]) -> Vec<&[u8]> {
    let starts: Vec<usize> = (0..stream.len()).filter(|&i| is_start(stream, i)).collect();
    let mut out = Vec::new();
    let mut from = 0;
    for (k, &s) in starts.iter().enumerate() {
        if stream[s + 3] == 0xB6 {
            let end = starts.get(k + 1).copied().unwrap_or(stream.len());
            out.push(&stream[from..end]);
            from = end;
        }
    }
    out
}

fn decode_all(chunks: &[&[u8]]) -> (Vec<Picture>, Decoder) {
    let mut d = Decoder::new();
    let mut pics = Vec::new();
    for (i, c) in chunks.iter().enumerate() {
        pics.extend(d.decode(c, i as i64).expect("decode"));
    }
    pics.extend(d.flush());
    (pics, d)
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    let se: f64 = a.iter().zip(b).map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2)).sum();
    if se == 0.0 {
        return 99.0;
    }
    10.0 * (255.0f64 * 255.0 * a.len() as f64 / se).log10()
}

fn planes(p: &Picture) -> Vec<u8> {
    [&p.y[..], &p.u[..], &p.v[..]].concat()
}

/// The `i`-th frame of a buffer of frames shaped like `p`.
fn frame<'a>(p: &Picture, buf: &'a [u8], i: usize) -> &'a [u8] {
    let n = (p.width * p.height + 2 * p.chroma_width() * p.chroma_height()) as usize;
    &buf[i * n..(i + 1) * n]
}

struct Checked {
    pics: Vec<Picture>,
    reference: Vec<u8>,
    ff: PathBuf,
}

/// Encode with `args`, decode with this decoder and with ffmpeg; no errors, the same picture
/// count, and every picture (or every I/P picture with `only_references`) within `floor` dB of
/// ffmpeg's.
fn oracle(name: &str, size: &str, frames: u32, args: &[&str], floor: f64, only_references: bool) -> Option<Checked> {
    let ff = ffmpeg()?;
    let stream = encode(&ff, name, size, frames, args);
    let reference = reference(&ff, &stream, name);
    let (pics, dec) = decode_all(&chunks(&stream));
    assert_eq!(dec.errors(), 0, "{name}: {:?}", dec.last_error());
    assert_eq!(pics.len() as u32, frames, "{name}: picture count");
    assert_eq!(reference.len(), pics.len() * frame(&pics[0], &reference, 0).len(), "{name}: ffmpeg picture count");
    for (i, p) in pics.iter().enumerate() {
        if only_references && p.vop_type == VopType::B {
            continue;
        }
        let q = psnr(&planes(p), frame(p, &reference, i));
        assert!(q >= floor, "{name}: picture {i} ({:?}) is {q:.2} dB from ffmpeg's, below {floor}", p.vop_type);
    }
    Some(Checked { pics, reference, ff })
}

/// 45 dB leaves room for IDCT mismatch between decoders (each is held to IEEE 1180), which
/// accumulates over a GOP; a decoding error costs far more.
const FLOOR: f64 = 45.0;

#[test]
fn simple_profile_i_and_p() {
    oracle("simple", "176x144", 30, &["-qscale:v", "4", "-bf", "0", "-g", "12"], FLOOR, false);
}

#[test]
fn fine_quantiser() {
    oracle("q1", "176x144", 12, &["-qscale:v", "1", "-bf", "0", "-g", "6"], FLOOR, false);
}

/// 98×66 is not whole macroblocks: references are padded from the macroblock edge.
#[test]
fn coarse_quantiser_and_partial_macroblocks() {
    oracle("q31", "98x66", 20, &["-qscale:v", "31", "-bf", "0", "-g", "10"], FLOOR, false);
}

#[test]
fn b_vops_come_out_in_display_order() {
    let Some(c) = oracle("bframes", "176x144", 30, &["-qscale:v", "4", "-bf", "2", "-g", "12"], FLOOR, false) else { return };
    assert!(c.pics.iter().any(|p| p.vop_type == VopType::B));
    assert!(c.pics.windows(2).all(|w| w[0].time < w[1].time));
}

#[test]
fn four_vectors_and_ac_prediction() {
    oracle("mv4aic", "176x144", 30, &["-qscale:v", "3", "-bf", "0", "-flags", "+mv4+aic", "-g", "15"], FLOOR, false);
}

#[test]
fn quarter_sample() {
    oracle("qpel", "176x144", 30, &["-qscale:v", "3", "-bf", "0", "-flags", "+qpel", "-g", "15"], FLOOR, false);
}

#[test]
fn quarter_sample_four_vectors_b_vops() {
    oracle("qpelmv4b", "176x144", 30, &["-qscale:v", "3", "-bf", "2", "-flags", "+qpel+mv4+aic", "-g", "15"], FLOOR, false);
}

#[test]
fn mpeg_quantisation_with_custom_matrices() {
    oracle("mpegquant", "176x144", 30, &["-qscale:v", "4", "-bf", "1", "-mpeg_quant", "1", "-g", "12"], FLOOR, false);
    let intra = (0..64).map(|i| (8 + i / 2).to_string()).collect::<Vec<_>>().join(",");
    let inter = (0..64).map(|i| (16 + i / 4).to_string()).collect::<Vec<_>>().join(",");
    oracle(
        "matrices",
        "176x144",
        20,
        &["-qscale:v", "4", "-bf", "1", "-mpeg_quant", "1", "-intra_matrix", &intra, "-inter_matrix", &inter],
        FLOOR,
        false,
    );
}

#[test]
fn video_packets() {
    oracle("resync", "176x144", 20, &["-qscale:v", "4", "-bf", "1", "-ps", "300", "-g", "10"], FLOOR, false);
}

#[test]
fn large_vectors() {
    oracle("fcode", "352x288", 20, &["-qscale:v", "4", "-bf", "2", "-me_range", "60", "-g", "20"], FLOOR, false);
}

/// Clean content: skipped P macroblocks skip their B co-located macroblocks, and B macroblocks
/// use direct mode without a vector difference (`modb` 1).
#[test]
fn skipped_and_uncoded_macroblocks() {
    oracle("clean", "352x288c", 30, &["-qscale:v", "4", "-bf", "2", "-g", "15"], FLOOR, false);
    oracle("cleanasp", "352x288c", 30, &["-qscale:v", "3", "-bf", "2", "-flags", "+qpel+mv4+aic", "-mpeg_quant", "1", "-g", "15"], FLOOR, false);
    oracle("cleanresync", "352x288c", 20, &["-qscale:v", "5", "-bf", "1", "-ps", "400"], FLOOR, false);
}

#[test]
fn interlaced() {
    oracle("interlaced", "176x144", 20, &["-qscale:v", "4", "-bf", "0", "-flags", "+ildct+ilme", "-g", "10"], FLOOR, false);
}

/// Field direct mode (§7.6.9.5.2) scales the co-located macroblock's field vectors. FFmpeg's
/// decoder predicts these macroblocks from the vector difference alone, so its pictures are no
/// reference for B-VOPs here: every B picture must instead be at least as close to the source as
/// ffmpeg's. I- and P-VOPs must match ffmpeg's as usual.
fn interlaced_b(name: &str, flags: &str) {
    let Some(c) = oracle(name, "176x144", 20, &["-qscale:v", "4", "-bf", "2", "-flags", flags, "-g", "10"], FLOOR, true) else { return };
    let src = source(&c.ff, "176x144", 20);
    let mut ahead = 0;
    for (i, p) in c.pics.iter().enumerate().filter(|(_, p)| p.vop_type == VopType::B) {
        let ours = psnr(&planes(p), frame(p, &src, i));
        let theirs = psnr(frame(p, &c.reference, i), frame(p, &src, i));
        assert!(ours >= theirs - 0.02, "{name}: B picture {i} is {ours:.2} dB from the source, ffmpeg's {theirs:.2}");
        ahead += usize::from(ours > theirs + 0.1);
    }
    assert!(ahead > 0, "{name}: no field direct macroblocks were exercised");
}

#[test]
fn interlaced_b_vops() {
    interlaced_b("interlacedb", "+ildct+ilme");
}

#[test]
fn interlaced_quarter_sample_b_vops() {
    interlaced_b("interlacedqpel", "+ildct+ilme+qpel");
}

#[test]
fn interlaced_clean_content() {
    oracle("interlacedclean", "352x288c", 20, &["-qscale:v", "4", "-bf", "2", "-flags", "+ildct+ilme", "-g", "10"], FLOOR, true);
}

#[test]
fn odd_dimensions_crop_to_the_coded_size() {
    let Some(c) = oracle("odd", "50x38", 6, &["-qscale:v", "5", "-bf", "1"], FLOOR, false) else { return };
    let p = &c.pics[0];
    assert_eq!((p.width, p.height, p.chroma_width(), p.chroma_height()), (50, 38, 25, 19));
    assert_eq!((p.y.len(), p.u.len()), (50 * 38, 25 * 19));
}

#[test]
fn stream_info_from_headers() {
    let Some(ff) = ffmpeg() else { return };
    let stream = encode(&ff, "info", "176x144", 3, &["-qscale:v", "4", "-bf", "1", "-flags", "+qpel", "-aspect", "16:9"]);
    let info = craft_video::mpeg4::probe(&stream).expect("probe");
    assert_eq!((info.width, info.height), (176, 144));
    assert!(info.quarter_sample && !info.interlaced && !info.low_delay);
    // 16:9 display of 176×144 pixels.
    assert_eq!(176 * u32::from(info.sar.0) * 9, 144 * u32::from(info.sar.1) * 16);
    assert!(info.codec_name().starts_with("MPEG-4 Visual"));
}

/// Packed B-VOPs as DivX and Xvid store them in AVI: the P-VOP after a run of B-VOPs travels in
/// one chunk with the first of them, and an uncoded VOP takes the freed slot. The pictures are
/// the same as from the plain stream.
#[test]
fn packed_b_vops_and_uncoded_vops() {
    let Some(ff) = ffmpeg() else { return };
    let stream = encode(&ff, "packed", "176x144", 13, &["-qscale:v", "4", "-bf", "1", "-g", "6"]);
    let plain = chunks(&stream);
    let (want, _) = decode_all(&plain);
    let resolution = craft_video::mpeg4::probe(&stream).expect("probe").time_resolution;
    let not_coded = {
        // P-VOP, modulo_time_base 0, time increment 0, vop_coded 0.
        let bits = (32 - resolution.saturating_sub(1).leading_zeros()).max(1);
        let mut w = BitWriter::new();
        w.write_bits(0x1B6, 32);
        w.write_bits(0b01, 2);
        w.write_bits(0b0_1, 2);
        w.write_bits(0, bits);
        w.write_bits(0b1_0, 2);
        while !w.is_byte_aligned() {
            w.write_bit(true);
        }
        w.finish()
    };
    let is_b = |c: &[u8]| c.windows(4).position(|w| w == [0, 0, 1, 0xB6]).is_some_and(|i| c.get(i + 4).is_some_and(|b| b >> 6 == 2));
    let mut packed: Vec<Vec<u8>> = Vec::new();
    let mut k = 0;
    while k < plain.len() {
        if k + 1 < plain.len() && is_b(plain[k + 1]) {
            packed.push([plain[k], plain[k + 1]].concat());
            packed.push(not_coded.clone());
            k += 2;
        } else {
            packed.push(plain[k].to_vec());
            k += 1;
        }
    }
    assert!(packed.len() == plain.len() && packed.iter().any(|c| c == &not_coded));
    let refs: Vec<&[u8]> = packed.iter().map(|c| &c[..]).collect();
    let (got, dec) = decode_all(&refs);
    assert_eq!(dec.errors(), 0);
    assert_eq!(got.len(), want.len());
    for (a, b) in got.iter().zip(&want) {
        assert_eq!((a.vop_type, a.time), (b.vop_type, b.time));
        assert_eq!(planes(a), planes(b));
    }
}

#[cfg(feature = "mpeg4-encode")]
/// `warping_mv_code` (Table B-33 length, then the value).
fn warping_mv(w: &mut BitWriter, d: i32) {
    const LEN: [(u32, u32); 15] =
        [(0, 2), (2, 3), (3, 3), (4, 3), (5, 3), (6, 3), (14, 4), (30, 5), (62, 6), (126, 7), (254, 8), (510, 9), (1022, 10), (2046, 11), (4094, 12)];
    let len = 32 - d.unsigned_abs().leading_zeros();
    w.write_bits(LEN[len as usize].0, LEN[len as usize].1);
    if len > 0 {
        let v = if d > 0 { d } else { d + (1 << len) - 1 };
        w.write_bits(v as u32, len);
    }
    w.write_bit(true);
}

#[cfg(feature = "mpeg4-encode")]
/// Configuration headers of an Advanced Simple stream with GMC: the layer the crate's encoder
/// writes, but version 2 with `sprite_enable` = GMC.
fn gmc_headers(width: u32, height: u32, points: u32, accuracy: u32, quarter_sample: bool) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(0x1B0, 32);
    w.write_bits(0xF5, 8);
    w.write_bits(0x1B5, 32);
    w.write_bit(false); // is_visual_object_identifier
    w.write_bits(1, 4); // video
    w.write_bit(false); // video_signal_type
    w.write_bits(0b01, 2); // next_start_code
    w.write_bits(0x100, 32);
    w.write_bits(0x120, 32);
    w.write_bit(false);
    w.write_bits(17, 8); // Advanced Simple
    w.write_bit(true);
    w.write_bits(2, 4); // video_object_layer_verid
    w.write_bits(1, 3);
    w.write_bits(1, 4); // square pixels
    w.write_bit(true);
    w.write_bits(1, 2);
    w.write_bit(true); // low_delay
    w.write_bit(false);
    w.write_bits(0, 2);
    w.write_bit(true);
    w.write_bits(25, 16);
    w.write_bit(true);
    w.write_bit(true);
    w.write_bits(1, 5);
    w.write_bit(true);
    w.write_bits(width, 13);
    w.write_bit(true);
    w.write_bits(height, 13);
    w.write_bit(true);
    w.write_bit(false); // interlaced
    w.write_bit(true); // obmc_disable
    w.write_bits(2, 2); // sprite_enable: GMC
    w.write_bits(points, 6);
    w.write_bits(accuracy, 2);
    w.write_bit(false); // sprite_brightness_change
    w.write_bit(false); // not_8_bit
    w.write_bit(false); // quant_type
    w.write_bit(quarter_sample);
    w.write_bit(true); // complexity_estimation_disable
    w.write_bit(true); // resync_marker_disable
    w.write_bit(false); // data_partitioned
    w.write_bit(false); // newpred_enable
    w.write_bit(false); // reduced_resolution_vop_enable
    w.write_bit(false); // scalability
    w.write_bit(false);
    while !w.is_byte_aligned() {
        w.write_bit(true);
    }
    w.finish()
}

#[cfg(feature = "mpeg4-encode")]
/// An S-VOP whose macroblocks cycle through skipped (warped), `mcsel` without residual, and
/// inter with a zero vector difference (so its vector is the predictor from GMC neighbours).
fn s_vop(mbs: usize, time: u32, warp: &[(i32, i32)], rounding: bool) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(0x1B6, 32);
    w.write_bits(0b11, 2);
    w.write_bit(false); // modulo_time_base
    w.write_bit(true);
    w.write_bits(time, 5);
    w.write_bit(true);
    w.write_bit(true); // vop_coded
    w.write_bit(rounding);
    w.write_bits(0, 3); // intra_dc_vlc_thr
    for &(du, dv) in warp {
        warping_mv(&mut w, du);
        warping_mv(&mut w, dv);
    }
    w.write_bits(6, 5); // vop_quant
    w.write_bits(1, 3); // vop_fcode_forward
    for i in 0..mbs {
        let fields: &[(u32, u32)] = match i % 3 {
            // not_coded
            0 => &[(1, 1)],
            // coded, mcbpc (inter, cbpc 0), mcsel, cbpy (none coded)
            1 => &[(0, 1), (1, 1), (1, 1), (3, 2)],
            // coded, mcbpc, no mcsel, cbpy, motion_code 0 for x and y
            _ => &[(0, 1), (1, 1), (0, 1), (3, 2), (1, 1), (1, 1)],
        };
        for &(v, n) in fields {
            w.write_bits(v, n);
        }
    }
    w.write_bit(false);
    while !w.is_byte_aligned() {
        w.write_bit(true);
    }
    w.finish()
}

/// Global motion compensation with one, two and three warping points at each accuracy, checked
/// against ffmpeg on hand-built S-VOPs that follow an I-VOP of this crate's encoder.
#[cfg(feature = "mpeg4-encode")]
#[test]
fn global_motion_compensation() {
    use craft_video::mpeg4::{Encoder, EncoderConfig, YuvFrame};
    let Some(ff) = ffmpeg() else { return };
    let (w, h) = (96u32, 64u32);
    let y: Vec<u8> = (0..w * h).map(|i| ((i % w) * 5 + (i / w) * 3 + (i * 7919 % 23)) as u8).collect();
    let u: Vec<u8> = (0..w * h / 4).map(|i| (100 + (i % 48) * 2) as u8).collect();
    let v: Vec<u8> = (0..w * h / 4).map(|i| (160 - (i / 48) * 3) as u8).collect();
    let mut cfg = EncoderConfig::new(w, h, 25, 1);
    cfg.headers_in_band = false;
    let mut enc = Encoder::new(cfg).expect("encoder");
    let intra = enc.encode(&YuvFrame { y: &y, u: &u, v: &v, y_stride: w as usize, uv_stride: w as usize / 2 }).expect("encode").data;
    let mbs = (w as usize / 16) * (h as usize / 16);
    let warps: [&[(i32, i32)]; 3] = [&[(5, -3)], &[(3, -2), (7, 4)], &[(-2, 3), (6, -5), (-4, 9)]];
    let cases = (1..=3u32).flat_map(|p| (0..4u32).map(move |a| (p, a, false))).chain([(2, 3, true), (3, 3, true), (1, 2, true)]);
    for (points, accuracy, qpel) in cases {
        {
            let name = format!("gmc{points}{accuracy}{}", if qpel { "q" } else { "" });
            let mut stream = gmc_headers(w, h, points, accuracy, qpel);
            stream.extend_from_slice(&intra);
            let warp = warps[points as usize - 1];
            for t in 1..4u32 {
                let scaled: Vec<(i32, i32)> = warp.iter().map(|&(a, b)| (a * t as i32, b - t as i32)).collect();
                stream.extend(s_vop(mbs, t, &scaled, t % 2 == 0));
            }
            let reference = reference(&ff, &stream, &name);
            let (pics, dec) = decode_all(&chunks(&stream));
            assert_eq!(dec.errors(), 0, "{name}: {:?}", dec.last_error());
            assert_eq!(pics.len(), 4, "{name}");
            assert!(pics[0].info.gmc && pics[0].info.quarter_sample == qpel);
            assert_eq!(pics[1].vop_type, VopType::S);
            // With one point at half-sample accuracy, ffmpeg rounds a negative mean vector of a GMC
            // macroblock one lower than the nearest integer, which moves the inter macroblocks
            // predicted from it. Its warped macroblocks still match.
            if (points, accuracy) == (1, 0) {
                continue;
            }
            for (i, p) in pics.iter().enumerate() {
                let q = psnr(&planes(p), frame(p, &reference, i));
                assert!(q >= FLOOR, "{name}: picture {i} is {q:.2} dB from ffmpeg's");
            }
        }
    }
}

/// This crate's encoder and decoder share the IDCT, so the decoder reproduces the encoder's
/// reconstruction exactly.
#[cfg(feature = "mpeg4-encode")]
#[test]
fn round_trip_with_the_encoder_is_exact() {
    use craft_video::mpeg4::{Encoder, EncoderConfig, RateControl, YuvFrame};
    for (w, h, q) in [(64u32, 48u32, 2u8), (98, 66, 9), (34, 18, 31)] {
        let mut cfg = EncoderConfig::new(w, h, 30000, 1001);
        cfg.rate = RateControl::Quant(q);
        cfg.keyint = 7;
        let mut enc = Encoder::new(cfg).expect("encoder");
        let mut dec = Decoder::new();
        let (cw, ch) = (w as usize / 2, h as usize / 2);
        for t in 0..16usize {
            let y: Vec<u8> = (0..(w * h) as usize).map(|i| ((i % w as usize) * 3 + (i / w as usize + t) * 2 + (i * 31 % 7)) as u8).collect();
            let u: Vec<u8> = (0..cw * ch).map(|i| (90 + (i + t) % 50) as u8).collect();
            let v: Vec<u8> = (0..cw * ch).map(|i| (170 - (i / cw + t) % 40) as u8).collect();
            let packet = enc.encode(&YuvFrame { y: &y, u: &u, v: &v, y_stride: w as usize, uv_stride: cw }).expect("encode");
            let pics = dec.decode(&packet.data, packet.pts).expect("decode");
            assert_eq!(pics.len(), 1, "low delay: one picture per packet");
            let r = enc.reconstruction().expect("reconstruction");
            assert!(pics[0].y == r.y && pics[0].u == r.u && pics[0].v == r.v, "{w}x{h} q{q} frame {t}");
        }
        assert_eq!(dec.errors(), 0);
    }
}

#[test]
fn corrupt_streams_never_panic() {
    let Some(ff) = ffmpeg() else { return };
    for (name, args) in [
        ("fuzz-asp", &["-qscale:v", "6", "-bf", "2", "-flags", "+qpel+mv4+aic", "-g", "5"][..]),
        ("fuzz-resync", &["-qscale:v", "6", "-bf", "1", "-ps", "200", "-mpeg_quant", "1"][..]),
        ("fuzz-interlaced", &["-qscale:v", "6", "-bf", "1", "-flags", "+ildct+ilme"][..]),
    ] {
        let stream = encode(&ff, name, "64x48", 6, args);
        craft_media_testkit::hostile::hammer(name, &stream, 300, |data| {
            let mut d = Decoder::new();
            for c in chunks(data) {
                let _ = d.decode(c, 0);
            }
            let _ = d.flush();
        });
    }
}

#[test]
fn damaged_packets_are_concealed() {
    let Some(ff) = ffmpeg() else { return };
    let stream = encode(&ff, "conceal", "176x144", 6, &["-qscale:v", "4", "-bf", "0", "-ps", "200", "-g", "6"]);
    let mut parts: Vec<Vec<u8>> = chunks(&stream).iter().map(|c| c.to_vec()).collect();
    // Garble the middle of the second VOP: its other packets still decode.
    let p = &mut parts[1];
    let mid = p.len() / 2;
    p[mid..mid + 8].fill(0x5A);
    let mut d = Decoder::new();
    let mut pics = Vec::new();
    for c in &parts {
        pics.extend(d.decode(c, 0).expect("a damaged VOP still decodes"));
    }
    pics.extend(d.flush());
    assert_eq!(pics.len(), 6);
    assert!(d.errors() >= 1);
}

#[test]
fn hostile_headers_are_errors() {
    let mut d = Decoder::new();
    assert!(matches!(d.decode(&[0, 0, 1, 0xB6, 0xFF], 0), Err(DecodeError::Invalid(_))));
    // A VOL of 8191×8191 against a small memory budget.
    let mut w = BitWriter::new();
    w.write_bits(0x120, 32);
    w.write_bit(false); // random_accessible_vol
    w.write_bits(1, 8); // Simple
    w.write_bit(false); // is_object_layer_identifier
    w.write_bits(1, 4); // square pixels
    w.write_bit(false); // vol_control_parameters
    w.write_bits(0, 2); // rectangular
    w.write_bit(true);
    w.write_bits(25, 16);
    w.write_bit(true);
    w.write_bit(false); // fixed_vop_rate
    w.write_bit(true);
    w.write_bits(8191, 13);
    w.write_bit(true);
    w.write_bits(8191, 13);
    w.write_bit(true);
    // interlaced, obmc_disable, sprite_enable, not_8_bit, quant_type, complexity_estimation_disable,
    // resync_marker_disable, data_partitioned, scalability
    for b in [false, true, false, false, false, true, true, false, false] {
        w.write_bit(b);
    }
    let vol = w.finish();
    let mut d = Decoder::new();
    d.set_memory_budget(1 << 20);
    assert!(matches!(d.decode_headers(&vol), Err(DecodeError::TooLarge(8191, 8191))));
    let mut d = Decoder::new();
    assert_eq!(d.decode_headers(&vol), Ok(()));
    assert_eq!(d.info().map(|i| (i.width, i.height)), Some((8191, 8191)));
}
