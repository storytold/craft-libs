//! Clean-room pure-Rust Theora video tests (headers, codec roundtrips, hostile inputs, and FFmpeg oracle).

use craft_video::frame::VideoFrame;
use craft_video::theora::{ColorSpace, Decoder, Encoder, EncoderConfig, Error, IdentificationHeader, Picture, PixelFormat, SetupHeader};
use std::process::Command;

fn test_picture(w: usize, h: usize) -> Picture {
    let mut y = vec![0u8; w * h];
    let cw = w.div_ceil(2);
    let ch = h.div_ceil(2);
    let mut u = vec![128u8; cw * ch];
    let mut v = vec![128u8; cw * ch];

    for r in 0..h {
        for c in 0..w {
            let val = ((r * 255 / h) ^ (c * 255 / w)) as u8;
            y[r * w + c] = val;
        }
    }
    for r in 0..ch {
        for c in 0..cw {
            u[r * cw + c] = (128 + ((r as i32 - ch as i32 / 2) * 2)) as u8;
            v[r * cw + c] = (128 + ((c as i32 - cw as i32 / 2) * 2)) as u8;
        }
    }

    Picture {
        width: w as u32,
        height: h as u32,
        pic_width: w as u32,
        pic_height: h as u32,
        pic_x: 0,
        pic_y: 0,
        pixel_format: PixelFormat::Yuv420,
        color_space: ColorSpace::Undefined,
        keyframe: true,
        pts: Some(0),
        y,
        u,
        v,
        y_stride: w,
        uv_stride: cw,
    }
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    let mse = a.iter().zip(b).map(|(x, y)| (f64::from(*x) - f64::from(*y)).powi(2)).sum::<f64>() / a.len().max(1) as f64;
    if mse == 0.0 { 99.0 } else { 10.0 * (255.0f64 * 255.0 / mse).log10() }
}

#[test]
fn test_theora_headers_roundtrip() {
    let cfg = EncoderConfig {
        width: 320,
        height: 240,
        pic_width: 320,
        pic_height: 240,
        pic_x: 0,
        pic_y: 0,
        fps_num: 30,
        fps_den: 1,
        pixel_format: PixelFormat::Yuv420,
        color_space: ColorSpace::Rec470M,
        quality: 48,
        keyframe_distance: 64,
    };
    let enc = Encoder::new(cfg).expect("encoder");
    let (id_pkt, comm_pkt, setup_pkt) = enc.headers();

    assert_eq!(id_pkt.len(), 42);
    assert_eq!(&id_pkt[0..7], &[0x80, b't', b'h', b'e', b'o', b'r', b'a']);
    assert_eq!(&comm_pkt[0..7], &[0x81, b't', b'h', b'e', b'o', b'r', b'a']);
    assert_eq!(&setup_pkt[0..7], &[0x82, b't', b'h', b'e', b'o', b'r', b'a']);

    let dec = Decoder::from_headers(&id_pkt, &comm_pkt, &setup_pkt).expect("decoder from headers");
    assert_eq!(dec.info.frame_width, 320);
    assert_eq!(dec.info.frame_height, 240);
    assert_eq!(dec.info.pic_width, 320);
    assert_eq!(dec.info.pic_height, 240);
    assert_eq!(dec.info.fps_num, 30);
    assert_eq!(dec.info.fps_den, 1);
    assert_eq!(dec.info.pixel_format, PixelFormat::Yuv420);
    assert_eq!(dec.info.color_space, ColorSpace::Rec470M);
}

#[test]
fn test_theora_encode_decode_intra_frame() {
    let (w, h) = (64, 64);
    let cfg = EncoderConfig {
        width: w as u32,
        height: h as u32,
        pic_width: w as u32,
        pic_height: h as u32,
        pic_x: 0,
        pic_y: 0,
        fps_num: 25,
        fps_den: 1,
        pixel_format: PixelFormat::Yuv420,
        color_space: ColorSpace::Undefined,
        quality: 48,
        keyframe_distance: 64,
    };
    let mut enc = Encoder::new(cfg).expect("encoder");
    let (id_pkt, comm_pkt, setup_pkt) = enc.headers();
    let mut dec = Decoder::from_headers(&id_pkt, &comm_pkt, &setup_pkt).expect("decoder");

    let original = test_picture(w, h);
    let packet = enc.encode_frame(&original, true).expect("encode");
    assert!(!packet.is_empty());

    let decoded = dec.decode_packet(&packet).expect("decode");
    assert!(decoded.keyframe);
    assert_eq!(decoded.width, w as u32);
    assert_eq!(decoded.height, h as u32);

    let p_y = psnr(&original.y, &decoded.y);
    let p_u = psnr(&original.u, &decoded.u);
    let p_v = psnr(&original.v, &decoded.v);

    assert!(p_y >= 25.0, "Luma PSNR too low: {p_y:.2} dB");
    assert!(p_u >= 30.0, "Chroma U PSNR too low: {p_u:.2} dB");
    assert!(p_v >= 30.0, "Chroma V PSNR too low: {p_v:.2} dB");
}

