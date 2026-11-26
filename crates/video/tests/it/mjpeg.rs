//! Motion JPEG: the encoder and decoder against each other, against ffmpeg as the reference
//! (oracle; skipped when ffmpeg is absent, a failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`), and on
//! hostile packets.

use craft_video::mjpeg::{ChromaSubsampling, Decoder, Encoder, EncoderConfig, Error, Frame, PixelFormat, Samples, is_packet, picture_size};
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
    // the same input every run: ffmpeg's own test sources draw random colors
    let input = dir.join("input.rgb");
    std::fs::write(&input, picture(67, 41, 3)).expect("write");
    let input = s(&input);
    let source = ["-f", "rawvideo", "-pix_fmt", "rgb24", "-s", "67x41", "-i", input.as_str()];
    for (name, pix_fmt, rgb_pix, channels) in [("420", "yuvj420p", "rgb24", 3), ("422", "yuvj422p", "rgb24", 3), ("444", "yuvj444p", "rgb24", 3)] {
        let jpg = dir.join(format!("{name}.jpg"));
        ffmpeg(&ff, &[&source[..], &["-frames:v", "1", "-c:v", "mjpeg", "-pix_fmt", pix_fmt, "-q:v", "2", &s(&jpg)]].concat());
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
            &[&source[..], &["-frames:v", "1", "-c:v", "mjpeg", "-pix_fmt", pix_fmt, "-q:v", "2", "-huffman", "default", &s(&plain)]].concat(),
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

/// libjpeg-turbo's test images, kept with `craft-jpeg` (`crates/jpeg/tests/data/SOURCE.md`).
const TESTORIG: &[u8] = include_bytes!("../../../jpeg/tests/data/testorig.jpg");
const TESTORIG_PROGRESSIVE: &[u8] = include_bytes!("../../../jpeg/tests/data/testorig-huffman-progressive.jpg");
const TESTORIG12: &[u8] = include_bytes!("../../../jpeg/tests/data/testorig12.jpg");
/// libjpeg-turbo's decode of `testorig12.jpg`: binary PPM, maxval 4095.
const TESTORIG12_PPM: &[u8] = include_bytes!("../../../jpeg/tests/data/testorig12.ppm");

/// A byte the padding of a row starts as, to see that `decode_into` leaves it alone.
const PAD: u8 = 0xA5;

/// `f` as `format` pixels, rows `stride` bytes apart with [`PAD`] after each row.
fn expected_pixels(f: &Frame, format: PixelFormat, stride: usize) -> Vec<u8> {
    let (w, h, ch) = (f.width as usize, f.height as usize, usize::from(f.channels));
    let eight: Vec<u8> = match &f.samples {
        Samples::U8(v) => v.clone(),
        Samples::U16(v) => v.iter().map(|&s| ((u32::from(s) * 255 + 32_767) / 65_535) as u8).collect(),
    };
    let mut out = vec![PAD; stride * h];
    for y in 0..h {
        for x in 0..w {
            let s = &eight[(y * w + x) * ch..][..ch];
            let rgb = match ch {
                1 => [s[0]; 3],
                3 => [s[0], s[1], s[2]],
                _ => panic!("{ch} channels"),
            };
            let px = match format {
                PixelFormat::Rgba8 => [rgb[0], rgb[1], rgb[2], 255],
                PixelFormat::Bgra8 => [rgb[2], rgb[1], rgb[0], 255],
            };
            out[y * stride + x * 4..][..4].copy_from_slice(&px);
        }
    }
    out
}

/// `decode_into` of `packet` at a stride of `pad` bytes past each row.
fn decoded_into(packet: &[u8], format: PixelFormat, pad: usize) -> (craft_video::mjpeg::Picture, Vec<u8>, usize) {
    let (w, h) = picture_size(packet).expect("size");
    let stride = w as usize * 4 + pad;
    let mut out = vec![PAD; stride * h as usize];
    let p = Decoder::new().decode_into(packet, format, &mut out, stride).expect("decode_into");
    (p, out, stride)
}

#[test]
fn decode_into_writes_the_pixels_of_decode_at_any_stride() {
    let enc = |w: usize, h: usize, ch: usize, sub| {
        Encoder::new(EncoderConfig { quality: 85, subsampling: sub })
            .expect("encoder")
            .encode(w as u32, h as u32, ch as u8, &picture(w, h, ch))
            .expect("encode")
    };
    let packets = [
        ("420", enc(67, 41, 3, ChromaSubsampling::S420)),
        ("422", enc(67, 41, 3, ChromaSubsampling::S422)),
        ("444", enc(64, 48, 3, ChromaSubsampling::S444)),
        ("1x1", enc(1, 1, 3, ChromaSubsampling::S420)),
        ("gray", enc(33, 17, 1, ChromaSubsampling::S420)),
        ("adobe0", with_app14(&enc(16, 16, 3, ChromaSubsampling::S444), 0)),
        ("testorig", TESTORIG.to_vec()),
        ("progressive", TESTORIG_PROGRESSIVE.to_vec()),
        ("12-bit", TESTORIG12.to_vec()),
    ];
    for (name, packet) in &packets {
        let f = Decoder::new().decode(packet).expect("decode");
        for format in [PixelFormat::Rgba8, PixelFormat::Bgra8] {
            for pad in [0, 3, 64] {
                let (p, out, stride) = decoded_into(packet, format, pad);
                assert_eq!((p.width, p.height, p.damaged), (f.width, f.height, f.damaged), "{name}");
                assert!(out == expected_pixels(&f, format, stride), "{name} {format:?} pad {pad}: pixels differ from decode's");
            }
        }
    }
}

#[test]
fn decode_into_reads_past_the_first_scan_as_decode_does() {
    let packet = Encoder::default().encode(40, 24, 3, &picture(40, 24, 3)).expect("encode");
    let sos = packet.windows(2).position(|w| w == [0xFF, 0xDA]).expect("SOS");
    let eoi = packet.len() - 2;
    let mut sof = packet.clone();
    sof.truncate(eoi);
    // a second frame header, and a second copy of the scan, after the first scan
    let frame = packet.windows(2).position(|w| w == [0xFF, 0xC0]).expect("SOF0");
    let frame_len = usize::from(u16::from_be_bytes([packet[frame + 2], packet[frame + 3]]));
    sof.extend_from_slice(&packet[frame..frame + 2 + frame_len]);
    sof.extend_from_slice(&packet[eoi..]);
    let mut two_scans = packet[..eoi].to_vec();
    two_scans.extend_from_slice(&packet[sos..]);
    // a comment before EOI is read by decode too
    let mut comment = packet[..eoi].to_vec();
    comment.extend_from_slice(&[0xFF, 0xFE, 0, 4, b'h', b'i']);
    comment.extend_from_slice(&packet[eoi..]);
    for (name, p) in [("second SOF", sof), ("second SOS", two_scans), ("COM", comment)] {
        let whole = Decoder::new().decode(&p);
        let mut out = vec![PAD; 40 * 4 * 24];
        let into = Decoder::new().decode_into(&p, PixelFormat::Rgba8, &mut out, 40 * 4);
        match whole {
            Err(e) => assert_eq!(into.err(), Some(e), "{name}"),
            Ok(f) => {
                assert_eq!(into.map(|p| (p.width, p.height, p.damaged)), Ok((f.width, f.height, f.damaged)), "{name}");
                assert!(out == expected_pixels(&f, PixelFormat::Rgba8, 40 * 4), "{name}: pixels differ from decode's");
            }
        }
    }
}

#[test]
fn decode_into_rounds_12_bit_pictures_close_to_libjpeg_turbo() {
    // The PPM's header is "P6\n227 149\n4095\n" and its samples are big-endian 16-bit.
    let header = b"P6\n227 149\n4095\n";
    assert_eq!(&TESTORIG12_PPM[..header.len()], header);
    let reference: Vec<u8> =
        TESTORIG12_PPM[header.len()..].as_chunks::<2>().0.iter().map(|b| ((u32::from(u16::from_be_bytes(*b)) * 255 + 2047) / 4095) as u8).collect();
    let (p, out, stride) = decoded_into(TESTORIG12, PixelFormat::Rgba8, 0);
    assert_eq!((p.width, p.height, stride), (227, 149, 227 * 4));
    let rgb: Vec<u8> = out.as_chunks::<4>().0.iter().flat_map(|px| [px[0], px[1], px[2]]).collect();
    let worst = rgb.iter().zip(&reference).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
    assert!(worst <= 1, "{worst} levels from libjpeg-turbo's decode");
    assert!(out.as_chunks::<4>().0.iter().all(|px| px[3] == 255));
}

#[test]
fn decode_into_refuses_a_buffer_or_stride_too_small_without_writing() {
    let packet = Encoder::default().encode(20, 10, 3, &picture(20, 10, 3)).expect("encode");
    assert_eq!(picture_size(&packet).expect("size"), (20, 10));
    let dec = Decoder::new();
    let progressive = (TESTORIG_PROGRESSIVE, 227 * 4, 149);
    for (packet, row, rows) in [(&packet[..], 80, 10), progressive] {
        // one byte short of the last row, and a stride under a row
        for (len, stride) in [(row * rows - 1, row), (row * rows, row - 1), (0, row)] {
            let mut out = vec![PAD; len];
            assert!(matches!(dec.decode_into(packet, PixelFormat::Rgba8, &mut out, stride), Err(Error::Input(_))), "{len} {stride}");
            assert!(out.iter().all(|&b| b == PAD), "nothing is written");
        }
        // the last row needs no padding after it
        let mut out = vec![PAD; row * 2 * (rows - 1) + row];
        assert!(dec.decode_into(packet, PixelFormat::Rgba8, &mut out, row * 2).is_ok());
        // a stride the buffer size overflows
        assert!(matches!(dec.decode_into(packet, PixelFormat::Rgba8, &mut out, usize::MAX), Err(Error::Input(_))));
    }
    // A packet decoded whole is checked against its frame header before it is decoded: the
    // progressive one cut before its first scan has no picture to decode.
    let sos = TESTORIG_PROGRESSIVE.windows(2).position(|w| w == [0xFF, 0xDA]).expect("SOS");
    let mut out = vec![PAD; 64];
    assert!(matches!(dec.decode_into(&TESTORIG_PROGRESSIVE[..sos], PixelFormat::Rgba8, &mut out, 227 * 4), Err(Error::Input(_))));
    let mut out = vec![0; 64];
    assert!(matches!(dec.decode_into(b"not a jpeg", PixelFormat::Rgba8, &mut out, 4), Err(Error::Invalid(_))));
    let mut out = vec![0; 80 * 10];
    assert!(matches!(dec.clone().max_pixels(199).decode_into(&packet, PixelFormat::Rgba8, &mut out, 80), Err(Error::Limit(_))));
    assert!(matches!(dec.max_pixels(199).decode_into(TESTORIG_PROGRESSIVE, PixelFormat::Rgba8, &mut out, 80), Err(Error::Limit(_))));
}

#[test]
fn picture_size_reads_the_frame_header_only() {
    for packet in [TESTORIG, TESTORIG_PROGRESSIVE, TESTORIG12] {
        assert_eq!(picture_size(packet).expect("size"), (227, 149));
        // the headers alone are enough
        let sos = packet.windows(2).position(|w| w == [0xFF, 0xDA]).expect("SOS");
        assert_eq!(picture_size(&packet[..sos]).expect("size"), (227, 149));
    }
    let packet = Encoder::default().encode(20, 10, 1, &[9; 200]).expect("encode");
    let sof = packet.windows(2).position(|w| w == [0xFF, 0xC0]).expect("SOF0");
    let mut dnl = packet.clone();
    dnl[sof + 5..sof + 7].copy_from_slice(&[0, 0]);
    assert!(matches!(picture_size(&dnl), Err(Error::Unsupported(_))));
    let mut no_width = packet.clone();
    no_width[sof + 7..sof + 9].copy_from_slice(&[0, 0]);
    assert!(matches!(picture_size(&no_width), Err(Error::Invalid(_))));
    for n in 0..sof + 9 {
        assert!(picture_size(&packet[..n]).is_err(), "cut at {n}");
    }
    for junk in [&b"RIFF"[..], &[0xFF, 0xD8, 0xFF, 0xD9], &[0xFF, 0xD8, 0xFF, 0xDA, 0, 2], &[0xFF, 0xD8, 0xFF, 0xE0, 0, 1]] {
        assert!(picture_size(junk).is_err());
    }
}

#[test]
fn hostile_packets_decode_into_errors_never_panics() {
    let packet = Encoder::default().encode(48, 40, 3, &picture(48, 40, 3)).expect("encode");
    let dec = Decoder::new();
    let mut out = vec![0u8; 48 * 4 * 40];
    for n in 0..packet.len() {
        let _ = dec.decode_into(&packet[..n], PixelFormat::Bgra8, &mut out, 48 * 4);
    }
    let mut x = 0x9E37_79B9u32;
    for i in 0..packet.len() {
        let mut p = packet.clone();
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        p[i] ^= (x >> 8) as u8 | 1;
        // a flipped size byte can ask for more than the buffer holds: an error, not a panic
        let _ = dec.decode_into(&p, PixelFormat::Rgba8, &mut out, 48 * 4);
        let _ = picture_size(&p);
    }
}

#[test]
fn ffmpeg_decodes_to_the_rgba_and_bgra_of_decode_into() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("mjpeg");
    let s = |p: &Path| p.to_str().expect("path").to_string();
    for (name, sub) in [("into420", ChromaSubsampling::S420), ("into422", ChromaSubsampling::S422)] {
        let packet =
            Encoder::new(EncoderConfig { quality: 90, subsampling: sub }).expect("encoder").encode(67, 41, 3, &picture(67, 41, 3)).expect("encode");
        let jpg = dir.join(format!("{name}.jpg"));
        std::fs::write(&jpg, &packet).expect("write");
        for (format, pix_fmt) in [(PixelFormat::Rgba8, "rgba"), (PixelFormat::Bgra8, "bgra")] {
            let raw = dir.join(format!("{name}_{pix_fmt}.raw"));
            ffmpeg(&ff, &["-i", &s(&jpg), "-f", "rawvideo", "-pix_fmt", pix_fmt, &s(&raw)]);
            let theirs = std::fs::read(&raw).expect("read");
            let (_, ours, _) = decoded_into(&packet, format, 0);
            let p = psnr(&theirs, &ours);
            println!("{name} {pix_fmt}: ffmpeg against decode_into {p:.1} dB");
            assert!(p >= 38.0, "{name} {pix_fmt}: {p:.1} dB");
        }
    }
}

