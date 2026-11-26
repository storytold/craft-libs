//! High 4:2:2 and High 10 streams against ffmpeg: libx264 encodes a test pattern (CABAC and
//! CAVLC, I, P and B pictures), the decoder's output must equal ffmpeg's decode sample for sample.
//! ffmpeg is an external test oracle only (skipped when absent; `CRAFT_MEDIA_FFMPEG` names it).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;

use crate::h264::decode::Decoder;

const W: usize = 176;
const H: usize = 144;

/// Encode `frames` of `testsrc2` with libx264 (`pix_fmt`, `profile`, extra x264 params), and decode
/// it with ffmpeg to raw planar samples. None when ffmpeg is not available.
fn encode(name: &str, pix_fmt: &str, profile: &str, x264: &str, frames: usize) -> Option<(Vec<u8>, Vec<u8>)> {
    let ff = craft_media_testkit::ffmpeg_or_skip(name)?;
    let dir = craft_media_testkit::fixtures_dir("h264-hbd");
    let src = dir.join(format!("{name}.h264"));
    let yuv = dir.join(format!("{name}.yuv"));
    let size = format!("testsrc2=size={W}x{H}:rate=25");
    let st = Command::new(&ff)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", &size, "-frames:v", &frames.to_string()])
        .args(["-pix_fmt", pix_fmt, "-c:v", "libx264", "-profile:v", profile, "-x264-params", x264, "-f", "h264"])
        .arg(&src)
        .status()
        .unwrap();
    assert!(st.success(), "{name}: ffmpeg could not encode (libx264 missing?)");
    let st = Command::new(&ff)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&src)
        .args(["-f", "rawvideo", "-pix_fmt", pix_fmt])
        .arg(&yuv)
        .status()
        .unwrap();
    assert!(st.success(), "{name}: ffmpeg could not decode");
    Some((std::fs::read(&src).unwrap(), std::fs::read(&yuv).unwrap()))
}

/// Our decode of `stream` as the planes ffmpeg writes (Y, U, V per picture; 16-bit samples little
/// endian when `wide`).
fn ours(stream: &[u8], threads: usize, wide: bool) -> Vec<u8> {
    let mut dec = Decoder::with_threads(threads);
    let mut pics = dec.decode(stream, 0).unwrap();
    pics.extend(dec.flush());
    let mut out = Vec::new();
    for p in &pics {
        if wide {
            for plane in [&p.y16, &p.u16, &p.v16] {
                for s in plane.iter() {
                    out.extend_from_slice(&s.to_le_bytes());
                }
            }
        } else {
            out.extend_from_slice(&p.y);
            out.extend_from_slice(&p.u);
            out.extend_from_slice(&p.v);
        }
    }
    out
}

fn check(name: &str, pix_fmt: &str, profile: &str, x264: &str) {
    let Some((stream, want)) = encode(name, pix_fmt, profile, x264, 12) else { return };
    let wide = pix_fmt.contains("10");
    for threads in [1, 4] {
        let got = ours(&stream, threads, wide);
        assert_eq!(got.len(), want.len(), "{name} ({threads} threads): {} bytes decoded, ffmpeg {}", got.len(), want.len());
        if let Some(k) = got.iter().zip(&want).position(|(a, b)| a != b) {
            let frame = want.len() / 12;
            panic!("{name} ({threads} threads): picture {} differs from ffmpeg at byte {} of it", k / frame, k % frame);
        }
    }
}

#[test]
fn high_422_8bit_cabac_matches_ffmpeg() {
    check("422_8_cabac", "yuv422p", "high422", "keyint=8:bframes=2");
}

#[test]
fn high_422_8bit_cavlc_matches_ffmpeg() {
    check("422_8_cavlc", "yuv422p", "high422", "keyint=8:bframes=0:cabac=0");
}

#[test]
fn high_422_8bit_intra_8x8_matches_ffmpeg() {
    check("422_8_intra", "yuv422p", "high422", "keyint=1:8x8dct=1");
}

