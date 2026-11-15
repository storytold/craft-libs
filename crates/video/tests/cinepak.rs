//! Clean-room tests for Cinepak video decoding: synthetic test vectors, hostile input fuzzing,
//! and black-box verification against FFmpeg.

use craft_video::cinepak::{Decoder, Error, decode_frame, decode_frame_with_palette};

/// Helper to build a minimal 4x4 Cinepak keyframe with a single V1 vector.
fn build_synthetic_v1_keyframe(y: [u8; 4], u: i8, v: i8) -> Vec<u8> {
    let mut data = Vec::new();
    // Total length:
    // Frame header: 10 bytes
    // Strip header: 12 bytes
    // Codebook subchunk 0x22 (V1 color): 4-byte header + 6 bytes payload = 10 bytes
    // Vector subchunk 0x32 (V1 only): 4-byte header + 1 byte index = 5 bytes
    // Total = 10 + 12 + 10 + 5 = 37 bytes.
    let total_len = 37u32;
    let strip_len = 27u16;

    // Frame header
    data.push(0x00); // Intra keyframe
    data.extend_from_slice(&total_len.to_be_bytes()[1..4]); // 24-bit total length
    data.extend_from_slice(&4u16.to_be_bytes()); // Width 4
    data.extend_from_slice(&4u16.to_be_bytes()); // Height 4
    data.extend_from_slice(&1u16.to_be_bytes()); // 1 strip

    // Strip header
    data.extend_from_slice(&0x1000u16.to_be_bytes()); // Strip ID (intra)
    data.extend_from_slice(&strip_len.to_be_bytes()); // Strip length
    data.extend_from_slice(&0u16.to_be_bytes()); // y0
    data.extend_from_slice(&0u16.to_be_bytes()); // x0
    data.extend_from_slice(&4u16.to_be_bytes()); // y1
    data.extend_from_slice(&4u16.to_be_bytes()); // x1

    // Subchunk 0x22 (V1 color full)
    data.push(0x22);
    data.extend_from_slice(&10u32.to_be_bytes()[1..4]); // Size 10
    data.extend_from_slice(&y);
    data.push(u as u8);
    data.push(v as u8);

    // Subchunk 0x32 (V1 only vector)
    data.push(0x32);
    data.extend_from_slice(&5u32.to_be_bytes()[1..4]); // Size 5
    data.push(0); // Vector index 0

    data
}

#[test]
fn test_synthetic_v1_keyframe() {
    // Pure gray: Y=180, U=0, V=0
    let packet = build_synthetic_v1_keyframe([180, 180, 180, 180], 0, 0);
    let frame = decode_frame(&packet).expect("decode");

    assert_eq!(frame.width, 4);
    assert_eq!(frame.height, 4);
    assert!(frame.key);
    assert!(frame.is_color);

    // Every pixel in the 4x4 frame should be RGB (180, 180, 180)
    for y in 0..4 {
        for x in 0..4 {
            let px = frame.pixel_rgb(x, y).expect("pixel");
            assert_eq!(px, [180, 180, 180], "mismatch at ({x}, {y})");
        }
    }
}

