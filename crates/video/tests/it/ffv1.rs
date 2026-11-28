//! FFV1 tests: round trips, hostile input, and ffmpeg as the reference (oracle; skipped when ffmpeg
//! is absent, a failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`).
//!
//! - Our packets, in an AVI with our configuration record, decode in ffmpeg to our source pixels.
//! - ffmpeg's FFV1 files (version 3, range coder, several pixel formats and slice counts) decode
//!   with our decoder to ffmpeg's pixels.
//! - So do ffmpeg's version 0, 1 and 3 files with the Golomb-Rice coder or either range coder table
//!   and non-keyframes, and damaged frames of these kinds are errors or wrong pixels, never panics.

use craft_media::avi::{AviWriter, MuxOptions, StreamSpec, VideoSpec};
use craft_video::ffv1::{Colorspace, Decoder, Encoder, EncoderConfig, Error, Frame};
use std::path::Path;
use std::process::Command;

fn noise(seed: &mut u32, mask: u32) -> u16 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    (*seed & mask) as u16
}

/// A frame of smooth gradients, noise bursts and flat areas.
fn frame(w: u32, h: u32, cs: Colorspace, bits: u8, shift: (u8, u8), planes: usize) -> Frame {
    let max = (1u32 << bits) - 1;
    let mut seed = 0x9E37_79B9u32;
    let mut ps = Vec::new();
    for p in 0..planes {
        let (pw, ph) = if cs == Colorspace::YCbCr && (p == 1 || p == 2) && planes >= 3 {
            ((w as usize).div_ceil(1 << shift.0), (h as usize).div_ceil(1 << shift.1))
        } else {
            (w as usize, h as usize)
        };
        let mut v = Vec::with_capacity(pw * ph);
        for y in 0..ph {
            for x in 0..pw {
                let s = match (y / 11 + p) % 3 {
                    0 => ((x * 5 + y * 3 + p * 40) as u32 * max / 600) % (max + 1),
                    1 => noise(&mut seed, max) as u32,
                    _ => max / 3,
                };
                v.push(s as u16);
            }
        }
        ps.push(v);
    }
    Frame { width: w, height: h, colorspace: cs, bit_depth: bits, chroma_shift: shift, planes: ps }
}

fn round_trip(f: &Frame, cfg: EncoderConfig) -> Frame {
    let enc = Encoder::new(f, cfg).expect("encoder");
    let config = enc.config_record();
    let packet = enc.encode(f).expect("encode");
    let dec = Decoder::new(&config, f.width, f.height).expect("decoder");
    dec.decode(&packet).expect("decode")
}

#[test]
fn round_trips_every_shape() {
    for (cs, bits, shift, planes) in [
        (Colorspace::YCbCr, 8u8, (0u8, 0u8), 1usize),
        (Colorspace::YCbCr, 8, (1, 1), 3),
        (Colorspace::YCbCr, 8, (1, 0), 3),
        (Colorspace::YCbCr, 8, (0, 0), 4),
        (Colorspace::YCbCr, 10, (1, 1), 3),
        (Colorspace::YCbCr, 12, (0, 0), 3),
        (Colorspace::YCbCr, 16, (1, 0), 3),
        (Colorspace::YCbCr, 16, (0, 0), 1),
        (Colorspace::Rgb, 8, (0, 0), 3),
        (Colorspace::Rgb, 8, (0, 0), 4),
        (Colorspace::Rgb, 10, (0, 0), 3),
        (Colorspace::Rgb, 12, (0, 0), 4),
        (Colorspace::Rgb, 15, (0, 0), 3),
    ] {
        for (w, h) in [(1, 1), (3, 5), (64, 48), (67, 41)] {
            let f = frame(w, h, cs, bits, shift, planes);
            for crc in [true, false] {
                let back = round_trip(&f, EncoderConfig { slice_crc: crc, ..EncoderConfig::default() });
                assert_eq!(back.planes, f.planes, "{cs:?} {bits}-bit {shift:?} {planes} planes {w}x{h} crc={crc}");
            }
        }
    }
}

#[test]
fn round_trips_with_several_slices() {
    let f = frame(96, 64, Colorspace::YCbCr, 8, (1, 1), 3);
    for (sh, sv) in [(2, 1), (1, 2), (3, 2), (4, 4)] {
        let back = round_trip(&f, EncoderConfig { slices_h: sh, slices_v: sv, slice_crc: true });
        assert_eq!(back.planes, f.planes, "{sh}x{sv} slices");
    }
    let rgb = frame(50, 30, Colorspace::Rgb, 8, (0, 0), 3);
    assert_eq!(round_trip(&rgb, EncoderConfig { slices_h: 3, slices_v: 2, slice_crc: true }).planes, rgb.planes);
}