/// High QP (strong deblocking of the four horizontal 4:2:2 chroma edges), weighted prediction,
/// B pyramids and 8x8 transforms in inter macroblocks.
#[test]
fn high_422_8bit_deblocking_and_weighted_matches_ffmpeg() {
    check("422_8_hard", "yuv422p", "high422", "keyint=12:bframes=3:b-pyramid=normal:weightp=2:qp=40:8x8dct=1");
}

/// Low QP: large chroma DC levels (the 4:2:2 DC is scaled at QPc + 3, past the shift point at 36
/// only at high QPs, so both scaling branches are covered by this and the previous test).
#[test]
fn high_422_8bit_low_qp_matches_ffmpeg() {
    check("422_8_lowqp", "yuv422p", "high422", "keyint=6:bframes=1:qp=4");
}

#[test]
fn high10_cabac_matches_ffmpeg() {
    check("420_10_cabac", "yuv420p10le", "high10", "keyint=8:bframes=2");
}

#[test]
fn high10_cavlc_matches_ffmpeg() {
    check("420_10_cavlc", "yuv420p10le", "high10", "keyint=8:bframes=0:cabac=0");
}

#[test]
fn high10_intra_8x8_matches_ffmpeg() {
    check("420_10_intra", "yuv420p10le", "high10", "keyint=1:8x8dct=1");
}

/// Weighted prediction (offsets scaled to 10 bits), B pyramids and strong deblocking (alpha,
/// beta and tC0 scaled to 10 bits).
#[test]
fn high10_deblocking_and_weighted_matches_ffmpeg() {
    check("420_10_hard", "yuv420p10le", "high10", "keyint=12:bframes=3:b-pyramid=normal:weightp=2:qp=46:8x8dct=1");
}

/// Negative QPs (down to -QpBdOffsetY = -12): x264's QP scale starts at -QpBdOffsetY, so its
/// qp 2 codes SliceQPY -10, with adaptive quantisation moving macroblocks around it.
#[test]
fn high10_negative_qp_matches_ffmpeg() {
    check("420_10_negqp", "yuv420p10le", "high10", "keyint=6:bframes=1:qp=2");
    check("420_10_negcrf", "yuv420p10le", "high10", "keyint=6:bframes=1:crf=1:qpmin=0:aq-mode=2:aq-strength=2");
}

#[test]
fn high_422_10bit_cabac_matches_ffmpeg() {
    check("422_10_cabac", "yuv422p10le", "high422", "keyint=8:bframes=2");
}

#[test]
fn high_422_10bit_cavlc_matches_ffmpeg() {
    check("422_10_cavlc", "yuv422p10le", "high422", "keyint=8:bframes=0:cabac=0");
}

#[test]
fn high_422_10bit_intra_8x8_matches_ffmpeg() {
    check("422_10_intra", "yuv422p10le", "high422", "keyint=1:8x8dct=1");
}

#[test]
fn high_422_10bit_deblocking_and_weighted_matches_ffmpeg() {
    check("422_10_hard", "yuv422p10le", "high422", "keyint=12:bframes=3:b-pyramid=normal:weightp=2:qp=46:8x8dct=1");
}

/// Adaptive quantisation (varying mb_qp_delta, so chroma QPs below 30 and negative ones) at
/// a bitrate target.
#[test]
fn high_422_10bit_rate_controlled_matches_ffmpeg() {
    check("422_10_abr", "yuv422p10le", "high422", "keyint=12:bframes=2:bitrate=3000:aq-mode=2:aq-strength=1.5");
}