#[test]
fn test_theora_empty_packet_inter_frame() {
    let (w, h) = (32, 32);
    let cfg = EncoderConfig {
        width: w as u32,
        height: h as u32,
        pic_width: w as u32,
        pic_height: h as u32,
        pic_x: 0,
        pic_y: 0,
        fps_num: 25,
        fps_den: 1,
        pixel_format: PixelFormat::Yuv420,
        color_space: ColorSpace::Undefined,
        quality: 40,
        keyframe_distance: 64,
    };
    let mut enc = Encoder::new(cfg).expect("encoder");
    let (id_pkt, comm_pkt, setup_pkt) = enc.headers();
    let mut dec = Decoder::from_headers(&id_pkt, &comm_pkt, &setup_pkt).expect("decoder");

    let pic = test_picture(w, h);
    let key_pkt = enc.encode_frame(&pic, true).expect("encode keyframe");
    let first_dec = dec.decode_packet(&key_pkt).expect("decode keyframe");
    assert!(first_dec.keyframe);

    // 0-byte packet is a repeat frame
    let repeat_dec = dec.decode_packet(&[]).expect("decode empty packet");
    assert!(!repeat_dec.keyframe);
    assert_eq!(first_dec.y, repeat_dec.y);
    assert_eq!(first_dec.u, repeat_dec.u);
    assert_eq!(first_dec.v, repeat_dec.v);
}