#[test]
fn track_dispatch_preserves_mjpeg_frames_and_timestamps() {
    use craft_media::{CodecId, CodecParams};
    use craft_video::{TrackDecoder, VideoError, frame::VideoFrame};

    let enc = Encoder::default();
    let direct = Decoder::new();
    let mut track = TrackDecoder::new(&CodecParams::new(CodecId::Mjpeg)).expect("Motion JPEG track");
    let color = enc.encode(17, 11, 3, &picture(17, 11, 3)).expect("RGB packet");
    let gray = enc.encode(13, 9, 1, &picture(13, 9, 1)).expect("gray packet");
    let mut want_color = VideoFrame::try_from(direct.decode(&color).expect("RGB picture")).expect("RGB frame");
    want_color.pts = Some(71);
    assert_eq!(track.decode(&color, 71).expect("RGB track picture"), [want_color]);
    assert!(matches!(track.decode(&[], 72), Err(VideoError::Malformed(_))));
    assert!(track.flush().expect("flush").is_empty());

    track.reset().expect("seek to an earlier packet");
    let mut want_gray = VideoFrame::try_from(direct.decode(&gray).expect("gray picture")).expect("gray frame");
    want_gray.pts = Some(-3);
    assert_eq!(track.decode(&gray, -3).expect("gray track picture"), [want_gray]);
    assert!(track.flush().expect("flush").is_empty());
}