/// One IDR picture of 2 x 2 I_PCM macroblocks (CAVLC) with `bd` = (BitDepthY, BitDepthC) bits
/// per sample, 4:2:2 when `c422`. Each entry of `mbs` holds 256 luma samples then the Cb and
/// Cr blocks (8 x MbHeightC each), in raster order.
fn pcm_stream(bd: (u32, u32), c422: bool, mbs: &[Vec<u16>]) -> Vec<u8> {
    use craft_media::bitstream::{BitWriter, escape_rbsp};
    let nal = |t: u8, rbsp: &[u8]| -> Vec<u8> { [vec![0, 0, 0, 1, (3 << 5) | t], escape_rbsp(rbsp)].concat() };
    let mut w = BitWriter::new();
    w.write_bits(122, 8); // High 4:2:2
    w.write_bits(0, 8);
    w.write_bits(30, 8);
    w.write_ue(0); // sps id
    w.write_ue(if c422 { 2 } else { 1 }); // chroma_format_idc
    w.write_ue(bd.0 - 8);
    w.write_ue(bd.1 - 8);
    w.write_bit(false); // qpprime_y_zero_transform_bypass
    w.write_bit(false); // seq_scaling_matrix_present
    w.write_ue(0); // log2_max_frame_num - 4
    w.write_ue(2); // pic_order_cnt_type
    w.write_ue(0); // max_num_ref_frames
    w.write_bit(false); // gaps
    w.write_ue(1); // 2 macroblocks wide
    w.write_ue(1); // 2 macroblocks high
    w.write_bit(true); // frame_mbs_only
    w.write_bit(true); // direct_8x8_inference
    w.write_bit(false); // cropping
    w.write_bit(false); // vui
    w.rbsp_trailing();
    let mut stream = nal(7, &w.finish());
    let mut w = BitWriter::new();
    w.write_ue(0); // pps id
    w.write_ue(0); // sps id
    w.write_bit(false); // CAVLC
    w.write_bit(false);
    w.write_ue(0); // slice groups
    w.write_ue(0);
    w.write_ue(0);
    w.write_bit(false); // weighted_pred
    w.write_bits(0, 2);
    w.write_se(0); // pic_init_qp - 26
    w.write_se(0);
    w.write_se(0); // chroma_qp_index_offset
    w.write_bit(true); // deblocking_filter_control_present
    w.write_bit(false);
    w.write_bit(false);
    w.rbsp_trailing();
    stream.extend(nal(8, &w.finish()));
    let mut w = BitWriter::new();
    w.write_ue(0); // first_mb_in_slice
    w.write_ue(7); // I slice
    w.write_ue(0); // pps id
    w.write_bits(0, 4); // frame_num
    w.write_ue(0); // idr_pic_id
    w.write_bit(false); // no_output_of_prior_pics
    w.write_bit(false); // long_term_reference
    w.write_se(0); // slice_qp_delta
    w.write_ue(1); // deblocking off
    for mb in mbs {
        w.write_ue(25); // I_PCM
        w.align_zero();
        for (i, &s) in mb.iter().enumerate() {
            w.write_bits(u32::from(s), if i < 256 { bd.0 } else { bd.1 });
        }
    }
    w.rbsp_trailing();
    stream.extend(nal(5, &w.finish()));
    stream
}

/// I_PCM samples of BitDepthY / BitDepthC bits come out exactly (8.3.5), as 16-bit planes, for
/// 4:2:0 and 4:2:2, equal and different luma / chroma depths. libx264 never writes I_PCM.
#[test]
fn deep_pcm_macroblocks_decode_exactly() {
    let mut seed = 0x1234_5678u32;
    let mut next = move |bits: u32| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed >> (32 - bits)) as u16
    };
    for (bd, c422) in [((10, 10), false), ((10, 10), true), ((9, 10), true), ((10, 8), false), ((8, 10), true), ((8, 10), false)] {
        let hc = if c422 { 16 } else { 8 };
        let mbs: Vec<Vec<u16>> = (0..4).map(|_| (0..256 + 2 * 8 * hc).map(|i| next(if i < 256 { bd.0 } else { bd.1 })).collect()).collect();
        let (mut y, mut u, mut v) = (vec![0u16; 32 * 32], vec![0u16; 16 * 2 * hc], vec![0u16; 16 * 2 * hc]);
        for (m, s) in mbs.iter().enumerate() {
            let (mx, my) = (m % 2, m / 2);
            for r in 0..16 {
                y[(my * 16 + r) * 32 + mx * 16..][..16].copy_from_slice(&s[r * 16..r * 16 + 16]);
            }
            for r in 0..hc {
                u[(my * hc + r) * 16 + mx * 8..][..8].copy_from_slice(&s[256 + r * 8..256 + r * 8 + 8]);
                v[(my * hc + r) * 16 + mx * 8..][..8].copy_from_slice(&s[256 + 8 * hc + r * 8..256 + 8 * hc + r * 8 + 8]);
            }
        }
        let stream = pcm_stream(bd, c422, &mbs);
        for threads in [1, 2] {
            let mut dec = Decoder::with_threads(threads);
            let mut pics = dec.decode(&stream, 0).unwrap();
            pics.extend(dec.flush());
            assert_eq!(pics.len(), 1, "{bd:?} 4:2:{}", if c422 { 2 } else { 0 });
            let p = &pics[0];
            assert_eq!((p.bit_depth, p.bit_depth_chroma, p.chroma422), (bd.0 as u8, bd.1 as u8, c422));
            // one component deeper than 8 bits puts all three planes in 16 bits, (8, 10) too
            assert!(p.is_deep() && p.y.is_empty() && p.u.is_empty() && p.v.is_empty(), "deep pictures fill the 16-bit planes only");
            assert_eq!((&p.y16, &p.u16, &p.v16), (&y, &u, &v), "{bd:?} 4:2:{} ({threads} threads)", if c422 { 2 } else { 0 });
        }
    }
}