#[test]
fn encoder_refuses_what_it_cannot_write() {
    let f = frame(8, 8, Colorspace::YCbCr, 8, (1, 1), 3);
    assert!(Encoder::new(&f, EncoderConfig { slices_h: 0, ..EncoderConfig::default() }).is_err());
    assert!(Encoder::new(&f, EncoderConfig { slices_h: 9, ..EncoderConfig::default() }).is_err());
    let mut bad = f.clone();
    bad.bit_depth = 7;
    assert!(Encoder::new(&bad, EncoderConfig::default()).is_err());
    let mut rgb16 = frame(8, 8, Colorspace::Rgb, 8, (0, 0), 3);
    rgb16.bit_depth = 16;
    assert!(Encoder::new(&rgb16, EncoderConfig::default()).is_err());
    let enc = Encoder::new(&f, EncoderConfig::default()).expect("encoder");
    let mut wrong = f.clone();
    wrong.planes[1].pop();
    assert!(enc.encode(&wrong).is_err());
    let mut over = f;
    over.planes[0][0] = 300;
    assert!(enc.encode(&over).is_err());
}

#[test]
fn hostile_input_is_an_error_not_a_panic() {
    let f = frame(24, 18, Colorspace::YCbCr, 8, (1, 1), 3);
    let enc = Encoder::new(&f, EncoderConfig { slices_h: 2, slices_v: 1, slice_crc: true }).expect("encoder");
    let config = enc.config_record();
    let packet = enc.encode(&f).expect("encode");
    // Every truncation of the configuration record and of the packet.
    for n in 0..config.len() {
        let _ = Decoder::new(&config[..n], 24, 18);
    }
    let dec = Decoder::new(&config, 24, 18).expect("decoder");
    for n in 0..packet.len() {
        let _ = dec.decode(&packet[..n]);
    }
    // Every single-bit flip: the CRCs catch them in a packet, and none panics.
    for i in 0..packet.len() {
        for bit in 0..8 {
            let mut c = packet.clone();
            c[i] ^= 1 << bit;
            assert!(dec.decode(&c).is_err(), "bit {bit} of byte {i} went unnoticed");
        }
    }
    for i in 0..config.len() {
        for bit in 0..8 {
            let mut c = config.clone();
            c[i] ^= 1 << bit;
            assert!(matches!(Decoder::new(&c, 24, 18), Err(Error::Crc(_))), "config bit {bit} of byte {i}");
        }
    }
    // Without slice CRCs, flips may decode to wrong pixels but must not panic.
    let enc = Encoder::new(&f, EncoderConfig { slice_crc: false, ..EncoderConfig::default() }).expect("encoder");
    let (config, packet) = (enc.config_record(), enc.encode(&f).expect("encode"));
    let dec = Decoder::new(&config, 24, 18).expect("decoder");
    for i in 0..packet.len() {
        for bit in 0..8 {
            let mut c = packet.clone();
            c[i] ^= 1 << bit;
            let _ = dec.decode(&c);
        }
    }
    // A huge frame size is refused before any allocation.
    assert!(Decoder::new(&config, u32::MAX, u32::MAX).is_err());
}

// ---- ffmpeg ----

