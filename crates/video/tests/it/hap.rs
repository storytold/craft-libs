//! Hap tests: round trips, chunking, hostile input, and ffmpeg's decoder as the reference (oracle;
//! skipped when ffmpeg is absent, a failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`). ffmpeg has no
//! Hap encoder here, so the oracle is one-way: it decodes the frames our encoder writes, in an AVI
//! file, to the pictures our decoder returns.

use craft_media::avi::{AviWriter, MuxOptions, StreamSpec, VideoSpec};
use craft_video::hap::{Decoder, Encoder, Error, Image, TextureFormat, Variant};
use std::path::Path;
use std::process::Command;

fn xorshift(seed: &mut u32) -> u32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    *seed
}

/// A picture of smooth gradients, hard edges, noise, flat areas and a soft alpha ramp.
fn picture(w: u32, h: u32, salt: u32) -> Vec<u8> {
    let mut seed = 0x9E37_79B9u32 ^ salt;
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = match (y / 13 + x / 29) % 4 {
                0 => ((x * 3 + salt) as u8, (y * 4) as u8, ((x + y) * 2) as u8),
                1 => (xorshift(&mut seed) as u8, xorshift(&mut seed) as u8, xorshift(&mut seed) as u8),
                2 => (200, 40, 90),
                _ => (if (x / 3) % 2 == 0 { 255 } else { 0 }, 128, if (y / 3) % 2 == 0 { 255 } else { 0 }),
            };
            out.extend_from_slice(&[r, g, b, (x * 255 / w.max(1)) as u8]);
        }
    }
    out
}

/// Gradients, edges and flat areas, with no noise, the kind of picture block compression suits.
fn smooth(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = match (y / 16 + x / 24) % 3 {
                0 => ((x * 255 / w.max(1)) as u8, (y * 255 / h.max(1)) as u8, 128),
                1 => (230, 60, 20),
                _ => (if (x / 8) % 2 == 0 { 250 } else { 10 }, 120, 40),
            };
            out.extend_from_slice(&[r, g, b, (x * 255 / w.max(1)) as u8]);
        }
    }
    out
}

fn psnr(a: &[u8], b: &[u8], channels: usize) -> f64 {
    let (mut err, mut n) = (0f64, 0f64);
    for (p, q) in a.chunks(4).zip(b.chunks(4)) {
        for c in 0..channels {
            err += (f64::from(p[c]) - f64::from(q[c])).powi(2);
            n += 1.0;
        }
    }
    if err == 0.0 { 99.0 } else { 10.0 * (255.0 * 255.0 * n / err).log10() }
}

const VARIANTS: [Variant; 5] = [Variant::Hap, Variant::HapAlpha, Variant::HapQ, Variant::HapQAlpha, Variant::HapAlphaOnly];

fn channels(v: Variant) -> usize {
    match v {
        Variant::HapAlpha | Variant::HapQAlpha => 4,
        _ => 3,
    }
}