/// A deep I_PCM macroblock cut short is reported as an error (from `decode`, or from
/// `take_error` once the picture's job has run), never a panic or a read past the slice; a cut
/// anywhere else in the stream does not panic either.
#[test]
fn truncated_deep_pcm_is_an_error() {
    let mbs = vec![vec![512u16; 256 + 2 * 8 * 16]; 4];
    let stream = pcm_stream((10, 10), true, &mbs);
    // the slice NAL is the third one; its I_PCM payload is 4 x 5 120 bits after a few header bytes
    let starts: Vec<usize> = stream.windows(4).enumerate().filter(|(_, w)| *w == [0, 0, 0, 1]).map(|(i, _)| i).collect();
    let slice = starts[2];
    let payload_end = stream.len() - 1; // the last byte holds the rbsp stop bit
    let inside = [slice + 20, slice + 700, slice + 1500, payload_end - 200, payload_end - 10];
    for cut in inside {
        assert!(cut > slice + 8 && cut < payload_end, "cut {cut} is inside the PCM payload");
        for threads in [1, 2] {
            let mut dec = Decoder::with_threads(threads);
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let decoded = dec.decode(&stream[..cut], 0);
                let flushed = dec.flush();
                (decoded.is_err(), dec.take_error().is_some(), flushed.len())
            }));
            let Ok((decode_err, job_err, _)) = r else { panic!("cut at {cut} panicked") };
            assert!(decode_err || job_err, "cut at {cut} ({threads} threads): no error reported");
        }
    }
    for cut in [10, 30, stream.len() - 1] {
        let mut dec = Decoder::with_threads(1);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = dec.decode(&stream[..cut], 0);
            let _ = dec.flush();
            let _ = dec.take_error();
        }));
        assert!(r.is_ok(), "cut at {cut} panicked");
    }
}

/// `reset` (a seek) keeps the recycled buffers of deep pictures as it does for 8-bit ones.
#[test]
fn reset_keeps_the_deep_picture_pool() {
    let mbs = vec![vec![300u16; 256 + 2 * 8 * 8]; 4];
    let stream = pcm_stream((10, 10), false, &mbs);
    let mut dec = Decoder::with_threads(1);
    let _ = dec.decode(&stream, 0).unwrap();
    let _ = dec.flush();
    let (_, before) = dec.pooled_pictures();
    assert!(before >= 1, "a decoded deep picture returns its buffers to the 16-bit pool");
    dec.reset();
    assert_eq!(dec.pooled_pictures().1, before, "reset keeps the 16-bit pool");
    let mut pics = dec.decode(&stream, 0).unwrap();
    pics.extend(dec.flush());
    assert_eq!(pics.len(), 1);
    assert!(pics[0].y16.iter().all(|&s| s == 300), "a reused buffer decodes the same picture");
}
