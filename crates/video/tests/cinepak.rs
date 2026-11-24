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

/// A decoded frame converts to the shared `VideoFrame`: the same RGB, as three planes.
#[test]
fn test_decoded_frame_converts_to_a_video_frame() {
    use craft_video::frame::{ColorFamily, Samples, VideoFrame};
    // Y=100, U=10, V=-20: not gray, so the three planes differ.
    let frame = decode_frame(&build_synthetic_v1_keyframe([100, 100, 100, 100], 10, -20)).expect("decode");
    let want = frame.pixel_rgb(3, 2).expect("pixel");
    let video = VideoFrame::try_from(frame).expect("converts");
    assert_eq!((video.width(), video.height(), video.family()), (4, 4, ColorFamily::Rgb));
    assert_ne!(want[0], want[2]);
    for (c, plane) in video.planes().iter().enumerate() {
        let Samples::U8(v) = plane.samples() else { panic!("8-bit planes") };
        assert_eq!(v[2 * 4 + 3], want[c], "plane {c}");
    }
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
fn test_interframe_grayscale_vector_neutralizes_chroma_planes() {
    let mut decoder = Decoder::new();

    // Frame 0: 8x4 color keyframe (2 macroblocks horizontally).
    let mut f0_data = Vec::new();
    let f0_total = 10 + 12 + 10 + 6;
    f0_data.push(0x00);
    f0_data.extend_from_slice(&(f0_total as u32).to_be_bytes()[1..4]);
    f0_data.extend_from_slice(&8u16.to_be_bytes()); // Width 8
    f0_data.extend_from_slice(&4u16.to_be_bytes()); // Height 4
    f0_data.extend_from_slice(&1u16.to_be_bytes()); // 1 strip

    f0_data.extend_from_slice(&0x1000u16.to_be_bytes());
    f0_data.extend_from_slice(&28u16.to_be_bytes());
    f0_data.extend_from_slice(&0u16.to_be_bytes());
    f0_data.extend_from_slice(&0u16.to_be_bytes());
    f0_data.extend_from_slice(&4u16.to_be_bytes());
    f0_data.extend_from_slice(&8u16.to_be_bytes());

    // 0x22: 1 color vector: Y=[100, 100, 100, 100], U=50, V=-40
    f0_data.push(0x22);
    f0_data.extend_from_slice(&10u32.to_be_bytes()[1..4]);
    f0_data.extend_from_slice(&[100, 100, 100, 100, 50, (-40i8) as u8]);

    // 0x32: 2 blocks
    f0_data.push(0x32);
    f0_data.extend_from_slice(&6u32.to_be_bytes()[1..4]);
    f0_data.extend_from_slice(&[0, 0]);

    let f0 = decoder.decode(&f0_data).expect("decode f0");
    assert!(f0.is_color);
    let u_plane = f0.u.as_ref().expect("u plane");
    let v_plane = f0.v.as_ref().expect("v plane");
    assert_eq!(u_plane[0], (50i16 + 128) as u8);
    assert_eq!(u_plane[2], (50i16 + 128) as u8);
    assert_eq!(v_plane[0], (-40i16 + 128) as u8);
    assert_eq!(v_plane[2], (-40i16 + 128) as u8);

    // Frame 1: interframe updating block 1 with grayscale codebook 0x26.
    let mut f1_data = Vec::new();
    let f1_total = 10 + 12 + 8 + 9;
    f1_data.push(0x01); // Interframe
    f1_data.extend_from_slice(&(f1_total as u32).to_be_bytes()[1..4]);
    f1_data.extend_from_slice(&8u16.to_be_bytes());
    f1_data.extend_from_slice(&4u16.to_be_bytes());
    f1_data.extend_from_slice(&1u16.to_be_bytes());

    f1_data.extend_from_slice(&0x1100u16.to_be_bytes()); // Inter strip
    f1_data.extend_from_slice(&29u16.to_be_bytes());
    f1_data.extend_from_slice(&0u16.to_be_bytes());
    f1_data.extend_from_slice(&0u16.to_be_bytes());
    f1_data.extend_from_slice(&4u16.to_be_bytes());
    f1_data.extend_from_slice(&8u16.to_be_bytes());

    // 0x26: 1 grayscale vector: Y=[200, 200, 200, 200]
    f1_data.push(0x26);
    f1_data.extend_from_slice(&8u32.to_be_bytes()[1..4]);
    f1_data.extend_from_slice(&[200, 200, 200, 200]);

    // 0x31: inter vectors: block 0 skipped, block 1 updated with vector 0
    f1_data.push(0x31);
    f1_data.extend_from_slice(&9u32.to_be_bytes()[1..4]);
    f1_data.extend_from_slice(&0x4000_0000u32.to_be_bytes());
    f1_data.push(0);

    let f1 = decoder.decode(&f1_data).expect("decode f1");
    let u1 = f1.u.as_ref().expect("u plane");
    let v1 = f1.v.as_ref().expect("v plane");

    // Block 0 was skipped: keeps original color chroma
    assert_eq!(u1[0], 178);
    assert_eq!(v1[0], 88);

    // Block 1 was replaced by grayscale vector: chroma must be neutralized to 128
    assert_eq!(u1[2], 128);
    assert_eq!(v1[2], 128);
    assert_eq!(u1[3], 128);
    assert_eq!(v1[3], 128);
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

    // 24,000 x 24,000 passes 5 * area (2.88 GB) but exceeds 4 GiB budget once all live staging pictures are accounted for
    let dims_24k = [0x00, 0, 0, 10, 0x5D, 0xC0, 0x5D, 0xC0, 0, 1];
    assert!(matches!(decoder.decode(&dims_24k), Err(Error::MemoryBudget { width: 24000, height: 24000 })));

    // Interframe before keyframe (inter strip 0x1100)
    let inter_first = [0x01, 0, 0, 22, 0, 16, 0, 16, 0, 1, 0x11, 0x00, 0, 12, 0, 0, 0, 0, 0, 16, 0, 16];
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

#[test]
fn test_intra_strip_with_flag_one_decodes_on_fresh_decoder() {
    // Packet with flags = 0x01, but an intra strip (0x1000). Fresh decoder must decode without MissingReference.
    let mut data = Vec::new();
    let total_len = 37u32;
    let strip_len = 27u16;

    // Frame header with flag 0x01
    data.push(0x01);
    data.extend_from_slice(&total_len.to_be_bytes()[1..4]);
    data.extend_from_slice(&4u16.to_be_bytes()); // Width 4
    data.extend_from_slice(&4u16.to_be_bytes()); // Height 4
    data.extend_from_slice(&1u16.to_be_bytes()); // 1 strip

    // Strip header: intra (0x1000)
    data.extend_from_slice(&0x1000u16.to_be_bytes());
    data.extend_from_slice(&strip_len.to_be_bytes());
    data.extend_from_slice(&0u16.to_be_bytes());
    data.extend_from_slice(&0u16.to_be_bytes());
    data.extend_from_slice(&4u16.to_be_bytes());
    data.extend_from_slice(&4u16.to_be_bytes());

    // Subchunk 0x22 (V1 color full): green (Y=144, U=-72, V=-72)
    data.push(0x22);
    data.extend_from_slice(&10u32.to_be_bytes()[1..4]);
    data.extend_from_slice(&[144, 144, 144, 144, (-72i8) as u8, (-72i8) as u8]);

    // Subchunk 0x32 (vector)
    data.push(0x32);
    data.extend_from_slice(&5u32.to_be_bytes()[1..4]);
    data.push(0);

    let mut decoder = Decoder::new();
    let frame = decoder.decode(&data).expect("intra strip with flag 1 must succeed on fresh decoder");
    assert!(frame.key);
    assert_eq!(frame.pixel_rgb(0, 0).expect("pixel"), [0, 252, 0]);
}

#[test]
fn test_strip_codebook_inheritance() {
    // 4x8 frame with 2 strips (4x4 each).
    // flags = 0x01 (can_inherit enabled).
    // Strip 0 defines a V1 red codebook and vectors.
    // Strip 1 has NO codebook subchunk, and inherits Strip 0's codebook.
    let mut data = Vec::new();
    let strip0_len = 12 + 10 + 5; // 27 bytes
    let strip1_len = 12 + 5; // 17 bytes (no 0x22 codebook, only vector!)
    let total_len = (10 + strip0_len + strip1_len) as u32; // 54 bytes

    data.push(0x01); // can_inherit
    data.extend_from_slice(&total_len.to_be_bytes()[1..4]);
    data.extend_from_slice(&4u16.to_be_bytes()); // Width 4
    data.extend_from_slice(&8u16.to_be_bytes()); // Height 8
    data.extend_from_slice(&2u16.to_be_bytes()); // 2 strips

    // Strip 0: intra (0x1000)
    data.extend_from_slice(&0x1000u16.to_be_bytes());
    data.extend_from_slice(&(strip0_len as u16).to_be_bytes());
    data.extend_from_slice(&0u16.to_be_bytes());
    data.extend_from_slice(&0u16.to_be_bytes());
    data.extend_from_slice(&4u16.to_be_bytes());
    data.extend_from_slice(&4u16.to_be_bytes());
    // Strip 0 codebook (V1 red: Y=72, U=-37, V=91)
    data.push(0x22);
    data.extend_from_slice(&10u32.to_be_bytes()[1..4]);
    data.extend_from_slice(&[72, 72, 72, 72, (-37i8) as u8, 91u8]);
    // Strip 0 vector 0
    data.push(0x32);
    data.extend_from_slice(&5u32.to_be_bytes()[1..4]);
    data.push(0);

    // Strip 1: intra (0x1000) with y0=4..8
    data.extend_from_slice(&0x1000u16.to_be_bytes());
    data.extend_from_slice(&(strip1_len as u16).to_be_bytes());
    data.extend_from_slice(&4u16.to_be_bytes());
    data.extend_from_slice(&0u16.to_be_bytes());
    data.extend_from_slice(&8u16.to_be_bytes());
    data.extend_from_slice(&4u16.to_be_bytes());
    // Strip 1 has NO codebook: uses inherited vector 0 from Strip 0
    data.push(0x32);
    data.extend_from_slice(&5u32.to_be_bytes()[1..4]);
    data.push(0);

    let mut decoder = Decoder::new();
    let frame = decoder.decode(&data).expect("strip 1 must inherit strip 0 codebook");
    assert_eq!(frame.width, 4);
    assert_eq!(frame.height, 8);

    // Both strip 0 (y=0..4) and strip 1 (y=4..8) must be red
    for y in 0..8 {
        for x in 0..4 {
            let px = frame.pixel_rgb(x, y).expect("pixel");
            assert!(px[0] > 200 && px[1] == 0 && px[2] == 0, "mismatch at ({x}, {y})");
        }
    }
}

#[test]
fn test_decode_error_leaves_decoder_state_unchanged() {
    let mut decoder = Decoder::new();

    // 1. Decode a valid frame: 4x4 red
    let f0 = build_synthetic_v1_keyframe([72, 72, 72, 72], -37, 91);
    let frame0 = decoder.decode(&f0).expect("decode f0");
    let px0 = frame0.pixel_rgb(0, 0).expect("pixel");
    assert!(px0[0] > 200);

    // 2. Attempt to decode a corrupted frame that claims 2 strips but cuts off inside strip 1
    let mut corrupt = Vec::new();
    corrupt.push(0x01);
    let total_len = 50u32;
    corrupt.extend_from_slice(&total_len.to_be_bytes()[1..4]);
    corrupt.extend_from_slice(&4u16.to_be_bytes());
    corrupt.extend_from_slice(&4u16.to_be_bytes());
    corrupt.extend_from_slice(&2u16.to_be_bytes()); // 2 strips
    // Strip 0: valid 27 bytes (blue codebook)
    corrupt.extend_from_slice(&0x1000u16.to_be_bytes());
    corrupt.extend_from_slice(&27u16.to_be_bytes());
    corrupt.extend_from_slice(&0u16.to_be_bytes());
    corrupt.extend_from_slice(&0u16.to_be_bytes());
    corrupt.extend_from_slice(&4u16.to_be_bytes());
    corrupt.extend_from_slice(&4u16.to_be_bytes());
    corrupt.push(0x22);
    corrupt.extend_from_slice(&10u32.to_be_bytes()[1..4]);
    corrupt.extend_from_slice(&[72, 72, 72, 72, 91u8, (-37i8) as u8]);
    corrupt.push(0x32);
    corrupt.extend_from_slice(&5u32.to_be_bytes()[1..4]);
    corrupt.push(0);
    // Strip 1: declared size 20, but we stop early
    corrupt.extend_from_slice(&0x1000u16.to_be_bytes());
    corrupt.extend_from_slice(&20u16.to_be_bytes());

    let err = decoder.decode(&corrupt);
    assert!(matches!(err, Err(Error::Truncated)));

    // 3. Now decode an interframe referencing frame 0.
    // Strip 0 codebook should still be red (not blue from the aborted frame).
    let mut f2 = Vec::new();
    f2.push(0x01);
    let f2_strip_len = 17u16; // 12 header + 5 vector
    let f2_total = (10 + f2_strip_len) as u32;
    f2.extend_from_slice(&f2_total.to_be_bytes()[1..4]);
    f2.extend_from_slice(&4u16.to_be_bytes());
    f2.extend_from_slice(&4u16.to_be_bytes());
    f2.extend_from_slice(&1u16.to_be_bytes());
    f2.extend_from_slice(&0x1100u16.to_be_bytes()); // inter strip
    f2.extend_from_slice(&f2_strip_len.to_be_bytes());
    f2.extend_from_slice(&0u16.to_be_bytes());
    f2.extend_from_slice(&0u16.to_be_bytes());
    f2.extend_from_slice(&4u16.to_be_bytes());
    f2.extend_from_slice(&4u16.to_be_bytes());
    // Re-use vector 0
    f2.push(0x32);
    f2.extend_from_slice(&5u32.to_be_bytes()[1..4]);
    f2.push(0);

    let frame2 = decoder.decode(&f2).expect("decode f2");
    let px2 = frame2.pixel_rgb(0, 0).expect("pixel");
    assert!(px2[0] > 200 && px2[2] == 0, "codebook should still be red, got {px2:?}");
}

#[test]
fn test_keyframe_decode_error_preserves_reference_frame() {
    let mut decoder = Decoder::new();

    // 1. Decode a valid initial keyframe: 4x4 red
    let f0 = build_synthetic_v1_keyframe([72, 72, 72, 72], -37, 91);
    let frame0 = decoder.decode(&f0).expect("decode f0");
    assert!(frame0.pixel_rgb(0, 0).expect("pixel")[0] > 200);

    // 2. Attempt to decode a keyframe whose headers are valid but fails during chunk decoding
    let mut bad_key = Vec::new();
    bad_key.push(0x00); // keyframe, no inherit
    let bad_strip_len = 16u16;
    let bad_total = (10 + bad_strip_len) as u32;
    bad_key.extend_from_slice(&bad_total.to_be_bytes()[1..4]);
    bad_key.extend_from_slice(&4u16.to_be_bytes()); // width 4
    bad_key.extend_from_slice(&4u16.to_be_bytes()); // height 4
    bad_key.extend_from_slice(&1u16.to_be_bytes()); // 1 strip
    bad_key.extend_from_slice(&0x1000u16.to_be_bytes()); // intra strip 0x1000
    bad_key.extend_from_slice(&bad_strip_len.to_be_bytes());
    bad_key.extend_from_slice(&0u16.to_be_bytes());
    bad_key.extend_from_slice(&0u16.to_be_bytes());
    bad_key.extend_from_slice(&4u16.to_be_bytes());
    bad_key.extend_from_slice(&4u16.to_be_bytes());
    // Chunk with invalid size (2 < 4) triggers Error::InvalidChunk during decode
    bad_key.push(0x20);
    bad_key.extend_from_slice(&2u32.to_be_bytes()[1..4]);

    let err = decoder.decode(&bad_key);
    assert!(matches!(err, Err(Error::InvalidChunk(_))));

    // 3. Decoder must still retain frame0 as reference.
    // Decode an inter-frame referencing frame0.
    let mut f2 = Vec::new();
    f2.push(0x01);
    let f2_strip_len = 17u16;
    let f2_total = (10 + f2_strip_len) as u32;
    f2.extend_from_slice(&f2_total.to_be_bytes()[1..4]);
    f2.extend_from_slice(&4u16.to_be_bytes());
    f2.extend_from_slice(&4u16.to_be_bytes());
    f2.extend_from_slice(&1u16.to_be_bytes());
    f2.extend_from_slice(&0x1100u16.to_be_bytes()); // inter strip
    f2.extend_from_slice(&f2_strip_len.to_be_bytes());
    f2.extend_from_slice(&0u16.to_be_bytes());
    f2.extend_from_slice(&0u16.to_be_bytes());
    f2.extend_from_slice(&4u16.to_be_bytes());
    f2.extend_from_slice(&4u16.to_be_bytes());
    f2.push(0x32);
    f2.extend_from_slice(&5u32.to_be_bytes()[1..4]);
    f2.push(0);

    let frame2 = decoder.decode(&f2).expect("decode f2 after failed keyframe");
    let px2 = frame2.pixel_rgb(0, 0).expect("pixel");
    assert!(px2[0] > 200 && px2[2] == 0, "previous reference must be preserved");
}

/// One intra strip over rows `y0..y1` of a 4-wide frame: an optional full V1 colour codebook, then a
/// V1-only vector chunk selecting codebook entry 0.
fn strip_with_codebook(y0: u16, y1: u16, codebook: Option<([u8; 4], i8, i8)>) -> Vec<u8> {
    let mut body = Vec::new();
    if let Some((y, u, v)) = codebook {
        body.push(0x22);
        body.extend_from_slice(&10u32.to_be_bytes()[1..4]);
        body.extend_from_slice(&y);
        body.push(u as u8);
        body.push(v as u8);
    }
    body.push(0x32);
    body.extend_from_slice(&5u32.to_be_bytes()[1..4]);
    body.push(0);
    let mut strip = Vec::new();
    strip.extend_from_slice(&0x1000u16.to_be_bytes());
    strip.extend_from_slice(&((12 + body.len()) as u16).to_be_bytes());
    for v in [y0, 0, y1, 4] {
        strip.extend_from_slice(&v.to_be_bytes());
    }
    strip.extend_from_slice(&body);
    strip
}

fn frame_of(flags: u8, height: u16, strips: &[Vec<u8>]) -> Vec<u8> {
    let total = 10 + strips.iter().map(Vec::len).sum::<usize>();
    let mut data = vec![flags];
    data.extend_from_slice(&(total as u32).to_be_bytes()[1..4]);
    data.extend_from_slice(&4u16.to_be_bytes());
    data.extend_from_slice(&height.to_be_bytes());
    data.extend_from_slice(&(strips.len() as u16).to_be_bytes());
    for s in strips {
        data.extend_from_slice(s);
    }
    data
}

/// The first strip of a frame inherits the codebooks of the previous frame's last strip, not its
/// first.
#[test]
fn test_first_strip_inherits_the_last_strip_of_the_previous_frame() {
    let red = ([72, 72, 72, 72], -37, 91);
    let green = ([150, 150, 150, 150], -40, -50);
    let want_red = decode_frame(&build_synthetic_v1_keyframe(red.0, red.1, red.2)).expect("red").pixel_rgb(0, 0).expect("px");
    let want_green = decode_frame(&build_synthetic_v1_keyframe(green.0, green.1, green.2)).expect("green").pixel_rgb(0, 0).expect("px");
    assert_ne!(want_red, want_green);

    let mut decoder = Decoder::new();
    // Frame 0: the first strip carries a red codebook, the last strip a green one.
    let f0 = frame_of(0, 8, &[strip_with_codebook(0, 4, Some(red)), strip_with_codebook(4, 8, Some(green))]);
    let frame0 = decoder.decode(&f0).expect("frame 0");
    assert_eq!(frame0.pixel_rgb(0, 0).expect("px"), want_red);
    assert_eq!(frame0.pixel_rgb(0, 7).expect("px"), want_green);

    // Frame 1: one strip, inheriting, with no codebook of its own.
    let f1 = frame_of(1, 4, &[strip_with_codebook(0, 4, None)]);
    let frame1 = decoder.decode(&f1).expect("frame 1");
    assert_eq!(frame1.pixel_rgb(0, 0).expect("px"), want_green);
}

#[test]
fn test_grayscale_strip_uses_palette_after_color_frame() {
    let mut palette = [0u8; 768];
    // Palette index 1 is bright blue: R=10, G=20, B=230
    palette[3] = 10;
    palette[4] = 20;
    palette[5] = 230;

    let mut decoder = Decoder::with_palette(palette);

    // Frame 0: color frame (red)
    let red = ([72, 72, 72, 72], -37, 91);
    let f0 = frame_of(0, 4, &[strip_with_codebook(0, 4, Some(red))]);
    let frame0 = decoder.decode(&f0).expect("frame 0");
    let px0 = frame0.pixel_rgb(0, 0).expect("px");
    assert!(px0[0] > 200 && px0[2] == 0);

    // Frame 1: grayscale/paletted codebook (subchunk 0x26) with palette index 1
    let mut gray_body = Vec::new();
    gray_body.push(0x26); // v1 gray full
    gray_body.extend_from_slice(&8u32.to_be_bytes()[1..4]); // size 8 (4-byte header + 4 bytes payload)
    gray_body.extend_from_slice(&[1, 1, 1, 1]); // palette index 1
    gray_body.push(0x32); // v1 vector
    gray_body.extend_from_slice(&5u32.to_be_bytes()[1..4]);
    gray_body.push(0);

    let mut strip = Vec::new();
    strip.extend_from_slice(&0x1000u16.to_be_bytes()); // intra
    strip.extend_from_slice(&((12 + gray_body.len()) as u16).to_be_bytes());
    for v in [0u16, 0, 4, 4] {
        strip.extend_from_slice(&v.to_be_bytes());
    }
    strip.extend_from_slice(&gray_body);

    let f1 = frame_of(0, 4, &[strip]);
    let frame1 = decoder.decode(&f1).expect("frame 1");
    // Must be rendered with the configured palette index 1: [10, 20, 230]
    assert_eq!(frame1.pixel_rgb(0, 0).expect("px"), [10, 20, 230]);
}

#[test]
fn test_frame_pixel_rgb_overflow_returns_none() {
    let mut frame = decode_frame(&build_synthetic_v1_keyframe([50, 50, 50, 50], 0, 0)).expect("frame");
    frame.width = u32::MAX;
    frame.height = u32::MAX;
    assert_eq!(frame.pixel_rgb(0, u32::MAX - 1), None);
}

#[test]
fn test_inter_frame_without_inheritance_flag_preserves_retained_codebook() {
    let red = ([72, 72, 72, 72], -37, 91);
    let mut decoder = Decoder::new();

    // Frame 0: keyframe with red codebook
    let f0 = frame_of(0, 4, &[strip_with_codebook(0, 4, Some(red))]);
    let frame0 = decoder.decode(&f0).expect("frame 0");
    let px0 = frame0.pixel_rgb(0, 0).expect("px");
    assert!(px0[0] > 200 && px0[2] == 0);

    // Frame 1: inter frame (strip ID 0x1100), flags = 0 (no inheritance flag),
    // no codebook update, vector chunk 0x31 that updates block (0, 0) with vector 0.
    let mut inter_body = Vec::new();
    inter_body.push(0x31); // inter vector
    // Vector chunk size: 4 header + 4 flag bytes + 1 byte index = 9 bytes
    inter_body.extend_from_slice(&9u32.to_be_bytes()[1..4]);
    // 0x31 flags: 1 = updated, 0 = V1 (so prefix bits '10' -> 0x80000000)
    inter_body.extend_from_slice(&0x8000_0000u32.to_be_bytes());
    inter_body.push(0); // index 0

    let mut strip = Vec::new();
    strip.extend_from_slice(&0x1100u16.to_be_bytes()); // inter strip
    strip.extend_from_slice(&((12 + inter_body.len()) as u16).to_be_bytes());
    for v in [0u16, 0, 4, 4] {
        strip.extend_from_slice(&v.to_be_bytes());
    }
    strip.extend_from_slice(&inter_body);

    let f1 = frame_of(0, 4, &[strip]); // flags = 0 (can_inherit = false)
    let frame1 = decoder.decode(&f1).expect("frame 1");
    // Vector 0 was retained from Frame 0, so updated block (0, 0) must still be red!
    let px1 = frame1.pixel_rgb(0, 0).expect("px1");
    assert_eq!(px1, px0);
}

#[test]
fn test_discarded_strips_do_not_taint_grayscale_frame_is_color() {
    let red = ([72, 72, 72, 72], -37, 91);
    let mut decoder = Decoder::new();

    // Frame 0: two color strips
    let f0 = frame_of(0, 8, &[strip_with_codebook(0, 4, Some(red)), strip_with_codebook(4, 8, Some(red))]);
    let frame0 = decoder.decode(&f0).expect("frame 0");
    assert!(frame0.is_color);

    // Frame 1: one strip, keyframe, grayscale codebook (0x26), inheritance enabled (flags = 1)
    let mut gray_body = Vec::new();
    gray_body.push(0x26); // v1 gray full
    gray_body.extend_from_slice(&8u32.to_be_bytes()[1..4]);
    gray_body.extend_from_slice(&[50, 50, 50, 50]);
    gray_body.push(0x32); // v1 vector
    gray_body.extend_from_slice(&5u32.to_be_bytes()[1..4]);
    gray_body.push(0);

    let mut strip = Vec::new();
    strip.extend_from_slice(&0x1000u16.to_be_bytes());
    strip.extend_from_slice(&((12 + gray_body.len()) as u16).to_be_bytes());
    for v in [0u16, 0, 4, 4] {
        strip.extend_from_slice(&v.to_be_bytes());
    }
    strip.extend_from_slice(&gray_body);

    let f1 = frame_of(1, 4, &[strip]); // flags = 1 (can_inherit = true)
    let frame1 = decoder.decode(&f1).expect("frame 1");
    // Frame 1 is entirely grayscale and had only 1 strip; discarded second strip from frame 0 must not make frame1.is_color true!
    assert!(!frame1.is_color);
}

#[test]
fn test_decode_with_palette_from_sample_entry_and_codec_params() {
    use craft_media::isobmff::{Brand, CodecConfig, FourCc, Mp4Writer, SampleEntry, TrackConfig, VideoParams, WriteSample, WriterOptions, open};
    use std::io::Cursor;

    let f = FourCc(*b"cvid");
    let mut ct = Vec::new();
    ct.extend_from_slice(&0u32.to_be_bytes()); // ctSeed
    ct.extend_from_slice(&0u16.to_be_bytes()); // ctFlags: sequential
    ct.extend_from_slice(&3u16.to_be_bytes()); // ctSize: 4 entries
    for i in 0..4u16 {
        ct.extend_from_slice(&i.to_be_bytes());
        // QuickTime stores 16-bit colors; high byte is 8-bit color
        ct.extend_from_slice(&((i * 60) << 8).to_be_bytes()); // r: 0, 60, 120, 180
        ct.extend_from_slice(&((i * 40) << 8).to_be_bytes()); // g: 0, 40, 80, 120
        ct.extend_from_slice(&((i * 20) << 8).to_be_bytes()); // b: 0, 20, 40, 60
    }

    let entry = SampleEntry {
        format: f,
        data_reference_index: 1,
        codec: CodecConfig::Cinepak { fourcc: f, color_table_id: 0, color_table: Some(ct) },
        video: Some(VideoParams { width: 4, height: 4, depth: 8, compressor_name: "Cinepak".into(), ..Default::default() }),
        audio: None,
        bitrate: None,
    };

    // 1. Verify SampleEntry provides the palette
    let pal = entry.palette().expect("palette on sample entry");
    assert_eq!(&pal[0..12], &[0, 0, 0, 60, 40, 20, 120, 80, 40, 180, 120, 60]);

    // 2. Create a grayscale/paletted frame referencing palette indices: [1, 2, 3, 1]
    let mut gray_body = Vec::new();
    gray_body.push(0x26); // v1 gray full codebook
    gray_body.extend_from_slice(&8u32.to_be_bytes()[1..4]);
    gray_body.extend_from_slice(&[1, 2, 3, 1]); // vector 0 uses indices 1, 2, 3, 1
    gray_body.push(0x32); // v1 vector
    gray_body.extend_from_slice(&5u32.to_be_bytes()[1..4]);
    gray_body.push(0);

    let mut strip = Vec::new();
    strip.extend_from_slice(&0x1000u16.to_be_bytes());
    strip.extend_from_slice(&((12 + gray_body.len()) as u16).to_be_bytes());
    for v in [0u16, 0, 4, 4] {
        strip.extend_from_slice(&v.to_be_bytes());
    }
    strip.extend_from_slice(&gray_body);
    let frame_data = frame_of(0, 4, &[strip]);

    // 3. Mux into a QuickTime MOV container with Mp4Writer
    let mut writer = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mov)).expect("writer");
    let track_id = writer.add_track(TrackConfig::new(entry, 600)).expect("add_track");
    writer.write_sample(track_id, WriteSample { data: &frame_data, duration: 40, composition_offset: 0, is_sync: true }).expect("write_sample");
    let mov_bytes = writer.finish().expect("finish").into_inner();

    // 4. Demux MOV with open() and inspect track parameters
    let demuxed = open(mov_bytes.as_slice()).expect("open");
    assert_eq!(demuxed.tracks.len(), 1);
    let track = &demuxed.tracks[0];
    let params = track.codec_params();
    let pal_from_params = params.palette.expect("palette in codec_params");
    assert_eq!(&pal_from_params[0..12], &[0, 0, 0, 60, 40, 20, 120, 80, 40, 180, 120, 60]);
    assert_eq!(track.palette().as_ref(), Some(&pal));

    // 5. Decode demuxed sample using Decoder initialized from codec_params palette
    let sample = demuxed.read_sample(mov_bytes.as_slice(), 0, 0).expect("read_sample");
    let mut decoder = Decoder::with_palette_slice(&pal_from_params);
    let frame = decoder.decode(&sample).expect("decode");

    // Top-left 2x2 quadrant (bx=0, by=0) is index 1 -> (60, 40, 20)
    assert_eq!(frame.pixel_rgb(0, 0).expect("pixel"), [60, 40, 20]);
    assert_eq!(frame.pixel_rgb(1, 1).expect("pixel"), [60, 40, 20]);
    // Top-right 2x2 quadrant (bx=2, by=0) is index 2 -> (120, 80, 40)
    assert_eq!(frame.pixel_rgb(2, 0).expect("pixel"), [120, 80, 40]);
    assert_eq!(frame.pixel_rgb(3, 1).expect("pixel"), [120, 80, 40]);
    // Bottom-left 2x2 quadrant (bx=0, by=2) is index 3 -> (180, 120, 60)
    assert_eq!(frame.pixel_rgb(0, 2).expect("pixel"), [180, 120, 60]);
    assert_eq!(frame.pixel_rgb(1, 3).expect("pixel"), [180, 120, 60]);
    // Bottom-right 2x2 quadrant (bx=2, by=2) is index 1 -> (60, 40, 20)
    assert_eq!(frame.pixel_rgb(2, 2).expect("pixel"), [60, 40, 20]);
    assert_eq!(frame.pixel_rgb(3, 3).expect("pixel"), [60, 40, 20]);
}

