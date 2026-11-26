//! DSD (DSF and DSDIFF) decoding and decimation tests, including oracle tests against FFmpeg
//! (skipped when ffmpeg is absent, a failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`).

use craft_audio::dsd::{BitOrder, Format, decode, decode_with_factor, encode_dsdiff, encode_dsf, filter::SILENCE_LSBF, read_metadata, read_raw};
use std::process::Command;

/// Generates a test DSD 1-bit stream with various features (silence, alternating tones, noise).
fn make_test_stream(num_bytes: usize, seed: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(num_bytes);
    let mut s = seed as u32;
    for i in 0..num_bytes {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        let byte = match (i / 128) % 4 {
            0 => SILENCE_LSBF, // DSD silence
            1 => 0x55,         // Alternate silence pattern
            2 => (s & 0xFF) as u8,
            _ => {
                if (i / 16) % 2 == 0 {
                    0xFF // full positive level
                } else {
                    0x00 // full negative level
                }
            }
        };
        out.push(byte);
    }
    out
}

#[test]
fn roundtrip_dsf() {
    let sample_rate = 2_822_400;
    let block_size = 4096;
    let ch0 = make_test_stream(8192, 42);
    let ch1 = make_test_stream(8192, 99);
    let sample_count = 8192 * 8;

    let dsf_bytes = encode_dsf(sample_rate, &[&ch0, &ch1], sample_count, block_size).expect("encode_dsf");

    let meta = read_metadata(&dsf_bytes).expect("read_metadata");
    assert_eq!(meta.format, Format::Dsf);
    assert_eq!(meta.sample_rate, sample_rate);
    assert_eq!(meta.channels, 2);
    assert_eq!(meta.sample_count, sample_count);

    let raw = read_raw(&dsf_bytes).expect("read_raw");
    assert_eq!(raw.bit_order, BitOrder::LsbFirst);
    assert_eq!(raw.channel_bytes.len(), 2);
    assert_eq!(raw.channel_bytes[0], ch0);
    assert_eq!(raw.channel_bytes[1], ch1);

    let decoded = decode(&dsf_bytes).expect("decode");
    assert_eq!(decoded.sample_rate, sample_rate / 8);
    assert_eq!(decoded.channels.len(), 2);
    assert_eq!(decoded.channels[0].len(), 8192);
    assert_eq!(decoded.channels[1].len(), 8192);
}

#[test]
fn roundtrip_dsdiff() {
    let sample_rate = 2_822_400;
    let ch0 = make_test_stream(4096, 17);
    let ch1 = make_test_stream(4096, 73);
    let sample_count = 4096 * 8;

    let dff_bytes = encode_dsdiff(sample_rate, &[&ch0, &ch1], sample_count).expect("encode_dsdiff");

    let meta = read_metadata(&dff_bytes).expect("read_metadata");
    assert_eq!(meta.format, Format::Dsdiff);
    assert_eq!(meta.sample_rate, sample_rate);
    assert_eq!(meta.channels, 2);
    assert_eq!(meta.sample_count, sample_count);

    let raw = read_raw(&dff_bytes).expect("read_raw");
    assert_eq!(raw.bit_order, BitOrder::MsbFirst);
    assert_eq!(raw.channel_bytes.len(), 2);
    assert_eq!(raw.channel_bytes[0], ch0);
    assert_eq!(raw.channel_bytes[1], ch1);

    let decoded = decode(&dff_bytes).expect("decode");
    assert_eq!(decoded.sample_rate, sample_rate / 8);
    assert_eq!(decoded.channels.len(), 2);
    assert_eq!(decoded.channels[0].len(), 4096);
    assert_eq!(decoded.channels[1].len(), 4096);
}