#[test]
fn test_synthetic_v4_keyframe() {
    let mut data = Vec::new();
    // 4x4 frame with V4 codebook (4 vectors: vector 0=black, vector 1=red, vector 2=green, vector 3=blue).
    // Vector 0: Y=0, U=0, V=0 (black)
    // Vector 1: Y=72, U=-37, V=91 (pure red, R=254, G=0, B=0)
    // Vector 2: Y=144, U=-72, V=-72 (green, R=0, G=252, B=0)
    // Vector 3: Y=72, U=91, V=-37 (blue, R=0, G=1, B=254)
    // Subchunk 0x20: size = 4 + 4 * 6 = 28 bytes
    // Subchunk 0x30: size = 4 + 4 (flags) + 4 (indices) = 12 bytes
    // Strip header: 12 bytes. Frame header: 10 bytes.
    // Total = 10 + 12 + 28 + 12 = 62 bytes.
    let total_len = 62u32;
    let strip_len = 52u16;

    // Frame header
    data.push(0x00);
    data.extend_from_slice(&total_len.to_be_bytes()[1..4]);
    data.extend_from_slice(&4u16.to_be_bytes());
    data.extend_from_slice(&4u16.to_be_bytes());
    data.extend_from_slice(&1u16.to_be_bytes());

    // Strip header
    data.extend_from_slice(&0x1000u16.to_be_bytes());
    data.extend_from_slice(&strip_len.to_be_bytes());
    data.extend_from_slice(&0u16.to_be_bytes());
    data.extend_from_slice(&0u16.to_be_bytes());
    data.extend_from_slice(&4u16.to_be_bytes());
    data.extend_from_slice(&4u16.to_be_bytes());

    // Subchunk 0x20 (V4 color full)
    data.push(0x20);
    data.extend_from_slice(&28u32.to_be_bytes()[1..4]);
    // Vec 0: black
    data.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    // Vec 1: red
    data.extend_from_slice(&[72, 72, 72, 72, (-37i8) as u8, 91u8]);
    // Vec 2: green
    data.extend_from_slice(&[144, 144, 144, 144, (-72i8) as u8, (-72i8) as u8]);
    // Vec 3: blue
    data.extend_from_slice(&[72, 72, 72, 72, 91u8, (-37i8) as u8]);

    // Subchunk 0x30 (Intra vector: 1 bit per macroblock = 1 for V4)
    data.push(0x30);
    data.extend_from_slice(&12u32.to_be_bytes()[1..4]);
    data.extend_from_slice(&0x8000_0000u32.to_be_bytes()); // Bit 31 is 1 (V4 block)
    data.extend_from_slice(&[0, 1, 2, 3]); // Quadrants 0, 1, 2, 3

    let frame = decode_frame(&data).expect("decode");

    // Quadrant 0 (top-left 2x2): vector 0 (black)
    assert_eq!(frame.pixel_rgb(0, 0).expect("pixel"), [0, 0, 0]);
    assert_eq!(frame.pixel_rgb(1, 1).expect("pixel"), [0, 0, 0]);

    // Quadrant 1 (top-right 2x2): vector 1 (red)
    let px_tr = frame.pixel_rgb(2, 0).expect("pixel");
    assert!(px_tr[0] > 200 && px_tr[1] == 0 && px_tr[2] == 0, "expected red, got {px_tr:?}");

    // Quadrant 2 (bottom-left 2x2): vector 2 (green)
    let px_bl = frame.pixel_rgb(0, 2).expect("pixel");
    assert!(px_bl[1] > 200 && px_bl[0] == 0 && px_bl[2] == 0, "expected green, got {px_bl:?}");

    // Quadrant 3 (bottom-right 2x2): vector 3 (blue)
    let px_br = frame.pixel_rgb(2, 2).expect("pixel");
    assert!(px_br[2] > 200 && px_br[0] == 0, "expected blue, got {px_br:?}");
}