/// An 8 bpp frame of two 4x4 blocks: a V1 codebook of one vector, and the block updates.
fn indexed_frame(key: bool, vector: u8, blocks: &[u8]) -> Vec<u8> {
    let (strip_id, update): (u16, Vec<u8>) = if key {
        (0x1000, [&[0x32u8][..], &(4 + blocks.len() as u32).to_be_bytes()[1..4], blocks].concat())
    } else {
        (0x1100, [&[0x31u8][..], &(4 + 4 + blocks.len() as u32).to_be_bytes()[1..4], &0x4000_0000u32.to_be_bytes(), blocks].concat())
    };
    let book = [&[0x26u8][..], &8u32.to_be_bytes()[1..4], &[vector; 4]].concat();
    let strip_len = (12 + book.len() + update.len()) as u16;
    let mut f = vec![u8::from(!key)];
    f.extend_from_slice(&(10 + u32::from(strip_len)).to_be_bytes()[1..4]);
    f.extend_from_slice(&[0, 8, 0, 4, 0, 1]); // 8x4, one strip
    f.extend_from_slice(&strip_id.to_be_bytes());
    f.extend_from_slice(&strip_len.to_be_bytes());
    f.extend_from_slice(&[0, 0, 0, 0, 0, 4, 0, 8]);
    f.extend(book);
    f.extend(update);
    f
}