#[test]
fn test_theora_video_frame_conversion() {
    let mut pic = test_picture(32, 32);
    pic.color_space = ColorSpace::Rec470M;
    let vf = VideoFrame::try_from(pic.clone()).expect("convert to VideoFrame");
    assert_eq!(vf.width(), 32);
    assert_eq!(vf.height(), 32);
    assert_eq!(vf.planes().len(), 3);
    assert_eq!(vf.color.primaries, 4);
    assert_eq!(vf.color.transfer, 6);
    assert_eq!(vf.color.matrix, 6);

    pic.color_space = ColorSpace::Rec470Bg;
    let vf_bg = VideoFrame::try_from(pic.clone()).expect("convert Rec470Bg");
    assert_eq!(vf_bg.color.primaries, 5);
    assert_eq!(vf_bg.color.transfer, 1);
    assert_eq!(vf_bg.color.matrix, 5);

    // Empty or short plane buffers fail with FrameError without panicking
    let mut bad_pic = pic.clone();
    bad_pic.y.clear();
    assert!(VideoFrame::try_from(bad_pic).is_err());

    // Zero stride fails with FrameError without panicking
    let mut bad_stride = pic.clone();
    bad_stride.y_stride = 0;
    assert!(VideoFrame::try_from(bad_stride).is_err());

    // Short non-zero stride shorter than source plane width fails with FrameError
    let mut short_stride = pic.clone();
    short_stride.y_stride = 16;
    assert!(VideoFrame::try_from(short_stride).is_err());

    let mut short_uv_stride = pic.clone();
    short_uv_stride.uv_stride = 8;
    assert!(VideoFrame::try_from(short_uv_stride).is_err());

    // Oversized dimensions fail with FrameError without panicking
    let mut bad_dims = pic.clone();
    bad_dims.pic_width = u32::MAX;
    bad_dims.pic_height = u32::MAX;
    assert!(VideoFrame::try_from(bad_dims).is_err());

    // Matching frame and crop dimensions that exceed memory budget fail at budget check
    let mut budget_pic = test_picture(32, 32);
    budget_pic.width = 65536;
    budget_pic.height = 65536;
    budget_pic.pic_width = 65536;
    budget_pic.pic_height = 65536;
    budget_pic.y_stride = 65536;
    budget_pic.uv_stride = 32768;
    assert!(matches!(VideoFrame::try_from(budget_pic), Err(craft_video::frame::FrameError::Shape(_))));

    // Crop bounds exceeding frame dimensions fail with FrameError
    let mut bad_crop = pic.clone();
    bad_crop.pic_x = 30;
    bad_crop.pic_width = 10;
    assert!(VideoFrame::try_from(bad_crop).is_err());

    // Misaligned crop offsets (odd coordinates) preserve all chroma samples via S444 conversion
    let mut odd_pic = test_picture(32, 32);
    odd_pic.pic_x = 1;
    odd_pic.pic_y = 1;
    odd_pic.pic_width = 2;
    odd_pic.pic_height = 2;
    odd_pic.u[0] = 50;
    odd_pic.u[1] = 150;
    let vf_odd = VideoFrame::try_from(odd_pic).expect("convert odd crop");
    assert_eq!(vf_odd.subsampling(), craft_video::frame::Subsampling::S444);
    assert_eq!(vf_odd.width(), 2);
    assert_eq!(vf_odd.height(), 2);
    // Row 1 of VideoFrame corresponds to bottom row py=1 (cy=0)
    let u_bottom = vf_odd.planes()[1].row_u8(1).expect("u row 1");
    assert_eq!(u_bottom[0], 50);
    assert_eq!(u_bottom[1], 150);

    // Odd vertical crop with distinct chroma rows preserves vertical subsampling
    let mut odd_y_pic = test_picture(32, 32);
    odd_y_pic.pic_x = 0;
    odd_y_pic.pic_y = 1;
    odd_y_pic.pic_width = 4;
    odd_y_pic.pic_height = 3;
    odd_y_pic.u[0] = 77;
    odd_y_pic.u[16] = 88;
    let vf_odd_y = VideoFrame::try_from(odd_y_pic).expect("convert odd y crop");
    assert_eq!(vf_odd_y.subsampling(), craft_video::frame::Subsampling::S444);
    assert_eq!(vf_odd_y.height(), 3);

    // Aligned crop offset (pic_y = 0) with odd height (pic_height = 3) and distinct chroma rows selects S444 to preserve chroma pairing
    let mut odd_h_pic = test_picture(32, 32);
    odd_h_pic.pic_x = 0;
    odd_h_pic.pic_y = 0;
    odd_h_pic.pic_width = 4;
    odd_h_pic.pic_height = 3;
    odd_h_pic.u[0] = 10;
    odd_h_pic.u[16] = 20;
    let vf_odd_h = VideoFrame::try_from(odd_h_pic).expect("convert odd height crop");
    assert_eq!(vf_odd_h.subsampling(), craft_video::frame::Subsampling::S444);
    assert_eq!(vf_odd_h.height(), 3);
    assert_eq!(vf_odd_h.planes()[1].row_u8(0).unwrap()[0], 20);
    assert_eq!(vf_odd_h.planes()[1].row_u8(1).unwrap()[0], 10);
    assert_eq!(vf_odd_h.planes()[1].row_u8(2).unwrap()[0], 10);
}