#[test]
fn test_synthetic_interframe_skip_and_update() {
    let mut decoder = Decoder::new();

    // Frame 0: 8x4 keyframe (2 macroblocks horizontally).
    // Initialize both macroblocks with black (V1 vec 0).
    // Codebook 0x22: 1 vector (black).
    // Subchunk 0x32: 2 indices [0, 0].
    let mut f0_data = Vec::new();
    let f0_total = 10 + 12 + 10 + 6; // 38 bytes
    f0_data.push(0x00);
    f0_data.extend_from_slice(&(f0_total as u32).to_be_bytes()[1..4]);
    f0_data.extend_from_slice(&8u16.to_be_bytes()); // Width 8
    f0_data.extend_from_slice(&4u16.to_be_bytes()); // Height 4
    f0_data.extend_from_slice(&1u16.to_be_bytes());

    f0_data.extend_from_slice(&0x1000u16.to_be_bytes());
    f0_data.extend_from_slice(&28u16.to_be_bytes());
    f0_data.extend_from_slice(&0u16.to_be_bytes());
    f0_data.extend_from_slice(&0u16.to_be_bytes());
    f0_data.extend_from_slice(&4u16.to_be_bytes());
    f0_data.extend_from_slice(&8u16.to_be_bytes());

    // 0x22: 1 black vector
    f0_data.push(0x22);
    f0_data.extend_from_slice(&10u32.to_be_bytes()[1..4]);
    f0_data.extend_from_slice(&[0, 0, 0, 0, 0, 0]);

    // 0x32: 2 blocks
    f0_data.push(0x32);
    f0_data.extend_from_slice(&6u32.to_be_bytes()[1..4]);
    f0_data.extend_from_slice(&[0, 0]);

    let f0 = decoder.decode(&f0_data).expect("decode f0");
    assert_eq!(f0.pixel_rgb(0, 0).expect("pixel"), [0, 0, 0]);
    assert_eq!(f0.pixel_rgb(4, 0).expect("pixel"), [0, 0, 0]);

    // Frame 1: interframe!
    // Update codebook V1: vector 1 = white (Y=255, U=0, V=0).
    // Subchunk 0x31:
    // Block 0: skipped (bit 0 = 0).
    // Block 1: updated with V1 (bits 10).
    // Flags word: bit 31=0 (skip block 0), bit 30=1, bit 29=0 (V1 block 1) => 0b0100_0000... = 0x4000_0000.
    // 1 index byte for block 1: index 0.
    let mut f1_data = Vec::new();
    let f1_total = 10 + 12 + 10 + 9; // 41 bytes
    f1_data.push(0x01); // Interframe
    f1_data.extend_from_slice(&(f1_total as u32).to_be_bytes()[1..4]);
    f1_data.extend_from_slice(&8u16.to_be_bytes());
    f1_data.extend_from_slice(&4u16.to_be_bytes());
    f1_data.extend_from_slice(&1u16.to_be_bytes());

    f1_data.extend_from_slice(&0x1100u16.to_be_bytes()); // Inter strip
    f1_data.extend_from_slice(&31u16.to_be_bytes());
    f1_data.extend_from_slice(&0u16.to_be_bytes());
    f1_data.extend_from_slice(&0u16.to_be_bytes());
    f1_data.extend_from_slice(&4u16.to_be_bytes());
    f1_data.extend_from_slice(&8u16.to_be_bytes());

    // 0x22: vector 0 becomes white
    f1_data.push(0x22);
    f1_data.extend_from_slice(&10u32.to_be_bytes()[1..4]);
    f1_data.extend_from_slice(&[255, 255, 255, 255, 0, 0]);

    // 0x31: inter vectors
    f1_data.push(0x31);
    f1_data.extend_from_slice(&9u32.to_be_bytes()[1..4]);
    f1_data.extend_from_slice(&0x4000_0000u32.to_be_bytes());
    f1_data.push(0); // Vector index for block 1

    let f1 = decoder.decode(&f1_data).expect("decode f1");
    assert!(!f1.key);

    // Block 0 was skipped: MUST STILL BE BLACK
    assert_eq!(f1.pixel_rgb(0, 0).expect("pixel"), [0, 0, 0]);
    assert_eq!(f1.pixel_rgb(3, 3).expect("pixel"), [0, 0, 0]);

    // Block 1 was updated: MUST BE WHITE
    assert_eq!(f1.pixel_rgb(4, 0).expect("pixel"), [255, 255, 255]);
    assert_eq!(f1.pixel_rgb(7, 3).expect("pixel"), [255, 255, 255]);
}