#[test]
fn decimation_factors() {
    let sample_rate = 2_822_400;
    let ch0 = make_test_stream(2048, 123);
    let dsf_bytes = encode_dsf(sample_rate, &[&ch0], 2048 * 8, 4096).expect("encode_dsf");

    // Factor 8 -> 352,800 Hz
    let dec8 = decode_with_factor(&dsf_bytes, 8).expect("factor 8");
    assert_eq!(dec8.sample_rate, 352_800);
    assert_eq!(dec8.channels[0].len(), 2048);

    // Factor 16 -> 176,400 Hz
    let dec16 = decode_with_factor(&dsf_bytes, 16).expect("factor 16");
    assert_eq!(dec16.sample_rate, 176_400);
    assert_eq!(dec16.channels[0].len(), 1024);

    // Factor 32 -> 88,200 Hz
    let dec32 = decode_with_factor(&dsf_bytes, 32).expect("factor 32");
    assert_eq!(dec32.sample_rate, 88_200);
    assert_eq!(dec32.channels[0].len(), 512);

    // Factor 64 -> 44,100 Hz
    let dec64 = decode_with_factor(&dsf_bytes, 64).expect("factor 64");
    assert_eq!(dec64.sample_rate, 44_100);
    assert_eq!(dec64.channels[0].len(), 256);

    // Invalid factor
    assert!(decode_with_factor(&dsf_bytes, 12).is_err());
}

#[test]
fn silence_and_dc_levels() {
    let sample_rate = 2_822_400;
    // Pure silence
    let silence = vec![SILENCE_LSBF; 1024];
    let dsf_bytes = encode_dsf(sample_rate, &[&silence], 1024 * 8, 4096).expect("encode");
    let dec = decode(&dsf_bytes).expect("decode");
    for &sample in &dec.channels[0][12..] {
        assert!(sample.abs() < 1e-4, "silence sample should be ~0.0, got {sample}");
    }

    // Full positive level
    let ones = vec![0xFF; 1024];
    let dsf_ones = encode_dsf(sample_rate, &[&ones], 1024 * 8, 4096).expect("encode");
    let dec_ones = decode(&dsf_ones).expect("decode");
    for &sample in &dec_ones.channels[0][12..] {
        assert!((sample - 1.0).abs() < 1e-4, "ones sample should be ~1.0, got {sample}");
    }

    // Full negative level
    let zeros = vec![0x00; 1024];
    let dsf_zeros = encode_dsf(sample_rate, &[&zeros], 1024 * 8, 4096).expect("encode");
    let dec_zeros = decode(&dsf_zeros).expect("decode");
    for &sample in &dec_zeros.channels[0][12..] {
        assert!((sample - (-1.0)).abs() < 1e-4, "zeros sample should be ~-1.0, got {sample}");
    }
}

#[test]
fn multichannel_dsf_and_dsdiff() {
    let sample_rate = 2_822_400;
    // Test 6-channel (5.1 surround)
    let channels: Vec<Vec<u8>> = (0..6).map(|c| make_test_stream(1024, (c * 23 + 5) as u8)).collect();
    let ch_refs: Vec<&[u8]> = channels.iter().map(Vec::as_slice).collect();

    let dsf = encode_dsf(sample_rate, &ch_refs, 1024 * 8, 512).expect("encode 5.1 dsf");
    let dsf_meta = read_metadata(&dsf).expect("meta 5.1 dsf");
    assert_eq!(dsf_meta.channels, 6);
    let dsf_dec = decode(&dsf).expect("decode 5.1 dsf");
    assert_eq!(dsf_dec.channels.len(), 6);
    assert_eq!(dsf_dec.channels[0].len(), 1024);

    let dff = encode_dsdiff(sample_rate, &ch_refs, 1024 * 8).expect("encode 5.1 dsdiff");
    let dff_meta = read_metadata(&dff).expect("meta 5.1 dsdiff");
    assert_eq!(dff_meta.channels, 6);
    let dff_dec = decode(&dff).expect("decode 5.1 dsdiff");
    assert_eq!(dff_dec.channels.len(), 6);
    assert_eq!(dff_dec.channels[0].len(), 1024);
}

#[test]
fn dst_compression_rejected() {
    // Construct a minimal DSDIFF with CMPR = 'DST '
    let mut dff = encode_dsdiff(2_822_400, &[&[0xAA; 64]], 512).expect("encode");
    // Find 'DSD ' in CMPR chunk and replace with 'DST '
    let mut replaced = false;
    for i in 0..dff.len().saturating_sub(8) {
        if &dff[i..i + 4] == b"CMPR" && &dff[i + 12..i + 16] == b"DSD " {
            dff[i + 12..i + 16].copy_from_slice(b"DST ");
            replaced = true;
            break;
        }
    }
    assert!(replaced, "should find CMPR DSD chunk");
    assert!(read_metadata(&dff).is_err(), "DST compression should be rejected as unsupported");
    assert!(decode(&dff).is_err(), "DST compression should be rejected as unsupported");
}

