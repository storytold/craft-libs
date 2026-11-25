//! Black-box ffmpeg oracle and hostile-input tests for the clean-room H.263 decoder (`h263-decode`).

use std::process::Command;

use craft_video::h263::{Decoder, Picture, PictureType};

fn picture_bytes(pic: &Picture) -> Vec<u8> {
    [&pic.y[..], &pic.u[..], &pic.v[..]].concat()
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len(), "frame byte length mismatch");
    let se: f64 = a.iter().zip(b).map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2)).sum();
    if se == 0.0 {
        return 99.0;
    }
    10.0 * (255.0f64 * 255.0 * a.len() as f64 / se).log10()
}

/// Encode a synthetic sequence with ffmpeg, decode it with ffmpeg and with [`Decoder`], and
/// compare every picture (`PSNR >= min_psnr`, which allows for the IDCT's rounding).
fn check_stream(name: &str, w: u32, h: u32, frames: usize, args: &[&str], min_psnr: f64) -> Vec<Picture> {
    let Some(ff) = craft_media_testkit::ffmpeg_or_skip("h263 decode oracle") else { return Vec::new() };
    let dir = craft_media_testkit::private_dir(&format!("h263dec-{name}"));
    let path = dir.join("stream.h263");
    let encode = Command::new(&ff)
        .args(["-hide_banner", "-nostdin", "-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg(format!("testsrc2=size={w}x{h}:rate=25"))
        .args(["-frames:v", &frames.to_string(), "-pix_fmt", "yuv420p"])
        .args(args)
        .args(["-f", "rawvideo"])
        .arg(&path)
        .output()
        .expect("run ffmpeg encode");
    assert!(encode.status.success(), "{name}: encode failed: {}", String::from_utf8_lossy(&encode.stderr));
    let reference = Command::new(&ff)
        .args(["-hide_banner", "-nostdin", "-v", "error", "-f", "h263", "-i"])
        .arg(&path)
        .args(["-f", "rawvideo", "-pix_fmt", "yuv420p", "-"])
        .output()
        .expect("run ffmpeg decode");
    assert!(reference.status.success(), "{name}: reference decode failed: {}", String::from_utf8_lossy(&reference.stderr));
    let stream = std::fs::read(&path).expect("read stream");
    let _ = std::fs::remove_dir_all(&dir);

    let mut dec = Decoder::new();
    let pictures = dec.decode(&stream, 0).unwrap_or_else(|e| panic!("{name}: decode failed: {e}"));
    let frame_bytes = (w as usize * h as usize) + 2 * (w as usize).div_ceil(2) * (h as usize).div_ceil(2);
    assert_eq!(reference.stdout.len(), frame_bytes * frames, "{name}: reference byte count");
    assert_eq!(pictures.len(), frames, "{name}: picture count");
    for (i, (pic, expected)) in pictures.iter().zip(reference.stdout.chunks_exact(frame_bytes)).enumerate() {
        assert_eq!((pic.width, pic.height), (w, h), "{name}: size");
        let p = psnr(&picture_bytes(pic), expected);
        assert!(p >= min_psnr, "{name}: frame {i} PSNR {p:.2} dB < {min_psnr:.2} dB");
    }
    pictures
}

#[test]
fn decodes_baseline_standard_formats() {
    for (name, w, h) in [("subqcif", 128, 96), ("qcif", 176, 144), ("cif", 352, 288)] {
        let pictures = check_stream(name, w, h, 6, &["-c:v", "h263", "-q:v", "3"], 55.0);
        if let Some(first) = pictures.first() {
            assert_eq!(first.picture_type, PictureType::Intra);
            assert!(pictures[1..].iter().all(|p| p.picture_type == PictureType::Inter));
        }
    }
}

#[test]
fn decodes_custom_picture_formats() {
    // 200x120 and 100x68 are not standard formats and not multiples of 16: PLUSPTYPE with CPFMT.
    for (name, w, h) in [("custom_200x120", 200, 120), ("custom_100x68", 100, 68)] {
        check_stream(name, w, h, 5, &["-c:v", "h263p", "-threads", "1", "-q:v", "3"], 55.0);
    }
}

#[test]
fn decodes_unrestricted_motion_vectors() {
    check_stream("umv", 176, 144, 6, &["-c:v", "h263p", "-threads", "1", "-q:v", "3", "-umv", "1"], 55.0);
}

#[test]
fn decodes_overlapped_motion_compensation_and_four_vectors() {
    check_stream("obmc_p", 176, 144, 6, &["-c:v", "h263p", "-threads", "1", "-q:v", "3", "-obmc", "1", "-flags", "+mv4"], 55.0);
    check_stream("obmc_baseline_syntax", 176, 144, 6, &["-c:v", "h263", "-q:v", "3", "-obmc", "1", "-flags", "+mv4"], 55.0);
}

#[test]
fn decodes_deblocking_filter() {
    check_stream("deblock", 176, 144, 6, &["-c:v", "h263p", "-threads", "1", "-q:v", "12", "-flags", "+loop"], 55.0);
}

mod synthetic {
    use craft_media::bitstream::BitWriter;
    use craft_video::h263::{DecodeError, Decoder, PictureType};

    /// A picture start code, TR and the PTYPE of a standard format (`source` 1 = sub-QCIF, 128x96).
    fn header(w: &mut BitWriter, source: u32, inter: bool, quant: u32, flags: [bool; 4]) {
        w.write_bits(0x20, 22);
        w.write_bits(0, 8);
        w.write_bits(0b10, 2);
        w.write_bits(0, 3);
        w.write_bits(source, 3);
        w.write_bit(inter);
        for flag in flags {
            w.write_bit(flag); // UMV, SAC, AP, PB
        }
        w.write_bits(quant, 5);
        w.write_bit(false); // CPM
        w.write_bit(false); // PEI
    }

    /// A sub-QCIF picture of 8 by 6 macroblocks in 6 GOBs, each with a GOB header; `mb` writes one macroblock.
    fn picture(inter: bool, mut mb: impl FnMut(&mut BitWriter)) -> Vec<u8> {
        let mut w = BitWriter::new();
        header(&mut w, 1, inter, 8, [false; 4]);
        for gob in 0..6 {
            if gob > 0 {
                w.write_bits(1, 17);
                w.write_bits(gob, 5);
                w.write_bits(0, 2);
                w.write_bits(8, 5);
            }
            for _ in 0..8 {
                mb(&mut w);
            }
        }
        w.align_zero();
        w.finish()
    }

    /// An INTRA macroblock with no AC coefficients and every block's DC at `dc_code`.
    fn flat_intra(w: &mut BitWriter, dc_code: u32) {
        w.write_bits(1, 1); // MCBPC: INTRA, CBPC 00
        w.write_bits(0b0011, 4); // CBPY: no coded luminance block
        for _ in 0..6 {
            w.write_bits(dc_code, 8);
        }
    }

    #[test]
    fn flat_intra_picture_has_the_signalled_level() {
        let stream = picture(false, |w| flat_intra(w, 100));
        let pics = Decoder::new().decode(&stream, 7).expect("decode");
        assert_eq!(pics.len(), 1);
        let pic = &pics[0];
        assert_eq!((pic.width, pic.height, pic.pts), (128, 96, 7));
        assert_eq!(pic.picture_type, PictureType::Intra);
        assert_eq!(pic.info.pixel_aspect, (12, 11));
        // INTRADC code 100 is the level 800 of the transform, 100 after it.
        assert!(pic.y.iter().chain(&pic.u).chain(&pic.v).all(|&s| s == 100));
        assert_eq!((pic.y.len(), pic.u.len(), pic.v.len()), (128 * 96, 64 * 48, 64 * 48));
    }

    #[test]
    fn skipped_inter_picture_repeats_the_reference() {
        let mut dec = Decoder::new();
        let first = dec.decode(&picture(false, |w| flat_intra(w, 60)), 0).expect("intra");
        let skipped = dec.decode(&picture(true, |w| w.write_bit(true)), 1).expect("inter");
        assert_eq!(skipped[0].picture_type, PictureType::Inter);
        assert_eq!(skipped[0].y, first[0].y);
        assert_eq!(skipped[0].u, first[0].u);
        assert_eq!(skipped[0].v, first[0].v);
    }

    #[test]
    fn a_packet_may_hold_several_pictures() {
        let mut stream = picture(false, |w| flat_intra(w, 90));
        stream.extend(picture(true, |w| w.write_bit(true)));
        stream.extend(picture(true, |w| w.write_bit(true)));
        let pics = Decoder::new().decode(&stream, 3).expect("decode");
        assert_eq!(pics.len(), 3);
        assert!(pics.iter().all(|p| p.pts == 3 && p.y.iter().all(|&s| s == 90)));
    }

    #[test]
    fn inter_picture_needs_a_reference_of_its_size() {
        let inter = picture(true, |w| w.write_bit(true));
        assert!(matches!(Decoder::new().decode(&inter, 0), Err(DecodeError::Invalid(_))));
        let mut dec = Decoder::new();
        dec.decode(&picture(false, |w| flat_intra(w, 60)), 0).expect("intra");
        dec.reset();
        assert!(matches!(dec.decode(&inter, 0), Err(DecodeError::Invalid(_))), "reset forgets the reference");
    }

    #[test]
    fn refuses_what_it_does_not_implement() {
        for (name, flags) in [("SAC", [false, true, false, false]), ("PB-frames", [false, false, false, true])] {
            let mut w = BitWriter::new();
            header(&mut w, 1, true, 8, flags);
            w.write_bits(0, 32);
            let err = Decoder::new().decode(&w.finish(), 0).expect_err(name);
            assert!(matches!(err, DecodeError::Unsupported(_)), "{name}: {err}");
        }
        for source in [0, 6] {
            let mut w = BitWriter::new();
            header(&mut w, source, false, 8, [false; 4]);
            w.write_bits(0, 32);
            assert!(matches!(Decoder::new().decode(&w.finish(), 0), Err(DecodeError::Invalid(_))), "source format {source}");
        }
    }

    /// A PLUSPTYPE header for a custom format of `w` by `h` samples, I-picture, no optional modes.
    fn custom_header(width: u32, height: u32) -> BitWriter {
        let mut w = BitWriter::new();
        w.write_bits(0x20, 22);
        w.write_bits(0, 8);
        w.write_bits(0b10, 2);
        w.write_bits(0, 3);
        w.write_bits(7, 3);
        w.write_bits(1, 3); // UFEP
        w.write_bits(6, 3); // custom source format
        w.write_bits(0, 11); // PCF, UMV, SAC, AP, AIC, DF, SS, RPS, ISD, AIV, MQ: off
        w.write_bit(true); // marker
        w.write_bits(0, 3); // reserved
        w.write_bits(0, 3); // I-picture
        w.write_bits(0, 2); // RPR, RRU
        w.write_bit(false); // RTYPE
        w.write_bits(0, 2);
        w.write_bit(true); // marker
        w.write_bit(false); // CPM
        w.write_bits(1, 4); // square pixels
        w.write_bits(width / 4 - 1, 9);
        w.write_bit(true);
        w.write_bits(height / 4, 9);
        w.write_bits(8, 5); // PQUANT
        w.write_bit(false); // PEI
        w
    }

    #[test]
    fn custom_format_not_a_multiple_of_sixteen_is_cropped() {
        let (width, height) = (100u32, 68u32);
        let mut w = custom_header(width, height);
        let (mb_w, mb_h) = (width.div_ceil(16), height.div_ceil(16));
        // k = 1 macroblock row per GOB.
        for gob in 0..mb_h {
            if gob > 0 {
                w.write_bits(1, 17);
                w.write_bits(gob, 5);
                w.write_bits(0, 2);
                w.write_bits(8, 5);
            }
            for _ in 0..mb_w {
                flat_intra(&mut w, 70);
            }
        }
        w.align_zero();
        let pics = Decoder::new().decode(&w.finish(), 0).expect("decode custom format");
        let pic = &pics[0];
        assert_eq!((pic.width, pic.height), (width, height));
        assert_eq!(pic.y.len(), (width * height) as usize);
        assert_eq!(pic.u.len(), (width / 2 * (height / 2)) as usize);
        assert!(pic.y.iter().all(|&s| s == 70));
        assert_eq!(pic.info.pixel_aspect, (1, 1));
    }

    #[test]
    fn a_picture_larger_than_the_budget_is_refused_before_it_is_allocated() {
        let mut w = custom_header(2048, 1152);
        w.write_bits(0, 64);
        let stream = w.finish();
        let mut dec = Decoder::new();
        dec.set_memory_budget(1 << 20);
        assert!(matches!(dec.decode(&stream, 0), Err(DecodeError::Invalid(_))));
    }

    #[test]
    fn truncated_and_corrupted_streams_return_errors_not_panics() {
        let intra = picture(false, |w| flat_intra(w, 100));
        let inter = picture(true, |w| {
            w.write_bit(false); // COD: coded
            w.write_bits(0b1, 1); // MCBPC: INTER, CBPC 00
            w.write_bits(0b0011, 4); // CBPY (inter, no coded blocks)
            w.write_bits(1, 1); // MVD x = 0
            w.write_bits(1, 1); // MVD y = 0
        });
        for stream in [&intra, &inter] {
            for len in 0..stream.len() {
                let mut dec = Decoder::new();
                let _ = dec.decode(&intra, 0);
                let _ = dec.decode(&stream[..len], 0);
            }
            for bit in 0..stream.len() * 8 {
                let mut broken = stream.to_vec();
                broken[bit / 8] ^= 0x80 >> (bit % 8);
                let mut dec = Decoder::new();
                let _ = dec.decode(&intra, 0);
                let _ = dec.decode(&broken, 0);
            }
        }
    }
}