#[test]
fn test_grayscale_and_custom_palette() {
    // 4x4 frame using 8-bit grayscale codebook 0x26 (V1)
    let mut data = Vec::new();
    let total_len = 10 + 12 + 8 + 5; // 35 bytes
    data.push(0x00);
    data.extend_from_slice(&(total_len as u32).to_be_bytes()[1..4]);
    data.extend_from_slice(&4u16.to_be_bytes());
    data.extend_from_slice(&4u16.to_be_bytes());
    data.extend_from_slice(&1u16.to_be_bytes());

    data.extend_from_slice(&0x1000u16.to_be_bytes());
    data.extend_from_slice(&25u16.to_be_bytes());
    data.extend_from_slice(&0u16.to_be_bytes());
    data.extend_from_slice(&0u16.to_be_bytes());
    data.extend_from_slice(&4u16.to_be_bytes());
    data.extend_from_slice(&4u16.to_be_bytes());

    // 0x26: 8-bit V1 codebook (4 bytes: index 42)
    data.push(0x26);
    data.extend_from_slice(&8u32.to_be_bytes()[1..4]);
    data.extend_from_slice(&[42, 42, 42, 42]);

    // 0x32: 1 block index 0
    data.push(0x32);
    data.extend_from_slice(&5u32.to_be_bytes()[1..4]);
    data.push(0);

    // 1. Decode as raw grayscale (no palette)
    let f_gray = decode_frame(&data).expect("decode grayscale");
    assert!(!f_gray.is_color);
    assert_eq!(f_gray.pixel_rgb(0, 0).expect("pixel"), [42, 42, 42]);
    assert_eq!(f_gray.y[0], 42);

    // 2. Decode with a custom palette where index 42 is bright yellow (255, 255, 0)
    let mut palette = [0u8; 768];
    palette[42 * 3] = 255;
    palette[42 * 3 + 1] = 255;
    palette[42 * 3 + 2] = 0;

    let f_pal = decode_frame_with_palette(&data, &palette).expect("decode paletted");
    assert_eq!(f_pal.pixel_rgb(0, 0).expect("pixel"), [255, 255, 0]);
}

#[test]
fn test_hostile_inputs_never_panic() {
    let mut decoder = Decoder::new();

    // Empty slice
    assert!(matches!(decoder.decode(&[]), Err(Error::Truncated)));

    // Header shorter than 10 bytes
    assert!(matches!(decoder.decode(&[0; 9]), Err(Error::Truncated)));

    // Zero width/height
    let zero_dims = [0x00, 0, 0, 10, 0, 0, 0, 10, 0, 1];
    assert!(matches!(decoder.decode(&zero_dims), Err(Error::InvalidHeader(_))));

    // Zero strips
    let zero_strips = [0x00, 0, 0, 10, 0, 10, 0, 10, 0, 0];
    assert!(matches!(decoder.decode(&zero_strips), Err(Error::InvalidHeader(_))));

    // Unreasonable dimensions exceeding memory budget
    let huge_dims = [0x00, 0, 0, 10, 0xFF, 0xFF, 0xFF, 0xFF, 0, 1];
    assert!(matches!(decoder.decode(&huge_dims), Err(Error::MemoryBudget { .. })));

    // Interframe before keyframe
    let inter_first = [0x01, 0, 0, 10, 0, 16, 0, 16, 0, 1];
    assert!(matches!(decoder.decode(&inter_first), Err(Error::MissingReference)));

    // Random garbage
    for len in [10, 15, 25, 50, 100] {
        let garbage = vec![0xEE; len];
        let _ = decoder.decode(&garbage);
    }
}