fn ffmpeg_run(ff: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new(ff).args(["-v", "error", "-nostdin", "-y"]).args(args).output().expect("run ffmpeg");
    assert!(out.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

fn raw_bytes(f: &Frame) -> Vec<u8> {
    let mut out = Vec::new();
    for p in &f.planes {
        for &v in p {
            if f.bit_depth == 8 {
                out.push(v as u8);
            } else {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    out
}

/// `(ffmpeg pix_fmt, colourspace, bits, chroma shift, planes)`; RGB planes are G, B, R in ffmpeg's gbrp.
type Format = (&'static str, Colorspace, u8, (u8, u8), usize);

const FORMATS: &[Format] = &[
    ("gray", Colorspace::YCbCr, 8, (0, 0), 1),
    ("yuv420p", Colorspace::YCbCr, 8, (1, 1), 3),
    ("yuv422p", Colorspace::YCbCr, 8, (1, 0), 3),
    ("yuv444p", Colorspace::YCbCr, 8, (0, 0), 3),
    ("yuv420p10le", Colorspace::YCbCr, 10, (1, 1), 3),
    ("yuv444p12le", Colorspace::YCbCr, 12, (0, 0), 3),
    ("gray16le", Colorspace::YCbCr, 16, (0, 0), 1),
    ("yuv444p16le", Colorspace::YCbCr, 16, (0, 0), 3),
    ("gbrp", Colorspace::Rgb, 8, (0, 0), 3),
    ("gbrp10le", Colorspace::Rgb, 10, (0, 0), 3),
    ("gbrp12le", Colorspace::Rgb, 12, (0, 0), 3),
    ("yuva444p", Colorspace::YCbCr, 8, (0, 0), 4),
    ("gbrap", Colorspace::Rgb, 8, (0, 0), 4),
    ("gbrap12le", Colorspace::Rgb, 12, (0, 0), 4),
];

/// Formats only ffmpeg writes: our encoder stops at 15-bit RGB (the transform needs 17 bits).
const DECODE_ONLY: &[Format] = &[("gbrp16le", Colorspace::Rgb, 16, (0, 0), 3), ("yuv411p", Colorspace::YCbCr, 8, (2, 0), 3)];

/// ffmpeg's planar RGB is G, B, R (then alpha); ours is R, G, B (then alpha).
fn gbr_order(planes: &[Vec<u16>]) -> Vec<Vec<u16>> {
    let mut v = vec![planes[1].clone(), planes[2].clone(), planes[0].clone()];
    v.extend(planes[3..].iter().cloned());
    v
}

fn avi_with(frame: &Frame, config: &[u8], packet: &[u8]) -> Vec<u8> {
    let mut spec = VideoSpec::new(*b"FFV1", frame.width, frame.height, 25, 1);
    spec.format.extra = config.to_vec();
    let mut w = AviWriter::new(std::io::Cursor::new(Vec::new()), vec![StreamSpec::Video(spec)], MuxOptions::default()).expect("writer");
    w.write_chunk(0, packet, true).expect("chunk");
    w.finish().expect("finish").into_inner()
}

#[test]
fn ffmpeg_decodes_our_packets_to_our_pixels() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("ffv1");
    for &(pix_fmt, cs, bits, shift, planes) in FORMATS {
        for (slices_h, slices_v) in [(1, 1), (2, 2), (3, 2)] {
            let f = frame(96, 48, cs, bits, shift, planes);
            let enc = Encoder::new(&f, EncoderConfig { slices_h, slices_v, slice_crc: true }).expect("encoder");
            let avi = avi_with(&f, &enc.config_record(), &enc.encode(&f).expect("encode"));
            let path = dir.join(format!("ours_{pix_fmt}_{slices_h}x{slices_v}.avi"));
            std::fs::write(&path, avi).expect("write");
            let theirs = ffmpeg_run(&ff, &["-i", path.to_str().expect("path"), "-f", "rawvideo", "-pix_fmt", pix_fmt, "-"]);
            // gbrp is planar G, B, R; our RGB planes are R, G, B.
            let mut want = f.clone();
            if cs == Colorspace::Rgb {
                want.planes = gbr_order(&f.planes);
            }
            assert!(theirs == raw_bytes(&want), "{pix_fmt} {slices_h}x{slices_v}: ffmpeg's pixels differ from ours");
        }
    }
}

#[test]
fn our_decoder_reads_ffmpegs_files() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("ffv1");
    for &(pix_fmt, cs, bits, shift, planes) in FORMATS.iter().chain(DECODE_ONLY) {
        for slices in ["1", "4", "6"] {
            for (ctx, extra) in [("0", false), ("1", true)] {
                let path = dir.join(format!("ffmpeg_{pix_fmt}_s{slices}_c{ctx}.avi"));
                let lavfi = "testsrc2=size=96x48:rate=1";
                ffmpeg_run(
                    &ff,
                    &[
                        "-f",
                        "lavfi",
                        "-i",
                        lavfi,
                        "-frames:v",
                        "1",
                        "-vf",
                        &format!("format={pix_fmt}"),
                        "-c:v",
                        "ffv1",
                        "-level",
                        "3",
                        "-coder",
                        "1",
                        "-context",
                        ctx,
                        "-slices",
                        slices,
                        "-slicecrc",
                        "1",
                        "-g",
                        "1",
                        path.to_str().expect("path"),
                    ],
                );
                let _ = extra;
                let file = std::fs::read(&path).expect("read");
                let avi = craft_media::avi::open(&file[..]).expect("open avi");
                let stream = &avi.streams[0];
                let config = stream.video.as_ref().expect("video").extra.clone();
                let chunk = stream.chunks[0];
                let packet = file[chunk.offset as usize..chunk.offset as usize + chunk.size as usize].to_vec();
                let dec = Decoder::new(&config, 96, 48).unwrap_or_else(|e| panic!("{pix_fmt} s{slices} c{ctx}: config: {e}"));
                let got = dec.decode(&packet).unwrap_or_else(|e| panic!("{pix_fmt} s{slices} c{ctx}: decode: {e}"));
                let theirs = ffmpeg_run(&ff, &["-i", path.to_str().expect("path"), "-f", "rawvideo", "-pix_fmt", pix_fmt, "-"]);
                let mut want = got.clone();
                if cs == Colorspace::Rgb {
                    want.planes = gbr_order(&got.planes);
                }
                assert_eq!((got.colorspace, got.bit_depth, got.chroma_shift, got.planes.len()), (cs, bits, shift, planes), "{pix_fmt}");
                if theirs != raw_bytes(&want) {
                    let mut off = 0;
                    for (i, p) in want.planes.iter().enumerate() {
                        let n = p.len() * if bits == 8 { 1 } else { 2 };
                        let same = theirs[off..off + n] == raw_bytes(&Frame { planes: vec![p.clone()], ..want.clone() })[..];
                        eprintln!("{pix_fmt} s{slices} c{ctx}: plane {i} {}", if same { "equal" } else { "DIFFERENT" });
                        off += n;
                    }
                    panic!("{pix_fmt} s{slices} c{ctx}: our pixels differ from ffmpeg's");
                }
            }
        }
    }
}

/// A format of ffmpeg's FFV1 encoder: `(encoder input pix_fmt, pix_fmt to compare in, colourspace,
/// bits, chroma shift, planes)`. ffmpeg decodes `bgr0` as packed RGB; `gbrp` holds the same samples.
type Source = (&'static str, &'static str, Colorspace, u8, (u8, u8), usize);

const SOURCES: &[Source] = &[
    ("yuv420p", "yuv420p", Colorspace::YCbCr, 8, (1, 1), 3),
    ("yuv422p", "yuv422p", Colorspace::YCbCr, 8, (1, 0), 3),
    ("yuv444p", "yuv444p", Colorspace::YCbCr, 8, (0, 0), 3),
    ("gray", "gray", Colorspace::YCbCr, 8, (0, 0), 1),
    ("yuva420p", "yuva420p", Colorspace::YCbCr, 8, (1, 1), 4),
    ("bgr0", "gbrp", Colorspace::Rgb, 8, (0, 0), 3),
    ("yuv420p10le", "yuv420p10le", Colorspace::YCbCr, 10, (1, 1), 3),
    ("gbrp10le", "gbrp10le", Colorspace::Rgb, 10, (0, 0), 3),
];

/// testsrc2 (gradients, flat areas and edges, which give runs) with a noisy left half (which
/// gives large differences and Golomb-Rice escapes), moving from frame to frame.
const NOISY_TESTSRC: &str = "split[a][b];[b]noise=alls=60:allf=t,crop=iw/2:ih:0:0[n];[a][n]overlay=0:0";

/// Encode `frames` frames of `w` x `h` with ffmpeg's FFV1 encoder and the options `opts`.
fn ffmpeg_ffv1(ff: &Path, name: &str, source: &Source, w: u32, h: u32, frames: usize, opts: &[&str]) -> std::path::PathBuf {
    let path = craft_media_testkit::fixture_subdir("ffv1").join(format!("{name}.avi"));
    let lavfi = format!("testsrc2=size={w}x{h}:rate=25");
    let vf = format!("{NOISY_TESTSRC},format={}", source.0);
    let frames = frames.to_string();
    let mut args = vec!["-f", "lavfi", "-i", &lavfi, "-frames:v", &frames, "-vf", &vf, "-c:v", "ffv1"];
    args.extend_from_slice(opts);
    args.push(path.to_str().expect("path"));
    ffmpeg_run(ff, &args);
    path
}

/// An AVI's FFV1 stream: its configuration record (empty for versions 0 and 1), frame size, and
/// packets with their keyframe flags.
struct AviFfv1 {
    config: Vec<u8>,
    width: u32,
    height: u32,
    packets: Vec<(Vec<u8>, bool)>,
}

fn read_avi(path: &Path) -> AviFfv1 {
    let file = std::fs::read(path).expect("read");
    let avi = craft_media::avi::open(&file[..]).expect("open avi");
    let stream = &avi.streams[0];
    let video = stream.video.as_ref().expect("video");
    let packets = stream.chunks.iter().map(|c| (file[c.offset as usize..c.offset as usize + c.size as usize].to_vec(), c.key)).collect();
    AviFfv1 { config: video.extra.clone(), width: video.width as u32, height: video.height.unsigned_abs(), packets }
}

/// Decode every frame of an ffmpeg FFV1 file in order and check each against ffmpeg's own decode;
/// returns how many were non-keyframes.
fn decodes_like_ffmpeg(ff: &Path, path: &Path, source: &Source, label: &str) -> usize {
    let &(_, raw_fmt, cs, bits, shift, planes) = source;
    let avi = read_avi(path);
    let dec = Decoder::new(&avi.config, avi.width, avi.height).unwrap_or_else(|e| panic!("{label}: config: {e}"));
    let theirs = ffmpeg_run(ff, &["-i", path.to_str().expect("path"), "-f", "rawvideo", "-pix_fmt", raw_fmt, "-"]);
    let mut off = 0;
    let mut non_key = 0;
    for (i, (packet, key)) in avi.packets.iter().enumerate() {
        non_key += usize::from(!key);
        let got = dec.decode(packet).unwrap_or_else(|e| panic!("{label}: frame {i}: {e}"));
        assert_eq!((got.colorspace, got.bit_depth, got.chroma_shift, got.planes.len()), (cs, bits, shift, planes), "{label}: frame {i}");
        let mut want = got.clone();
        if cs == Colorspace::Rgb {
            want.planes = gbr_order(&got.planes);
        }
        let ours = raw_bytes(&want);
        assert!(theirs.get(off..off + ours.len()) == Some(&ours[..]), "{label}: frame {i} differs from ffmpeg's");
        off += ours.len();
    }
    assert_eq!(off, theirs.len(), "{label}: ffmpeg decoded a different number of frames");
    non_key
}

#[test]
fn ffmpegs_version_0_and_1_streams_decode_bit_exact() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    for level in ["0", "1"] {
        for coder in ["rice", "range_def", "range_tab"] {
            for source in SOURCES {
                // Version 0 has 8-bit samples only.
                if level == "0" && source.3 > 8 {
                    continue;
                }
                let name = format!("ffmpeg_v{level}_{coder}_{}", source.0);
                let opts = ["-level", level, "-coder", coder, "-g", "3"];
                let path = ffmpeg_ffv1(&ff, &name, source, 49, 25, 7, &opts);
                assert!(decodes_like_ffmpeg(&ff, &path, source, &name) > 0, "{name}: no non-keyframes");
            }
        }
    }
}

#[test]
fn ffmpegs_golomb_rice_version_3_streams_decode_bit_exact() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    // Slice boundaries must fall on chroma samples: 4 slices of 100 x 52 do, 49 x 25 has one slice.
    for (w, h, slices, crc) in [(49, 25, "1", "0"), (100, 52, "4", "1")] {
        for source in SOURCES {
            let name = format!("ffmpeg_v3_rice_{}_s{slices}_crc{crc}", source.0);
            let opts = ["-level", "3", "-coder", "rice", "-slices", slices, "-slicecrc", crc, "-g", "3"];
            let path = ffmpeg_ffv1(&ff, &name, source, w, h, 7, &opts);
            assert!(decodes_like_ffmpeg(&ff, &path, source, &name) > 0, "{name}: no non-keyframes");
        }
    }
}

#[test]
fn ffmpegs_range_coded_non_keyframes_decode_bit_exact() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    for coder in ["range_def", "range_tab"] {
        for source in SOURCES {
            let name = format!("ffmpeg_v3_{coder}_{}_gop", source.0);
            let opts = ["-level", "3", "-coder", coder, "-slices", "4", "-slicecrc", "1", "-g", "3"];
            let path = ffmpeg_ffv1(&ff, &name, source, 100, 52, 7, &opts);
            assert!(decodes_like_ffmpeg(&ff, &path, source, &name) > 0, "{name}: no non-keyframes");
        }
    }
}

/// Damaged frames of the new kinds never panic or hang. With slice CRCs (version 3) every cut and
/// every flipped bit is an error; Golomb-Rice data runs to the end of a version 0 or 1 frame, so
/// every cut is an error there too. A non-keyframe is damaged after its keyframe decoded, so the
/// damage meets carried-over states.
#[test]
fn damaged_golomb_rice_and_non_keyframes_are_errors_not_panics() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    // (name, ffmpeg options, every cut is an error, every bit flip is an error)
    let cases: &[(&str, &[&str], bool, bool)] = &[
        ("v3_rice_crc", &["-level", "3", "-coder", "rice", "-slices", "1", "-slicecrc", "1"], true, true),
        ("v3_range_crc", &["-level", "3", "-coder", "range_def", "-slices", "1", "-slicecrc", "1"], true, true),
        ("v3_rice", &["-level", "3", "-coder", "rice", "-slices", "1", "-slicecrc", "0"], false, false),
        ("v0_rice", &["-level", "0", "-coder", "rice"], true, false),
        ("v1_rice", &["-level", "1", "-coder", "rice"], true, false),
        ("v1_range", &["-level", "1", "-coder", "range_def"], false, false),
        ("v1_range_tab", &["-level", "1", "-coder", "range_tab"], false, false),
    ];
    for &(name, opts, cuts_fail, flips_fail) in cases {
        let name = format!("ffmpeg_hostile_{name}");
        let opts: Vec<&str> = opts.iter().copied().chain(["-g", "2"]).collect();
        let avi = read_avi(&ffmpeg_ffv1(&ff, &name, &SOURCES[0], 16, 8, 2, &opts));
        let (config, w, h) = (avi.config.clone(), avi.width, avi.height);
        let key = avi.packets[0].0.clone();
        let next = avi.packets[1].0.clone();
        assert!(!avi.packets[1].1, "{name}: the second frame is a keyframe");
        let dec = Decoder::new(&config, w, h).expect("decoder");
        for (frame, packet) in [&key, &next].into_iter().enumerate() {
            let cuts = (0..packet.len()).filter(|_| cuts_fail).map(|n| packet[..n].to_vec());
            let flips = (0..packet.len() * 8).filter(|_| flips_fail).map(|i| {
                let mut c = packet.clone();
                c[i / 8] ^= 1 << (i % 8);
                c
            });
            for (case, damaged) in cuts.chain(flips).enumerate() {
                if frame == 1 {
                    dec.decode(&key).expect("keyframe");
                }
                assert!(dec.decode(&damaged).is_err(), "{name}: damage {case} of frame {frame} went unnoticed");
            }
        }
        for (frame, seed) in [key.clone(), next].into_iter().enumerate() {
            let (config, key) = (config.clone(), key.clone());
            craft_media_testkit::hostile::hammer(&format!("{name} frame {frame}"), &seed, 2000, move |d| {
                let Ok(dec) = Decoder::new(&config, w, h) else { return };
                if frame == 1 {
                    let _ = dec.decode(&key);
                }
                let _ = dec.decode(d);
                // A second time: after a failure, or continuing the damaged frame.
                let _ = dec.decode(d);
            });
        }
    }
}