#[test]
fn track_mjpeg_allocation_cap_survives_reset_and_larger_requests() {
    use craft_media::{CodecId, CodecParams};
    use craft_video::{TrackDecoder, VideoError};

    let encoder = Encoder::default();
    let small = encoder.encode(16, 16, 1, &[31; 16 * 16]).expect("small JPEG");
    let large = encoder.encode(256, 256, 1, &[31; 256 * 256]).expect("large JPEG");
    let mut track = TrackDecoder::new(&CodecParams::new(CodecId::Mjpeg)).expect("Motion JPEG track");
    assert_eq!(track.decode(&large, 0).expect("default cap admits the picture")[0].width(), 256);

    for cap in [16 << 10, u64::MAX] {
        track.limit_mjpeg_allocation(cap).expect("Motion JPEG allocation cap");
        track.reset().expect("seek");
        assert_eq!(track.decode(&small, 11).expect("small picture fits")[0].width(), 16);
        assert!(
            matches!(track.decode(&large, 12), Err(VideoError::Malformed(ref message)) if message.contains("above the limits")),
            "the larger picture exceeds the stricter cap"
        );
    }
}

#[cfg(feature = "prores")]
#[test]
fn the_track_mjpeg_allocation_cap_is_codec_specific() {
    use craft_media::{CodecId, CodecParams};
    use craft_video::{TrackDecoder, VideoError};

    let mut track = TrackDecoder::new(&CodecParams::new(CodecId::ProRes)).expect("ProRes track");
    assert!(matches!(track.limit_mjpeg_allocation(16 << 10), Err(VideoError::Unsupported(_))));
}
