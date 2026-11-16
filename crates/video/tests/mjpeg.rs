//! Motion JPEG: the encoder and decoder against each other, against ffmpeg as the reference
//! (oracle; skipped when ffmpeg is absent, a failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`), and on
//! hostile packets.

use craft_video::mjpeg::{ChromaSubsampling, Decoder, Encoder, EncoderConfig, Frame, Samples, is_packet};
use std::path::Path;
use std::process::Command;

/// A smooth picture with a diagonal edge, `channels` samples per pixel.
fn picture(w: usize, h: usize, channels: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(w * h * channels);
    for y in 0..h {
        for x in 0..w {
            let edge = if x + y > (w + h) / 2 { 40.0 } else { 0.0 };
            for c in 0..channels {
                let base = 90.0 + 60.0 * ((x as f32 * 0.11 + c as f32).sin() + (y as f32 * 0.07 - c as f32).cos());
                v.push((base + edge + 20.0 * c as f32).clamp(0.0, 255.0) as u8);
            }
        }
    }
    v
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    let mse = a.iter().zip(b).map(|(x, y)| (f64::from(*x) - f64::from(*y)).powi(2)).sum::<f64>() / a.len().max(1) as f64;
    if mse == 0.0 { 99.0 } else { 10.0 * (255.0f64 * 255.0 / mse).log10() }
}

fn bytes(f: &Frame) -> &[u8] {
    match &f.samples {
        Samples::U8(v) => v,
        _ => panic!("8-bit expected"),
    }
}

#[test]
fn encoded_frames_decode_back_close_to_the_input() {
    let dec = Decoder::new();
    for (w, h, ch, sub, min) in [
        (64, 48, 3, ChromaSubsampling::S444, 38.0),
        (67, 41, 3, ChromaSubsampling::S422, 33.0),
        (67, 41, 3, ChromaSubsampling::S420, 32.0),
        (1, 1, 3, ChromaSubsampling::S420, 25.0),
        (33, 17, 1, ChromaSubsampling::S420, 38.0),
    ] {
        let pixels = picture(w, h, ch);
        let enc = Encoder::new(EncoderConfig { quality: 92, subsampling: sub }).expect("encoder");
        let packet = enc.encode(w as u32, h as u32, ch as u8, &pixels).expect("encode");
        assert!(is_packet(&packet));
        assert_eq!(&packet[packet.len() - 2..], &[0xFF, 0xD9], "a packet runs from SOI to EOI");
        let f = dec.decode(&packet).expect("decode");
        assert_eq!((f.width as usize, f.height as usize, f.channels as usize, f.damaged), (w, h, ch, false));
        let p = psnr(&pixels, bytes(&f));
        assert!(p >= min, "{w}x{h} {ch}ch {sub:?}: {p:.1} dB under {min}");
    }
}

#[test]
fn rgba_drops_alpha_and_bad_input_is_refused() {
    let enc = Encoder::default();
    let rgba: Vec<u8> = (0..16 * 16 * 4).map(|i| (i * 7) as u8).collect();
    let f = Decoder::new().decode(&enc.encode(16, 16, 4, &rgba).expect("encode")).expect("decode");
    assert_eq!(f.channels, 3);
    assert!(enc.encode(16, 16, 3, &[0; 10]).is_err());
    assert!(enc.encode(0, 16, 3, &[]).is_err());
    assert!(enc.encode(16, 16, 2, &rgba).is_err());
    assert!(enc.encode(70_000, 1, 1, &[0; 70_000]).is_err());
}

#[test]
fn the_pixel_limit_is_applied_before_allocating() {
    let enc = Encoder::default();
    let packet = enc.encode(64, 64, 1, &[7; 64 * 64]).expect("encode");
    assert!(Decoder::new().max_pixels(64 * 64).decode(&packet).is_ok());
    assert!(matches!(Decoder::new().max_pixels(64 * 64 - 1).decode(&packet), Err(craft_video::mjpeg::Error::Limit(_))));
    // a header that claims 65535 x 65535 pixels of a tiny file
    let mut huge = packet.clone();
    let sof = huge.windows(2).position(|w| w == [0xFF, 0xC0]).expect("SOF0");
    huge[sof + 5..sof + 9].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
    assert!(Decoder::new().max_pixels(1 << 20).decode(&huge).is_err());
}