#[test]
fn id3v2_tag_in_dsf() {
    let sample_rate = 2_822_400;
    let mut dsf = encode_dsf(sample_rate, &[&[0x69; 128]], 1024, 512).expect("encode");
    let metadata_offset = dsf.len() as u64;

    // Append simple ID3v2.3 tag with TIT2="Test Title" and TPE1="Test Artist"
    let mut id3 = Vec::new();
    id3.extend_from_slice(b"ID3");
    id3.push(3); // v2.3
    id3.push(0); // revision
    id3.push(0); // flags

    let mut frames = Vec::new();
    // TIT2 frame
    frames.extend_from_slice(b"TIT2");
    let title_bytes = b"\x03DSD Master"; // UTF-8
    frames.extend_from_slice(&(title_bytes.len() as u32).to_be_bytes());
    frames.extend_from_slice(&[0, 0]); // flags
    frames.extend_from_slice(title_bytes);

    // TPE1 frame
    frames.extend_from_slice(b"TPE1");
    let artist_bytes = b"\x03The Audiophiles";
    frames.extend_from_slice(&(artist_bytes.len() as u32).to_be_bytes());
    frames.extend_from_slice(&[0, 0]); // flags
    frames.extend_from_slice(artist_bytes);

    // synchsafe size
    let tag_len = frames.len();
    id3.push(((tag_len >> 21) & 0x7F) as u8);
    id3.push(((tag_len >> 14) & 0x7F) as u8);
    id3.push(((tag_len >> 7) & 0x7F) as u8);
    id3.push((tag_len & 0x7F) as u8);
    id3.extend_from_slice(&frames);

    dsf.extend_from_slice(&id3);
    // Update file size and metadata_offset of 'DSD ' chunk
    let file_size = dsf.len() as u64;
    dsf[12..20].copy_from_slice(&file_size.to_le_bytes());
    dsf[20..28].copy_from_slice(&metadata_offset.to_le_bytes());

    let meta = read_metadata(&dsf).expect("read metadata with ID3");
    assert_eq!(meta.title.as_deref(), Some("DSD Master"));
    assert_eq!(meta.artist.as_deref(), Some("The Audiophiles"));
}

#[test]
fn raw_audio_bit_order_conversion() {
    let ch0 = vec![0b00000001, 0b10000000];
    let dsf = encode_dsf(2_822_400, &[&ch0], 16, 512).expect("encode");
    let mut raw = read_raw(&dsf).expect("read raw");
    assert_eq!(raw.bit_order, BitOrder::LsbFirst);
    assert_eq!(raw.channel_bytes[0], ch0);

    raw.convert_bit_order(BitOrder::MsbFirst);
    assert_eq!(raw.bit_order, BitOrder::MsbFirst);
    assert_eq!(raw.channel_bytes[0], vec![0b10000000, 0b00000001]);
}

#[test]
fn hostile_inputs() {
    // Truncated headers
    assert!(read_metadata(b"").is_err());
    assert!(read_metadata(b"DSD ").is_err());
    assert!(read_metadata(b"FRM8").is_err());

    // Bad magic
    assert!(read_metadata(b"RIFF\x00\x00\x00\x00WAVE").is_err());

    // Truncated DSF chunk
    let mut bad_dsf = encode_dsf(2_822_400, &[&[0x69; 100]], 800, 4096).expect("encode");
    bad_dsf.truncate(50);
    assert!(decode(&bad_dsf).is_err());

    // Corrupted fmt chunk size
    let mut bad_fmt = encode_dsf(2_822_400, &[&[0x69; 100]], 800, 4096).expect("encode");
    bad_fmt[32] = 0; // bits_per_sample = 0
    assert!(decode(&bad_fmt).is_err());

    // Absurd channels
    let mut bad_ch = encode_dsf(2_822_400, &[&[0x69; 100]], 800, 4096).expect("encode");
    bad_ch[28 + 24] = 99; // channel_count = 99
    assert!(decode(&bad_ch).is_err());
}