#[test]
fn track_reports_configured_alpha_before_decoding() {
    use craft_media::{CodecId, CodecParams};
    use craft_video::TrackDecoder;

    for (color, planes, alpha) in [
        (Colorspace::YCbCr, 1, false),
        (Colorspace::YCbCr, 2, true),
        (Colorspace::YCbCr, 3, false),
        (Colorspace::YCbCr, 4, true),
        (Colorspace::Rgb, 3, false),
        (Colorspace::Rgb, 4, true),
    ] {
        let source = frame(11, 9, color, 8, (0, 0), planes);
        let encoder = Encoder::new(&source, EncoderConfig::default()).expect("encoder");
        let params = CodecParams::new(CodecId::Ffv1).with_size(source.width, source.height).with_extradata(encoder.config_record());
        let mut track = TrackDecoder::new(&params).expect("track");
        assert_eq!(track.configured_alpha(), Some(alpha), "{color:?} with {planes} planes");
        track.reset().expect("track reset");
        assert_eq!(track.configured_alpha(), Some(alpha));

        let packet = encoder.encode(&source).expect("coded picture");
        let pictures = track.decode(&packet, 43).expect("decoded picture");
        assert_eq!(pictures.len(), 1);
        assert_eq!(pictures[0].alpha().is_some(), alpha);
    }
}

#[cfg(feature = "prores")]
#[test]
fn configured_alpha_is_unknown_for_prores_without_a_picture() {
    let params = craft_media::CodecParams::new(craft_media::CodecId::ProRes);
    let track = craft_video::TrackDecoder::new(&params).expect("track");
    assert_eq!(track.configured_alpha(), None);
}