#[test]
fn hostile_packets_are_errors_never_panics() {
    let enc = Encoder::default();
    let packet = enc.encode(48, 40, 3, &picture(48, 40, 3)).expect("encode");
    let dec = Decoder::new();
    // every truncation
    for n in 0..packet.len() {
        let _ = dec.decode(&packet[..n]);
    }
    // single byte flips, and a run of noise after the headers
    let mut x = 0x2545_F491u32;
    for i in 0..packet.len() {
        let mut p = packet.clone();
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        p[i] ^= (x >> 8) as u8 | 1;
        let _ = dec.decode(&p);
    }
    let mut noisy = packet.clone();
    let from = noisy.len() / 2;
    for b in &mut noisy[from..] {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *b = (x >> 8) as u8;
    }
    let _ = dec.decode(&noisy);
    for junk in [&[][..], &[0xFF, 0xD8], &[0xFF, 0xD8, 0xFF, 0xD9], &[0xFF, 0xD8, 0xFF, 0xC3, 0, 2], b"\xFF\xD8\xFF\xDA\x00\x02"] {
        assert!(dec.decode(junk).is_err());
    }
}

#[test]
fn packets_that_are_not_baseline_or_progressive_huffman_are_unsupported() {
    // SOF3 (lossless): a frame header, no more
    let sof3 = [0xFF, 0xD8, 0xFF, 0xC3, 0, 11, 8, 0, 8, 0, 8, 1, 1, 0x11, 0, 0xFF, 0xD9];
    assert!(matches!(Decoder::new().decode(&sof3), Err(craft_video::mjpeg::Error::Unsupported(_))), "{:?}", Decoder::new().decode(&sof3));
}

fn ffmpeg(ff: &Path, args: &[&str]) {
    let out = Command::new(ff).args(["-v", "error", "-nostdin", "-y"]).args(args).output().expect("run ffmpeg");
    assert!(out.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// The SOS-preceding DHT segments cut out of a JPEG, as AVI `MJPG` packets often come.
fn without_huffman_tables(jpeg: &[u8]) -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8];
    let mut at = 2;
    while at + 3 < jpeg.len() && jpeg[at] == 0xFF {
        let marker = jpeg[at + 1];
        let len = usize::from(u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]));
        if marker == 0xDA {
            break;
        }
        if marker != 0xC4 {
            out.extend_from_slice(&jpeg[at..at + 2 + len]);
        }
        at += 2 + len;
    }
    out.extend_from_slice(&jpeg[at..]);
    out
}

#[test]
fn we_decode_what_ffmpeg_encodes_to_what_ffmpeg_decodes() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("mjpeg");
    let s = |p: &Path| p.to_str().expect("path").to_string();
    for (name, pix_fmt, rgb_pix, channels) in [("420", "yuvj420p", "rgb24", 3), ("422", "yuvj422p", "rgb24", 3), ("444", "yuvj444p", "rgb24", 3)] {
        let jpg = dir.join(format!("{name}.jpg"));
        ffmpeg(
            &ff,
            &["-f", "lavfi", "-i", "gradients=size=67x41:rate=1", "-frames:v", "1", "-c:v", "mjpeg", "-pix_fmt", pix_fmt, "-q:v", "2", &s(&jpg)],
        );
        let reference = dir.join(format!("{name}.raw"));
        ffmpeg(&ff, &["-i", &s(&jpg), "-f", "rawvideo", "-pix_fmt", rgb_pix, &s(&reference)]);
        let packet = std::fs::read(&jpg).expect("read");
        let f = Decoder::new().decode(&packet).expect("decode");
        assert_eq!((f.width, f.height, f.channels as usize), (67, 41, channels), "{name}");
        let p = psnr(&std::fs::read(&reference).expect("read"), bytes(&f));
        println!("{name}: {p:.1} dB against ffmpeg's decode");
        assert!(p >= 38.0, "{name}: {p:.1} dB");

        // the same stream with the Huffman tables left out, as AVI MJPG often has it, if ffmpeg
        // wrote the Annex K ones
        let plain = dir.join(format!("{name}_default_tables.jpg"));
        ffmpeg(
            &ff,
            &[
                "-f",
                "lavfi",
                "-i",
                "gradients=size=67x41:rate=1",
                "-frames:v",
                "1",
                "-c:v",
                "mjpeg",
                "-pix_fmt",
                pix_fmt,
                "-q:v",
                "2",
                "-huffman",
                "default",
                &s(&plain),
            ],
        );
        let stripped = without_huffman_tables(&std::fs::read(&plain).expect("read"));
        let with_tables = Decoder::new().decode(&std::fs::read(&plain).expect("read")).expect("decode");
        let without = Decoder::new().decode(&stripped).expect("decode without DHT");
        assert_eq!(with_tables, without, "{name}: the default tables are put in");
    }
}