#[test]
fn ffmpeg_dsf_oracle() {
    let Some(ffmpeg) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("dsd");

    let sample_rate = 2_822_400;
    let block_size = 4096;
    let ch0 = make_test_stream(16384, 11);
    let ch1 = make_test_stream(16384, 88);
    let sample_count = 16384 * 8;

    let dsf_bytes = encode_dsf(sample_rate, &[&ch0, &ch1], sample_count, block_size).expect("encode_dsf");
    let dsf_path = dir.join("test_oracle.dsf");
    std::fs::write(&dsf_path, &dsf_bytes).expect("write fixture");

    let raw_out_path = dir.join("ffmpeg_dsf.raw");
    let status = Command::new(&ffmpeg).arg("-y").arg("-i").arg(&dsf_path).arg("-f").arg("f32le").arg(&raw_out_path).output().expect("run ffmpeg");
    assert!(status.status.success(), "ffmpeg failed: {}", String::from_utf8_lossy(&status.stderr));

    let raw_bytes = std::fs::read(&raw_out_path).expect("read raw");
    let ffmpeg_floats: Vec<f32> = raw_bytes.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();

    let decoded = decode(&dsf_bytes).expect("our decode");
    assert_eq!(decoded.channels.len(), 2);
    assert_eq!(decoded.channels[0].len(), 16384);
    assert_eq!(decoded.channels[1].len(), 16384);

    // FFmpeg interleaves L0, R0, L1, R1...
    // Compare samples after filter startup (sample 12 onwards).
    for i in 12..16000 {
        let ff_l = ffmpeg_floats[i * 2];
        let ff_r = ffmpeg_floats[i * 2 + 1];
        let our_l = decoded.channels[0][i];
        let our_r = decoded.channels[1][i];

        assert!((ff_l - our_l).abs() < 1e-4, "sample {i} left mismatch: ffmpeg={ff_l}, ours={our_l}");
        assert!((ff_r - our_r).abs() < 1e-4, "sample {i} right mismatch: ffmpeg={ff_r}, ours={our_r}");
    }
}

#[test]
fn ffmpeg_dsdiff_oracle() {
    let Some(ffmpeg) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("dsd");

    let sample_rate = 2_822_400;
    let ch0 = make_test_stream(8192, 33);
    let ch1 = make_test_stream(8192, 77);
    let sample_count = 8192 * 8;

    let dff_bytes = encode_dsdiff(sample_rate, &[&ch0, &ch1], sample_count).expect("encode_dsdiff");
    let dff_path = dir.join("test_oracle.dff");
    std::fs::write(&dff_path, &dff_bytes).expect("write fixture");

    let raw_out_path = dir.join("ffmpeg_dff.raw");
    let status = Command::new(&ffmpeg).arg("-y").arg("-i").arg(&dff_path).arg("-f").arg("f32le").arg(&raw_out_path).output().expect("run ffmpeg");
    assert!(status.status.success(), "ffmpeg failed: {}", String::from_utf8_lossy(&status.stderr));

    let raw_bytes = std::fs::read(&raw_out_path).expect("read raw");
    let ffmpeg_floats: Vec<f32> = raw_bytes.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();

    let decoded = decode(&dff_bytes).expect("our decode");
    assert_eq!(decoded.channels.len(), 2);
    assert_eq!(decoded.channels[0].len(), 8192);

    for i in 12..8000 {
        let ff_l = ffmpeg_floats[i * 2];
        let ff_r = ffmpeg_floats[i * 2 + 1];
        let our_l = decoded.channels[0][i];
        let our_r = decoded.channels[1][i];

        assert!((ff_l - our_l).abs() < 1e-4, "sample {i} left mismatch: ffmpeg={ff_l}, ours={our_l}");
        assert!((ff_r - our_r).abs() < 1e-4, "sample {i} right mismatch: ffmpeg={ff_r}, ours={our_r}");
    }
}