#[test]
fn every_variant_round_trips_at_good_quality() {
    for (w, h) in [(64u32, 48u32), (130, 70), (4, 4), (1, 1), (17, 5)] {
        let src = smooth(w, h);
        for v in VARIANTS {
            for (snappy, chunks) in [(false, 1usize), (true, 1), (false, 3), (true, 4)] {
                let enc = Encoder::new(w, h, v).expect("encoder").with_snappy(snappy).with_chunks(chunks);
                let frame = enc.encode_rgba(&src).expect("encode");
                let img = Decoder::new(w, h).expect("decoder").decode_rgba(&frame).expect("decode");
                assert_eq!((img.width, img.height, img.rgba.len()), (w, h, (w * h * 4) as usize));
                if v == Variant::HapAlphaOnly {
                    let (a, b): (Vec<u8>, Vec<u8>) = (src.chunks(4).map(|p| p[3]).collect(), img.rgba.chunks(4).map(|p| p[3]).collect());
                    let worst = a.iter().zip(&b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
                    assert!(worst <= 16, "{v:?} {w}x{h}: alpha off by {worst}");
                } else if w >= 16 && h >= 16 {
                    let q = psnr(&src, &img.rgba, channels(v).min(3));
                    assert!(q > 30.0, "{v:?} {w}x{h} snappy={snappy} chunks={chunks}: {q:.1} dB");
                }
            }
        }
    }
}

#[test]
fn snappy_and_chunks_do_not_change_the_pixels() {
    let (w, h) = (96, 64);
    let src = picture(w, h, 3);
    for v in VARIANTS {
        let want = Decoder::new(w, h).expect("d").decode_rgba(&Encoder::new(w, h, v).expect("e").encode_rgba(&src).expect("enc")).expect("dec");
        for (snappy, chunks) in [(true, 1usize), (false, 5), (true, 7), (true, 600)] {
            let frame = Encoder::new(w, h, v).expect("e").with_snappy(snappy).with_chunks(chunks).encode_rgba(&src).expect("enc");
            let got = Decoder::new(w, h).expect("d").decode_rgba(&frame).expect("dec");
            assert_eq!(got, want, "{v:?} snappy={snappy} chunks={chunks}");
        }
    }
}

#[test]
fn flat_colours_are_exact_in_rgb_variants() {
    let (w, h) = (32, 32);
    for v in [Variant::Hap, Variant::HapAlpha, Variant::HapQ, Variant::HapQAlpha] {
        for rgb in [[0u8, 0, 0], [255, 255, 255], [255, 0, 0], [0, 255, 0], [0, 0, 255], [8, 16, 24]] {
            let src: Vec<u8> = (0..w * h).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect();
            let frame = Encoder::new(w, h, v).expect("e").encode_rgba(&src).expect("enc");
            let img = Decoder::new(w, h).expect("d").decode_rgba(&frame).expect("dec");
            let worst = src.iter().zip(&img.rgba).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
            // 565 endpoints and the YCoCg round trip put a flat colour within a few levels.
            assert!(worst <= 8, "{v:?} {rgb:?}: off by {worst}");
        }
    }
}

#[test]
fn textures_come_back_as_stored() {
    let src = picture(64, 32, 1);
    let frame = Encoder::new(64, 32, Variant::HapQAlpha).expect("e").with_snappy(true).encode_rgba(&src).expect("enc");
    let t = Decoder::new(64, 32).expect("d").decode_textures(&frame).expect("textures");
    assert_eq!(t.len(), 2);
    assert_eq!((t[0].format, t[0].data.len()), (TextureFormat::YcocgDxt5, 16 * 8 * 16));
    assert_eq!((t[1].format, t[1].data.len()), (TextureFormat::Bc4Alpha, 16 * 8 * 8));
    assert_eq!(Variant::HapQAlpha.fourcc(), *b"HapM");
}

#[test]
fn offsets_table_and_unknown_sections_are_honoured() {
    // A chunked frame built by hand: two chunks, stored out of order through an offset table, and
    // a sub-section of an unknown type that must be skipped.
    let tex: Vec<u8> = (0..16 * 8).map(|i| i as u8).collect(); // 16 DXT1 blocks = 16x16 pixels
    let sect = |kind: u8, p: &[u8]| {
        let mut v = vec![p.len() as u8, (p.len() >> 8) as u8, (p.len() >> 16) as u8, kind];
        v.extend_from_slice(p);
        v
    };
    let (first, second) = tex.split_at(64);
    let data = [second, first].concat(); // second half stored first
    let mut ins = sect(0x02, &[0x0A, 0x0A]);
    ins.extend(sect(0x7F, b"junk"));
    ins.extend(sect(0x03, &[[64u32, 64].map(u32::to_le_bytes)].concat().concat()));
    ins.extend(sect(0x04, &[[64u32, 0].map(u32::to_le_bytes)].concat().concat()));
    let mut payload = sect(0x01, &ins);
    payload.extend_from_slice(&data);
    let frame = sect(0xCB, &payload);
    let t = Decoder::new(16, 16).expect("d").decode_textures(&frame).expect("textures");
    assert_eq!(t[0].data, tex);
}

#[test]
fn big_sections_use_the_long_header() {
    // 1024 x 1024 DXT5 is 1 MiB; at 4096 x 4096 a section tops 16 MiB and needs the 8-byte header.
    let (w, h) = (4096u32, 4096u32);
    let src = vec![77u8; (w * h * 4) as usize];
    let frame = Encoder::new(w, h, Variant::HapAlpha).expect("e").encode_rgba(&src).expect("enc");
    assert_eq!(&frame[..3], &[0, 0, 0]);
    assert_eq!(frame[3], 0xAE);
    let t = Decoder::new(w, h).expect("d").decode_textures(&frame).expect("textures");
    assert_eq!(t[0].data.len(), 1024 * 1024 * 16);
}

#[test]
fn bad_parameters_are_errors() {
    assert!(matches!(Decoder::new(0, 10), Err(Error::Input(_))));
    assert!(matches!(Decoder::new(40_000, 10), Err(Error::Input(_))));
    assert!(matches!(Encoder::new(10, 0, Variant::Hap), Err(Error::Input(_))));
    assert!(matches!(Encoder::new(32768, 32768, Variant::Hap), Err(Error::Input(_))));
    let e = Encoder::new(8, 8, Variant::Hap).expect("e");
    assert!(matches!(e.encode_rgba(&[0; 10]), Err(Error::Input(_))));
}

#[test]
fn hostile_frames_are_errors_not_panics() {
    let (w, h) = (32, 32);
    let d = Decoder::new(w, h).expect("d");
    let src = picture(w, h, 9);
    let good = Encoder::new(w, h, Variant::HapQAlpha).expect("e").with_snappy(true).with_chunks(3).encode_rgba(&src).expect("enc");
    assert!(d.decode_rgba(&good).is_ok());
    // Every truncation.
    for n in 0..good.len() {
        assert!(d.decode_rgba(&good[..n]).is_err(), "truncated to {n}");
    }
    // Every single-byte corruption must be handled, with an error or a picture.
    let mut seed = 3u32;
    for _ in 0..3000 {
        let mut bad = good.clone();
        let i = xorshift(&mut seed) as usize % bad.len();
        bad[i] = xorshift(&mut seed) as u8;
        let _ = d.decode_rgba(&bad);
    }
    // Random bytes with plausible headers.
    for kind in [0xAB, 0xBB, 0xCB, 0xAE, 0xBF, 0xCF, 0xA1, 0xB1, 0xC1, 0x0D, 0xAC, 0xA2, 0x00, 0xFF] {
        for len in [0usize, 1, 3, 4, 8, 100] {
            let mut f = vec![len as u8, 0, 0, kind];
            f.extend((0..len).map(|_| xorshift(&mut seed) as u8));
            let _ = d.decode_textures(&f);
            let _ = d.decode_rgba(&f);
        }
    }
    // Sizes that overflow or lie.
    assert!(d.decode_rgba(&[0, 0, 0, 0xAB, 0xFF, 0xFF, 0xFF, 0xFF]).is_err());
    assert!(d.decode_rgba(&[0xFF, 0xFF, 0xFF, 0xAB]).is_err());
    assert!(d.decode_rgba(&[]).is_err());
    // BC7 frames are read but not turned into pictures.
    let mut bc7 = vec![0, 4, 0, 0xAC];
    bc7.extend(vec![0u8; 1024]);
    assert!(d.decode_textures(&bc7).is_ok());
    assert!(matches!(d.decode_rgba(&bc7), Err(Error::Unsupported(_))));
}

fn ffmpeg_pictures(ffmpeg: &Path, avi: &Path, pix_fmt: &str) -> Vec<u8> {
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-v", "error", "-i"])
        .arg(avi)
        .args(["-f", "rawvideo", "-pix_fmt", pix_fmt, "-"])
        .output()
        .expect("run ffmpeg");
    assert!(out.status.success(), "ffmpeg failed on {}: {}", avi.display(), String::from_utf8_lossy(&out.stderr));
    out.stdout
}

fn avi_of(fourcc: [u8; 4], w: u32, h: u32, frames: &[Vec<u8>]) -> Vec<u8> {
    let spec = VideoSpec::new(fourcc, w, h, 25, 1);
    let mut mux = AviWriter::new(std::io::Cursor::new(Vec::new()), vec![StreamSpec::Video(spec)], MuxOptions::default()).expect("writer");
    for f in frames {
        mux.write_chunk(0, f, true).expect("chunk");
    }
    mux.finish().expect("finish").into_inner()
}

#[test]
fn ffmpeg_decodes_our_frames_to_our_pictures() {
    let Some(ffmpeg) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("hap");
    let mut worst_overall = 0u8;
    for (w, h) in [(64u32, 48u32), (130, 70), (36, 20)] {
        for v in VARIANTS {
            for (snappy, chunks) in [(false, 1usize), (true, 1), (true, 4), (false, 3)] {
                let enc = Encoder::new(w, h, v).expect("e").with_snappy(snappy).with_chunks(chunks);
                let dec = Decoder::new(w, h).expect("d");
                let pics: Vec<Image> = (0..3).map(|i| dec.decode_rgba(&enc.encode_rgba(&picture(w, h, i)).expect("enc")).expect("dec")).collect();
                let frames: Vec<Vec<u8>> = (0..3).map(|i| enc.encode_rgba(&picture(w, h, i)).expect("enc")).collect();
                let name = format!("{}_{w}x{h}_{}_{chunks}.avi", String::from_utf8_lossy(&v.fourcc()), u8::from(snappy));
                let file = dir.join(&name);
                std::fs::write(&file, avi_of(v.fourcc(), w, h, &frames)).expect("write fixture");
                let theirs = ffmpeg_pictures(&ffmpeg, &file, "rgba");
                let n = (w * h * 4) as usize;
                assert_eq!(theirs.len(), 3 * n, "{name}: ffmpeg returned {} bytes", theirs.len());
                for (i, pic) in pics.iter().enumerate() {
                    let t = &theirs[i * n..(i + 1) * n];
                    let worst = if v == Variant::HapAlphaOnly {
                        // ffmpeg decodes the alpha-only format to a grey picture.
                        pic.rgba.chunks(4).zip(t.chunks(4)).map(|(a, b)| a[3].abs_diff(b[0])).max().unwrap_or(0)
                    } else {
                        // Hap1 and HapY have no alpha: compare colour only.
                        let c = channels(v);
                        pic.rgba.chunks(4).zip(t.chunks(4)).flat_map(|(a, b)| (0..c).map(move |k| a[k].abs_diff(b[k]))).max().unwrap_or(0)
                    };
                    worst_overall = worst_overall.max(worst);
                    assert!(worst == 0, "{name} frame {i}: ffmpeg differs from us by up to {worst}");
                }
            }
        }
    }
    eprintln!("worst difference from ffmpeg: {worst_overall}");
}

/// Arbitrary texture bytes, not only what our encoder writes, decode to what ffmpeg decodes: this
/// covers every endpoint order, palette index and YCoCg scale a block can hold.
#[test]
fn ffmpeg_decodes_arbitrary_blocks_to_our_pictures() {
    let Some(ffmpeg) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("hap");
    let mut seed = 0xDEAD_BEEFu32;
    let (w, h) = (64u32, 64u32);
    let blocks = (w / 4 * (h / 4)) as usize;
    for (v, kind, block_bytes) in
        [(Variant::Hap, 0xABu8, 8usize), (Variant::HapAlpha, 0xAE, 16), (Variant::HapQ, 0xAF, 16), (Variant::HapAlphaOnly, 0xA1, 8)]
    {
        let dec = Decoder::new(w, h).expect("d");
        let frames: Vec<Vec<u8>> = (0..20)
            .map(|_| {
                let n = blocks * block_bytes;
                let mut f = vec![n as u8, (n >> 8) as u8, (n >> 16) as u8, kind];
                f.extend((0..n).map(|_| xorshift(&mut seed) as u8));
                f
            })
            .collect();
        let file = dir.join(format!("random_{}.avi", String::from_utf8_lossy(&v.fourcc())));
        std::fs::write(&file, avi_of(v.fourcc(), w, h, &frames)).expect("write fixture");
        let theirs = ffmpeg_pictures(&ffmpeg, &file, "rgba");
        let n = (w * h * 4) as usize;
        assert_eq!(theirs.len(), frames.len() * n);
        for (i, f) in frames.iter().enumerate() {
            let ours = dec.decode_rgba(f).expect("decode");
            let t = &theirs[i * n..(i + 1) * n];
            let same = if v == Variant::HapAlphaOnly {
                ours.rgba.chunks(4).zip(t.chunks(4)).all(|(a, b)| a[3] == b[0])
            } else if v == Variant::HapAlpha {
                ours.rgba == t
            } else {
                ours.rgba.chunks(4).zip(t.chunks(4)).all(|(a, b)| a[..3] == b[..3])
            };
            assert!(same, "{v:?} random frame {i} differs from ffmpeg");
        }
    }
}