#[test]
fn ffmpeg_decodes_what_we_encode() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("mjpeg");
    let s = |p: &Path| p.to_str().expect("path").to_string();
    for (name, w, h, ch, sub, pix_fmt) in [
        ("o420", 67usize, 41usize, 3usize, ChromaSubsampling::S420, "rgb24"),
        ("o422", 67, 41, 3, ChromaSubsampling::S422, "rgb24"),
        ("o444", 64, 48, 3, ChromaSubsampling::S444, "rgb24"),
        ("ogray", 33, 17, 1, ChromaSubsampling::S420, "gray"),
    ] {
        let pixels = picture(w, h, ch);
        let packet = Encoder::new(EncoderConfig { quality: 90, subsampling: sub })
            .expect("encoder")
            .encode(w as u32, h as u32, ch as u8, &pixels)
            .expect("encode");
        let jpg = dir.join(format!("{name}.jpg"));
        std::fs::write(&jpg, &packet).expect("write");
        let out = dir.join(format!("{name}.raw"));
        ffmpeg(&ff, &["-i", &s(&jpg), "-f", "rawvideo", "-pix_fmt", pix_fmt, &s(&out)]);
        let theirs = std::fs::read(&out).expect("read");
        let ours = Decoder::new().decode(&packet).expect("decode");
        let p = psnr(&theirs, bytes(&ours));
        let q = psnr(&pixels, &theirs);
        println!("{name}: ffmpeg decode against ours {p:.1} dB, against the input {q:.1} dB");
        assert!(p >= 38.0, "{name}: ffmpeg's decode of our packet differs from ours: {p:.1} dB");
        assert!(q >= 28.0, "{name}: ffmpeg's decode is far from the input: {q:.1} dB");
    }
}

/// `packet` with an Adobe APP14 segment carrying `transform`, right after SOI.
fn with_app14(packet: &[u8], transform: u8) -> Vec<u8> {
    let mut out = packet[..2].to_vec();
    out.extend_from_slice(&[0xFF, 0xEE, 0, 14]);
    out.extend_from_slice(b"Adobe");
    out.extend_from_slice(&[0, 100, 0, 0, 0, 0, transform]);
    out.extend_from_slice(&packet[2..]);
    out
}

#[test]
fn adobe_transform_zero_keeps_numeric_id_components_as_rgb() {
    // The encoder stores YCbCr under component ids 1, 2, 3. An Adobe APP14 transform of 0 says the
    // three components are R, G and B as stored, so the decoder must not convert them again.
    let (r, g, b) = (200.0f32, 60.0f32, 30.0f32);
    let pixels: Vec<u8> = [r as u8, g as u8, b as u8].repeat(16 * 16);
    let enc = Encoder::new(EncoderConfig { quality: 100, subsampling: ChromaSubsampling::S444 }).unwrap();
    let packet = enc.encode(16, 16, 3, &pixels).unwrap();
    let dec = Decoder::new();
    let ycc = [0.299 * r + 0.587 * g + 0.114 * b, 128.0 - 0.168_736 * r - 0.331_264 * g + 0.5 * b, 128.0 + 0.5 * r - 0.418_688 * g - 0.081_312 * b];
    let as_rgb = dec.decode(&with_app14(&packet, 0)).unwrap();
    assert_eq!(as_rgb.channels, 3);
    for px in bytes(&as_rgb).as_chunks::<3>().0 {
        for (got, want) in px.iter().zip(ycc) {
            assert!((f32::from(*got) - want).abs() <= 3.0, "{px:?} against stored {ycc:?}");
        }
    }
    // Transform 1 (YCbCr), and no APP14 at all, convert back to the picture's colour.
    for packet in [with_app14(&packet, 1), packet] {
        let f = dec.decode(&packet).unwrap();
        for px in bytes(&f).as_chunks::<3>().0 {
            for (got, want) in px.iter().zip([r, g, b]) {
                assert!((f32::from(*got) - want).abs() <= 3.0, "{px:?}");
            }
        }
    }
}