#[test]
fn test_hostile_theora_packets_do_not_panic() {
    // 1. Truncated identification header
    assert!(IdentificationHeader::parse(&[]).is_err());
    assert!(IdentificationHeader::parse(&[0x80, b't', b'h', b'e', b'o', b'r', b'a']).is_err());

    // 2. Corrupted setup header
    assert!(SetupHeader::parse(&[0x82, b't', b'h', b'e', b'o', b'r', b'a', 0xFF, 0xFF]).is_err());

    // 3. Decoder on random / fuzzed packets
    let (id, comm, setup) = Encoder::new(EncoderConfig::default()).unwrap().headers();
    let mut dec = Decoder::from_headers(&id, &comm, &setup).expect("decoder");

    // Inter frame before keyframe must fail cleanly
    assert!(matches!(dec.decode_packet(&[0x40, 0x00]), Err(Error::MissingReference)));

    // Fuzzed inputs
    let fuzz_cases: &[&[u8]] = &[
        &[0x00],
        &[0x00, 0x00, 0x00, 0x00],
        &[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        &[0x80, 0x00, 0x00], // Header bit set in data packet
    ];

    for case in fuzz_cases {
        let _ = dec.decode_packet(case);
    }
}

#[test]
fn test_theora_ffmpeg_oracle_roundtrip() {
    let Some(ff) = craft_media_testkit::ffmpeg() else {
        return;
    };
    let dir = craft_media_testkit::fixture_subdir("theora");
    let (w, h) = (64, 64);
    let cfg = EncoderConfig {
        width: w as u32,
        height: h as u32,
        pic_width: w as u32,
        pic_height: h as u32,
        pic_x: 0,
        pic_y: 0,
        fps_num: 25,
        fps_den: 1,
        pixel_format: PixelFormat::Yuv420,
        color_space: ColorSpace::Undefined,
        quality: 45,
        keyframe_distance: 64,
    };
    let mut enc = Encoder::new(cfg).expect("encoder");
    let (id_pkt, comm_pkt, setup_pkt) = enc.headers();

    let pic = test_picture(w, h);
    let video_pkt = enc.encode_frame(&pic, true).expect("encode");

    // Write an Ogg Theora file using craft-media PageWriter
    let ogv_path = dir.join("cleanroom_test.ogv");
    let mut pw = craft_media::ogg::PageWriter::new(0x12345678);

    let mut file_bytes = Vec::new();
    file_bytes.extend(pw.header(&id_pkt));
    file_bytes.extend(pw.header(&comm_pkt));
    file_bytes.extend(pw.header(&setup_pkt));
    file_bytes.extend(pw.packet(&video_pkt, 1 << 6));
    file_bytes.extend(pw.finish(None));

    std::fs::write(&ogv_path, &file_bytes).expect("write ogv");

    // Run ffmpeg to decode our Ogg Theora file to raw YUV
    let raw_out = dir.join("ffmpeg_decoded.raw");
    let status = Command::new(&ff)
        .args(["-v", "error", "-i", ogv_path.to_str().unwrap(), "-f", "rawvideo", "-pix_fmt", "yuv420p", "-y", raw_out.to_str().unwrap()])
        .status()
        .expect("ffmpeg run");

    assert!(status.success(), "ffmpeg failed to decode our Theora file");

    let ff_raw = std::fs::read(&raw_out).expect("read ffmpeg output");
    assert_eq!(ff_raw.len(), w * h * 3 / 2);

    // Decode with our pure-Rust decoder
    let mut dec = Decoder::from_headers(&id_pkt, &comm_pkt, &setup_pkt).expect("decoder");
    let our_pic = dec.decode_packet(&video_pkt).expect("decode");

    // FFmpeg outputs top-to-bottom YUV, while Theora Picture is bottom-to-top internal storage
    // Convert our Picture to VideoFrame to get top-to-bottom raster!
    let vf = VideoFrame::try_from(our_pic).expect("to VideoFrame");
    let our_y = match vf.planes()[0].samples() {
        craft_video::frame::Samples::U8(v) => v.as_slice(),
        _ => panic!("u8 expected"),
    };

    let ff_y = &ff_raw[0..w * h];
    let p = psnr(ff_y, our_y);
    println!("Oracle PSNR against FFmpeg decode: {p:.2} dB");
    assert!(p >= 35.0, "FFmpeg decode differs noticeably: {p:.2} dB");
}

#[test]
fn test_theora_encoder_consecutive_calls_roundtrip() {
    let (w, h) = (32, 32);
    let cfg = EncoderConfig {
        width: w as u32,
        height: h as u32,
        pic_width: w as u32,
        pic_height: h as u32,
        pic_x: 0,
        pic_y: 0,
        fps_num: 25,
        fps_den: 1,
        pixel_format: PixelFormat::Yuv420,
        color_space: ColorSpace::Undefined,
        quality: 40,
        keyframe_distance: 64,
    };
    let mut enc = Encoder::new(cfg).expect("encoder");
    let (id_pkt, comm_pkt, setup_pkt) = enc.headers();
    let mut dec = Decoder::from_headers(&id_pkt, &comm_pkt, &setup_pkt).expect("decoder");

    let pic1 = test_picture(w, h);
    let pkt1 = enc.encode_frame(&pic1, false).expect("frame 1");
    let dec1 = dec.decode_packet(&pkt1).expect("decode frame 1");
    assert!(dec1.keyframe);

    let mut pic2 = test_picture(w, h);
    pic2.y[0] = 200;
    let pkt2 = enc.encode_frame(&pic2, false).expect("frame 2 with force_keyframe=false");
    let dec2 = dec.decode_packet(&pkt2).expect("decode frame 2");
    assert!(dec2.keyframe);
}

#[test]
fn test_theora_comment_header_bounds() {
    use craft_video::theora::CommentHeader;

    // Truncated vendor string length
    let truncated = vec![0x81, b't', b'h', b'e', b'o', b'r', b'a', 0x01];
    assert!(matches!(CommentHeader::parse(&truncated), Err(Error::Truncated(_))));

    // Vendor string length exceeding available bytes (or overflow on 32-bit: 0xFFFFFFFF)
    let mut overflow_vendor = vec![0x81, b't', b'h', b'e', b'o', b'r', b'a'];
    overflow_vendor.extend_from_slice(&0xFFFFFFFFu32.to_le_bytes());
    assert!(matches!(CommentHeader::parse(&overflow_vendor), Err(Error::Truncated(_))));

    // Valid vendor, but comment length overflows
    let mut overflow_comment = vec![0x81, b't', b'h', b'e', b'o', b'r', b'a'];
    overflow_comment.extend_from_slice(&4u32.to_le_bytes());
    overflow_comment.extend_from_slice(b"test");
    overflow_comment.extend_from_slice(&1u32.to_le_bytes()); // 1 comment
    overflow_comment.extend_from_slice(&0xFFFFFFFFu32.to_le_bytes()); // comment len overflows
    assert!(matches!(CommentHeader::parse(&overflow_comment), Err(Error::Truncated(_))));
}

#[test]
fn test_theora_hostile_dimensions_rejected() {
    // Zero dimensions
    let cfg_zero = EncoderConfig {
        width: 0,
        height: 0,
        pic_width: 0,
        pic_height: 0,
        pic_x: 0,
        pic_y: 0,
        fps_num: 25,
        fps_den: 1,
        pixel_format: PixelFormat::Yuv420,
        color_space: ColorSpace::Undefined,
        quality: 40,
        keyframe_distance: 64,
    };
    assert!(matches!(Encoder::new(cfg_zero), Err(Error::DimensionMismatch(_))));

    // Overflowing dimensions
    let cfg_overflow = EncoderConfig {
        width: u32::MAX,
        height: 16,
        pic_width: 16,
        pic_height: 16,
        pic_x: 0,
        pic_y: 0,
        fps_num: 25,
        fps_den: 1,
        pixel_format: PixelFormat::Yuv420,
        color_space: ColorSpace::Undefined,
        quality: 40,
        keyframe_distance: 64,
    };
    assert!(matches!(Encoder::new(cfg_overflow), Err(Error::DimensionMismatch(_))));

    // Massive dimensions exceeding memory budget
    let enc_ok = Encoder::new(EncoderConfig {
        width: 16,
        height: 16,
        pic_width: 16,
        pic_height: 16,
        pic_x: 0,
        pic_y: 0,
        fps_num: 25,
        fps_den: 1,
        pixel_format: PixelFormat::Yuv444,
        color_space: ColorSpace::Undefined,
        quality: 40,
        keyframe_distance: 64,
    })
    .expect("encoder");
    let (_, comm_pkt, setup_pkt) = enc_ok.headers();

    // Create synthetic 16384x16384 YUV444 ID packet (fmbw = 1024, fmbh = 1024)
    let mut huge_id = vec![0x80, b't', b'h', b'e', b'o', b'r', b'a', 3, 2, 0];
    huge_id.extend_from_slice(&1024u16.to_be_bytes()); // fmbw = 1024 (16384 px)
    huge_id.extend_from_slice(&1024u16.to_be_bytes()); // fmbh = 1024 (16384 px)
    huge_id.extend_from_slice(&[0, 0x40, 0x00]); // picw = 16384
    huge_id.extend_from_slice(&[0, 0x40, 0x00]); // pich = 16384
    huge_id.extend_from_slice(&[0, 0]); // picx, picy
    huge_id.extend_from_slice(&30u32.to_be_bytes()); // frn
    huge_id.extend_from_slice(&1u32.to_be_bytes()); // frd
    huge_id.extend_from_slice(&[0, 0, 1]); // parn
    huge_id.extend_from_slice(&[0, 0, 1]); // pard
    huge_id.push(0); // color space
    huge_id.extend_from_slice(&[0, 0, 0]); // nombr
    // last two bytes: quality=0, kfgshift=6, pf_bits=3 (Yuv444), reserved=0
    // (0 << 10) | (6 << 5) | (3 << 3) = 0 | 192 | 24 = 216 = 0x00D8
    huge_id.extend_from_slice(&0x00D8u16.to_be_bytes());

    assert!(matches!(Decoder::from_headers(&huge_id, &comm_pkt, &setup_pkt), Err(Error::OutOfMemory)));
}

#[test]
fn test_theora_encoder_picture_strides() {
    let (w, h) = (32, 32);
    let cfg = EncoderConfig {
        width: w as u32,
        height: h as u32,
        pic_width: w as u32,
        pic_height: h as u32,
        pic_x: 0,
        pic_y: 0,
        fps_num: 25,
        fps_den: 1,
        pixel_format: PixelFormat::Yuv420,
        color_space: ColorSpace::Undefined,
        quality: 50,
        keyframe_distance: 64,
    };
    let mut enc = Encoder::new(cfg).expect("encoder");
    let (id_pkt, comm_pkt, setup_pkt) = enc.headers();
    let mut dec = Decoder::from_headers(&id_pkt, &comm_pkt, &setup_pkt).expect("decoder");

    let y_stride = 64; // stride > width
    let uv_stride = 32; // stride > cw (16)
    let mut y = vec![0u8; y_stride * h];
    let _cw = w / 2;
    let ch = h / 2;
    let u = vec![128u8; uv_stride * ch];
    let v = vec![128u8; uv_stride * ch];

    for r in 0..h {
        for c in 0..w {
            y[r * y_stride + c] = ((r * 255 / h) ^ (c * 255 / w)) as u8;
        }
        // Fill stride padding with garbage to ensure encoder does not read it
        for c in w..y_stride {
            y[r * y_stride + c] = 255;
        }
    }

    let pic = Picture {
        width: w as u32,
        height: h as u32,
        pic_width: w as u32,
        pic_height: h as u32,
        pic_x: 0,
        pic_y: 0,
        pixel_format: PixelFormat::Yuv420,
        color_space: ColorSpace::Undefined,
        keyframe: true,
        pts: None,
        y,
        u,
        v,
        y_stride,
        uv_stride,
    };

    let pkt = enc.encode_frame(&pic, true).expect("encode with stride");
    let decoded = dec.decode_packet(&pkt).expect("decode");
    assert_eq!(decoded.width, w as u32);
    assert_eq!(decoded.height, h as u32);
}

#[test]
fn test_theora_encoder_invalid_input_rejected() {
    let cfg = EncoderConfig {
        width: 32,
        height: 32,
        pic_width: 32,
        pic_height: 32,
        pic_x: 0,
        pic_y: 0,
        fps_num: 25,
        fps_den: 1,
        pixel_format: PixelFormat::Yuv420,
        color_space: ColorSpace::Undefined,
        quality: 40,
        keyframe_distance: 64,
    };
    let mut enc = Encoder::new(cfg).expect("encoder");

    // Overflowing stride
    let mut pic = test_picture(32, 32);
    pic.y_stride = usize::MAX;
    assert!(matches!(enc.encode_frame(&pic, true), Err(Error::DimensionMismatch(_))));

    // Zero stride
    let mut pic = test_picture(32, 32);
    pic.y_stride = 0;
    assert!(matches!(enc.encode_frame(&pic, true), Err(Error::DimensionMismatch(_))));

    // Truncated/missing active samples
    let mut pic = test_picture(32, 32);
    pic.y.clear();
    assert!(matches!(enc.encode_frame(&pic, true), Err(Error::DimensionMismatch(_))));

    // Dimension mismatch
    let mut pic = test_picture(32, 32);
    pic.width = 64;
    assert!(matches!(enc.encode_frame(&pic, true), Err(Error::DimensionMismatch(_))));

    // Mismatched picture bounds (e.g. pic_width = 0 with cleared plane) cannot bypass validation
    let mut pic = test_picture(32, 32);
    pic.pic_width = 0;
    pic.y.clear();
    assert!(matches!(enc.encode_frame(&pic, true), Err(Error::DimensionMismatch(_))));

    let mut pic = test_picture(32, 32);
    pic.pic_height = 16;
    assert!(matches!(enc.encode_frame(&pic, true), Err(Error::DimensionMismatch(_))));

    let mut pic = test_picture(32, 32);
    pic.pic_x = 2;
    assert!(matches!(enc.encode_frame(&pic, true), Err(Error::DimensionMismatch(_))));

    // Short non-zero stride shorter than plane width
    let mut pic = test_picture(32, 32);
    pic.y_stride = 16;
    assert!(matches!(enc.encode_frame(&pic, true), Err(Error::DimensionMismatch(_))));

    // Out-of-frame configuration picture bounds cannot bypass active picture validation
    let cfg_out_of_frame = EncoderConfig {
        width: 32,
        height: 32,
        pic_width: 16,
        pic_height: 32,
        pic_x: 32,
        pic_y: 0,
        fps_num: 25,
        fps_den: 1,
        pixel_format: PixelFormat::Yuv420,
        color_space: ColorSpace::Undefined,
        quality: 40,
        keyframe_distance: 64,
    };
    assert!(matches!(Encoder::new(cfg_out_of_frame), Err(Error::DimensionMismatch(_))));

    let cfg_zero_pic = EncoderConfig {
        width: 32,
        height: 32,
        pic_width: 0,
        pic_height: 32,
        pic_x: 0,
        pic_y: 0,
        fps_num: 25,
        fps_den: 1,
        pixel_format: PixelFormat::Yuv420,
        color_space: ColorSpace::Undefined,
        quality: 40,
        keyframe_distance: 64,
    };
    assert!(matches!(Encoder::new(cfg_zero_pic), Err(Error::DimensionMismatch(_))));
}

#[test]
fn test_theora_yuv444_chroma_motion_vector_half_pixel() {
    use craft_video::theora::reconstruct::compute_block_predictor;

    let plane_w = 16;
    let plane_h = 16;
    let ref_y = vec![0u8; plane_w * plane_h];
    let mut ref_cb = vec![0u8; plane_w * plane_h];
    let ref_cr = vec![0u8; plane_w * plane_h];

    // Set pixel (1, 0) to 200, pixel (2, 0) to 100
    ref_cb[1] = 200;
    ref_cb[2] = 100;

    let mut pred = [[0u8; 8]; 8];

    // For Yuv444, motion vector (2, 0) is 1 full pixel in half-pixel units (rfi=1: previous reference frame)
    compute_block_predictor(1, 0, 0, 1, (2, 0), PixelFormat::Yuv444, &ref_y, &ref_cb, &ref_cr, plane_w, plane_h, &mut pred);

    // In Yuv444 without subsampling, (2, 0) moves by 1 full pixel, so pred[0][0] must be ref_cb[0 * plane_w + 1] = 200!
    // (If quarter-pixel units were used erroneously, it would have averaged ref_cb[0] and ref_cb[1] giving (0 + 200)/2 = 100)
    assert_eq!(pred[0][0], 200);
}

/// With an odd picture offset the chroma samples under the visible luma are counted from the
/// rounded-down start to the rounded-up end, so a missing edge sample is an error and a missing
/// sample outside the visible crop is not.
#[test]
fn test_theora_odd_crop_checks_the_chroma_edge_samples() {
    let cfg = EncoderConfig {
        width: 32,
        height: 32,
        pic_width: 2,
        pic_height: 2,
        pic_x: 1,
        pic_y: 0,
        fps_num: 25,
        fps_den: 1,
        pixel_format: PixelFormat::Yuv422,
        color_space: ColorSpace::Undefined,
        quality: 40,
        keyframe_distance: 64,
    };
    let mut enc = Encoder::new(cfg).expect("encoder");
    let mut pic = test_picture(32, 32);
    pic.pixel_format = PixelFormat::Yuv422;
    pic.pic_width = 2;
    pic.pic_height = 2;
    pic.pic_x = 1;
    pic.uv_stride = 16;
    pic.u = vec![128; 16 * 32];
    pic.v = vec![128; 16 * 32];
    assert!(enc.encode_frame(&pic, true).is_ok());
    // Luma x = 1 and x = 2 sit over chroma columns 0 and 1; row 1, column 1 is the last one needed.
    let needed = 16 + 2;
    let mut short = pic.clone();
    short.u.truncate(needed - 1);
    assert!(matches!(enc.encode_frame(&short, true), Err(Error::DimensionMismatch(_))));
    let mut short = pic.clone();
    short.v.truncate(needed - 1);
    assert!(matches!(enc.encode_frame(&short, true), Err(Error::DimensionMismatch(_))));
    // Everything past the crop may be missing.
    let mut trimmed = pic.clone();
    trimmed.u.truncate(needed);
    trimmed.v.truncate(needed);
    assert!(enc.encode_frame(&trimmed, true).is_ok());
}