/// A palette change between two frames reaches the blocks an inter frame skips as well as the ones it
/// decodes: no picture mixes two palettes.
#[test]
fn test_palette_change_recolors_the_blocks_an_inter_frame_skips() {
    let palette = |entries: &[(u8, [u8; 3])]| {
        let mut p = [0u8; 768];
        for (i, c) in entries {
            p[usize::from(*i) * 3..usize::from(*i) * 3 + 3].copy_from_slice(c);
        }
        p
    };
    let (red, green, blue) = ([255, 0, 0], [0, 255, 0], [0, 0, 255]);
    let mut decoder = Decoder::with_palette(palette(&[(7, red), (9, green)]));
    let first = decoder.decode(&indexed_frame(true, 7, &[0, 0])).expect("keyframe");
    assert_eq!((first.pixel_rgb(0, 0), first.pixel_rgb(4, 0)), (Some(red), Some(red)));

    // Index 7 is blue now, index 9 still green.
    decoder.set_palette(Some(palette(&[(7, blue), (9, green)])));
    // Block 0 is skipped and keeps index 7; block 1 takes a new vector, index 9.
    let second = decoder.decode(&indexed_frame(false, 9, &[0])).expect("inter frame");
    assert_eq!(second.pixel_rgb(0, 0), Some(blue), "the skipped block is in the new palette");
    assert_eq!(second.pixel_rgb(4, 0), Some(green));
    assert_eq!(second.y[0], 7, "the indices are untouched");

    // Clearing the palette shows the indices as gray levels, skipped blocks included.
    decoder.set_palette(None);
    let third = decoder.decode(&indexed_frame(false, 200, &[0])).expect("inter frame");
    assert_eq!((third.pixel_rgb(0, 0), third.pixel_rgb(4, 0)), (Some([7, 7, 7]), Some([200, 200, 200])));
}