#[test]
fn test_ffmpeg_oracle_rgb24_and_gray() {
    let Some(ff) = craft_media_testkit::ffmpeg_or_skip("cinepak oracle") else { return };

    let temp_dir = std::env::temp_dir().join("craft_cinepak_test");
    let _ = std::fs::create_dir_all(&temp_dir);

    // Test 1: Color Cinepak AVI
    let avi_color = temp_dir.join("test_color.avi");
    let cmd = std::process::Command::new(&ff)
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=16x16:rate=1:duration=2",
            "-c:v",
            "cinepak",
            "-pix_fmt",
            "rgb24",
            avi_color.to_str().expect("path"),
        ])
        .output();
    let Ok(output) = cmd else { return };
    if !output.status.success() {
        return;
    }

    // Decode oracle reference using ffmpeg to raw rgb24
    let oracle_raw = std::process::Command::new(&ff)
        .args(["-i", avi_color.to_str().expect("path"), "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .expect("oracle decode")
        .stdout;

    // Read and decode through craft-media AVI demuxer and our cinepak decoder
    let avi_bytes = std::fs::read(&avi_color).expect("read avi");
    let avi = craft_media::avi::open(&avi_bytes).expect("avi open");
    let mut decoder = Decoder::new();

    let stream = &avi.streams[0];
    assert_eq!(stream.codec_params().codec, craft_media::CodecId::Cinepak);

    for (frame_idx, _) in stream.chunks.iter().enumerate() {
        let chunk_data = avi.read_chunk(&avi_bytes, 0, frame_idx).expect("read chunk");
        let frame = decoder.decode(&chunk_data).expect("decode frame");
        assert_eq!(frame.width, 16);
        assert_eq!(frame.height, 16);

        let oracle_slice = &oracle_raw[frame_idx * 16 * 16 * 3..(frame_idx + 1) * 16 * 16 * 3];
        assert_eq!(frame.rgb, oracle_slice, "frame {frame_idx} pixel mismatch with ffmpeg oracle");
    }
    assert_eq!(stream.chunks.len(), 2, "expected 2 decoded frames");

    // Test 2: Grayscale Cinepak AVI
    let avi_gray = temp_dir.join("test_gray.avi");
    let cmd_gray = std::process::Command::new(&ff)
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=16x16:rate=1:duration=2",
            "-c:v",
            "cinepak",
            "-pix_fmt",
            "gray",
            avi_gray.to_str().expect("path"),
        ])
        .output();
    let Ok(output_gray) = cmd_gray else { return };
    if !output_gray.status.success() {
        return;
    }

    let oracle_gray_raw = std::process::Command::new(&ff)
        .args(["-i", avi_gray.to_str().expect("path"), "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .expect("oracle gray decode")
        .stdout;

    let avi_gray_bytes = std::fs::read(&avi_gray).expect("read gray avi");
    let avi_gray = craft_media::avi::open(&avi_gray_bytes).expect("avi gray open");
    let mut gray_decoder = Decoder::new();

    let gray_stream = &avi_gray.streams[0];
    assert_eq!(gray_stream.codec_params().codec, craft_media::CodecId::Cinepak);

    for (gray_frame_idx, _) in gray_stream.chunks.iter().enumerate() {
        let chunk_data = avi_gray.read_chunk(&avi_gray_bytes, 0, gray_frame_idx).expect("read gray chunk");
        let frame = gray_decoder.decode(&chunk_data).expect("decode gray frame");
        assert_eq!(frame.width, 16);
        assert_eq!(frame.height, 16);

        let oracle_slice = &oracle_gray_raw[gray_frame_idx * 16 * 16 * 3..(gray_frame_idx + 1) * 16 * 16 * 3];
        assert_eq!(frame.rgb, oracle_slice, "gray frame {gray_frame_idx} mismatch with ffmpeg oracle");
    }
    assert_eq!(gray_stream.chunks.len(), 2, "expected 2 decoded gray frames");

    // Test 3: QuickTime MOV Cinepak demux & decode
    let mov_color = temp_dir.join("test_color.mov");
    let cmd_mov = std::process::Command::new(&ff)
        .args(["-y", "-f", "lavfi", "-i", "testsrc=size=16x16:rate=1:duration=2", "-c:v", "cinepak", mov_color.to_str().expect("path")])
        .output();
    let Ok(output_mov) = cmd_mov else { return };
    if !output_mov.status.success() {
        return;
    }

    let oracle_mov_raw = std::process::Command::new(&ff)
        .args(["-i", mov_color.to_str().expect("path"), "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .expect("oracle mov decode")
        .stdout;

    let mov_bytes = std::fs::read(&mov_color).expect("read mov");
    let mp4 = craft_media::isobmff::open(&mov_bytes).expect("open isobmff mov");
    let (track_idx, track) = mp4.tracks.iter().enumerate().find(|(_, t)| t.kind == craft_media::isobmff::TrackKind::Video).expect("video track");
    assert_eq!(track.codec_params().codec, craft_media::CodecId::Cinepak);

    let mut mov_decoder = Decoder::new();
    for sample_idx in 0..track.samples.len() {
        let sample_data = mp4.read_sample(&mov_bytes, track_idx, sample_idx).expect("read sample");
        let frame = mov_decoder.decode(&sample_data).expect("decode mov frame");
        assert_eq!(frame.width, 16);
        assert_eq!(frame.height, 16);

        let oracle_slice = &oracle_mov_raw[sample_idx * 16 * 16 * 3..(sample_idx + 1) * 16 * 16 * 3];
        assert_eq!(frame.rgb, oracle_slice, "mov frame {sample_idx} mismatch with ffmpeg oracle");
    }
}

#[test]
fn test_multistrip_relative_coordinates() {
    // 4x8 frame with 2 strips (4x4 each).
    // Strip 0: y0=0, x0=0, y1=4, x1=4. (V1 codebook: red)
    // Strip 1: y0=0, x0=0, y1=4, x1=4. (relative Y coordinate: resolves to y0=4, y1=8). (V1 codebook: blue)
    let mut data = Vec::new();
    // Strip len for each: 12 (strip header) + 10 (0x22 codebook) + 5 (0x32 vector) = 27 bytes.
    // Frame len: 10 + 27 + 27 = 64 bytes.
    let total_len = 64u32;
    data.push(0x00);
    data.extend_from_slice(&total_len.to_be_bytes()[1..4]);
    data.extend_from_slice(&4u16.to_be_bytes()); // Width 4
    data.extend_from_slice(&8u16.to_be_bytes()); // Height 8
    data.extend_from_slice(&2u16.to_be_bytes()); // 2 strips

    // Strip 0
    data.extend_from_slice(&0x1000u16.to_be_bytes());
    data.extend_from_slice(&27u16.to_be_bytes());
    data.extend_from_slice(&0u16.to_be_bytes());
    data.extend_from_slice(&0u16.to_be_bytes());
    data.extend_from_slice(&4u16.to_be_bytes());
    data.extend_from_slice(&4u16.to_be_bytes());
    // Strip 0 codebook (V1 red: Y=72, U=-37, V=91)
    data.push(0x22);
    data.extend_from_slice(&10u32.to_be_bytes()[1..4]);
    data.extend_from_slice(&[72, 72, 72, 72, (-37i8) as u8, 91u8]);
    // Strip 0 vector
    data.push(0x32);
    data.extend_from_slice(&5u32.to_be_bytes()[1..4]);
    data.push(0);

    // Strip 1 (y0=0 relative to strip 0's bottom 4, y1=4 relative height => y0=4..8)
    data.extend_from_slice(&0x1000u16.to_be_bytes());
    data.extend_from_slice(&27u16.to_be_bytes());
    data.extend_from_slice(&0u16.to_be_bytes());
    data.extend_from_slice(&0u16.to_be_bytes());
    data.extend_from_slice(&4u16.to_be_bytes());
    data.extend_from_slice(&4u16.to_be_bytes());
    // Strip 1 codebook (V1 blue: Y=72, U=91, V=-37)
    data.push(0x22);
    data.extend_from_slice(&10u32.to_be_bytes()[1..4]);
    data.extend_from_slice(&[72, 72, 72, 72, 91u8, (-37i8) as u8]);
    // Strip 1 vector
    data.push(0x32);
    data.extend_from_slice(&5u32.to_be_bytes()[1..4]);
    data.push(0);

    let frame = decode_frame(&data).expect("decode multistrip");
    assert_eq!(frame.width, 4);
    assert_eq!(frame.height, 8);

    // Top 4x4 (strip 0) must be red
    for y in 0..4 {
        for x in 0..4 {
            let px = frame.pixel_rgb(x, y).expect("pixel");
            assert!(px[0] > 200 && px[1] == 0 && px[2] == 0, "expected red at ({x}, {y}), got {px:?}");
        }
    }

    // Bottom 4x4 (strip 1) must be blue
    for y in 4..8 {
        for x in 0..4 {
            let px = frame.pixel_rgb(x, y).expect("pixel");
            assert!(px[2] > 200 && px[0] == 0, "expected blue at ({x}, {y}), got {px:?}");
        }
    }
}

#[test]
fn test_partial_codebook_updates() {
    let mut decoder = Decoder::new();

    // Frame 0: 4x4 keyframe with V1 codebook full (0x22).
    // Vector 0: black ([0,0,0,0], 0, 0)
    // Vector 1: black ([0,0,0,0], 0, 0)
    let mut f0 = Vec::new();
    let f0_strip_len = 12 + 16 + 5; // 33
    let f0_total = 10 + f0_strip_len; // 43
    f0.push(0x00);
    f0.extend_from_slice(&(f0_total as u32).to_be_bytes()[1..4]);
    f0.extend_from_slice(&4u16.to_be_bytes());
    f0.extend_from_slice(&4u16.to_be_bytes());
    f0.extend_from_slice(&1u16.to_be_bytes());

    f0.extend_from_slice(&0x1000u16.to_be_bytes());
    f0.extend_from_slice(&(f0_strip_len as u16).to_be_bytes());
    f0.extend_from_slice(&0u16.to_be_bytes());
    f0.extend_from_slice(&0u16.to_be_bytes());
    f0.extend_from_slice(&4u16.to_be_bytes());
    f0.extend_from_slice(&4u16.to_be_bytes());

    // 0x22: 2 black vectors
    f0.push(0x22);
    f0.extend_from_slice(&16u32.to_be_bytes()[1..4]);
    f0.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    f0.extend_from_slice(&[0, 0, 0, 0, 0, 0]);

    // 0x32: use vector 0
    f0.push(0x32);
    f0.extend_from_slice(&5u32.to_be_bytes()[1..4]);
    f0.push(0);

    let frame0 = decoder.decode(&f0).expect("decode f0");
    assert_eq!(frame0.pixel_rgb(0, 0).expect("pixel"), [0, 0, 0]);

    // Frame 1: interframe with 0x23 (partial V1 color update).
    // Update vector 1 only to white (255, 0, 0).
    // 32-bit mask: bit 31=0 (skip vec 0), bit 30=1 (update vec 1), rest 0 => 0x4000_0000.
    // Followed immediately by 6 bytes for vector 1.
    // Subchunk len = 4 (header) + 4 (mask) + 6 (vector 1) = 14 bytes.
    let mut f1 = Vec::new();
    let f1_subchunk_len = 4 + 4 + 6; // 14 bytes
    let f1_vec_len = 5; // 0x32 with index 1
    let f1_strip_len = 12 + f1_subchunk_len + f1_vec_len; // 31
    let f1_total = 10 + f1_strip_len; // 41

    f1.push(0x01);
    f1.extend_from_slice(&(f1_total as u32).to_be_bytes()[1..4]);
    f1.extend_from_slice(&4u16.to_be_bytes());
    f1.extend_from_slice(&4u16.to_be_bytes());
    f1.extend_from_slice(&1u16.to_be_bytes());

    f1.extend_from_slice(&0x1100u16.to_be_bytes());
    f1.extend_from_slice(&(f1_strip_len as u16).to_be_bytes());
    f1.extend_from_slice(&0u16.to_be_bytes());
    f1.extend_from_slice(&0u16.to_be_bytes());
    f1.extend_from_slice(&4u16.to_be_bytes());
    f1.extend_from_slice(&4u16.to_be_bytes());

    // 0x23: partial V1
    f1.push(0x23);
    f1.extend_from_slice(&(f1_subchunk_len as u32).to_be_bytes()[1..4]);
    f1.extend_from_slice(&0x4000_0000u32.to_be_bytes()); // word 0: updates vector 1
    // Entry for vector 1 (white)
    f1.extend_from_slice(&[255, 255, 255, 255, 0, 0]);

    // 0x32: update block to use vector 1
    f1.push(0x32);
    f1.extend_from_slice(&5u32.to_be_bytes()[1..4]);
    f1.push(1);

    let frame1 = decoder.decode(&f1).expect("decode f1");
    // Vector 1 is now white, so the block should be white
    assert_eq!(frame1.pixel_rgb(0, 0).expect("pixel"), [255, 255, 255]);
}
