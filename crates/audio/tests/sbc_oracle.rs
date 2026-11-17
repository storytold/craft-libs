#![cfg(feature = "sbc")]

use craft_audio::sbc::{
    AllocationMethod, BlockCount, ChannelMode, Decoder, Encoder, Header, SYNCWORD_MSBC, SYNCWORD_SBC, SamplingFrequency, Subbands, decode, encode,
};
use std::process::Command;

fn pearson_correlation(x: &[i16], y: &[i16], lag: usize) -> f64 {
    let n = x.len().min(y.len().saturating_sub(lag));
    if n == 0 {
        return 0.0;
    }
    let x = &x[..n];
    let y = &y[lag..lag + n];
    let mean_x = x.iter().map(|&v| v as f64).sum::<f64>() / (n as f64);
    let mean_y = y.iter().map(|&v| v as f64).sum::<f64>() / (n as f64);
    let mut num = 0.0;
    let mut denom_x = 0.0;
    let mut denom_y = 0.0;
    for (&a, &b) in x.iter().zip(y) {
        let da = (a as f64) - mean_x;
        let db = (b as f64) - mean_y;
        num += da * db;
        denom_x += da * da;
        denom_y += db * db;
    }
    let denom = (denom_x * denom_y).sqrt();
    if denom > 0.0 { num / denom } else { 0.0 }
}

fn generate_sine(sample_rate: u32, channels: usize, num_frames: usize) -> Vec<i16> {
    let mut pcm = Vec::with_capacity(num_frames * channels);
    for i in 0..num_frames {
        let t = (i as f64) / (sample_rate as f64);
        let s_l = (16000.0 * (2.0 * std::f64::consts::PI * 440.0 * t).sin()) as i16;
        pcm.push(s_l);
        if channels == 2 {
            let s_r = (16000.0 * (2.0 * std::f64::consts::PI * 880.0 * t).sin()) as i16;
            pcm.push(s_r);
        }
    }
    pcm
}

#[test]
fn test_header_parse_and_frame_length() {
    // Standard SBC header: 0x9C, 44.1kHz, 16 blocks, Joint Stereo, Loudness, 8 subbands, bitpool 32
    let mut packet = vec![SYNCWORD_SBC, 0xBD, 32, 0x00];
    let header = Header::parse(&packet).expect("parse header");
    assert_eq!(header.sampling_frequency, SamplingFrequency::Freq44k1);
    assert_eq!(header.blocks, BlockCount::Blocks16);
    assert_eq!(header.channel_mode, ChannelMode::JointStereo);
    assert_eq!(header.allocation_method, AllocationMethod::Loudness);
    assert_eq!(header.subbands, Subbands::Subbands8);
    assert_eq!(header.bitpool, 32);
    assert_eq!(header.channels(), 2);
    assert_eq!(header.samples_per_frame(), 128);

    // Frame length formula check
    let expected_len = 4 + (4 * 8 * 2) / 8 + (8usize + 16 * 32).div_ceil(8);
    assert_eq!(header.frame_length(), expected_len);
    assert_eq!(header.frame_length(), 77);

    // mSBC header: 0xAD
    packet[0] = SYNCWORD_MSBC;
    let msbc_header = Header::parse(&packet).expect("parse msbc header");
    assert_eq!(msbc_header.frame_length(), 57);
    assert_eq!(msbc_header.sample_rate(), 16000);
    assert_eq!(msbc_header.channels(), 1);
}

#[test]
fn test_silence_roundtrip() {
    for &sample_rate in &[16_000u32, 32_000, 44_100, 48_000] {
        for &channels in &[1usize, 2] {
            for &subbands in &[4usize, 8] {
                for &blocks in &[4usize, 8, 12, 16] {
                    let frame_samples = blocks * subbands * channels;
                    let pcm = vec![0i16; frame_samples * 4];
                    let mut encoder = Encoder::new(sample_rate, channels, blocks, subbands, 32, AllocationMethod::Loudness).expect("create encoder");

                    let mut sbc_bytes = Vec::new();
                    encoder.encode(&pcm, &mut sbc_bytes).expect("encode silence");
                    assert!(!sbc_bytes.is_empty());

                    let mut decoder = Decoder::new();
                    let mut decoded = Vec::new();
                    decoder.decode(&sbc_bytes, &mut decoded).expect("decode silence");

                    assert_eq!(decoded.len(), pcm.len());
                    for &s in &decoded {
                        assert!(s.abs() <= 16, "silence sample too large: {s}");
                    }
                }
            }
        }
    }
}

#[test]
fn test_sine_wave_roundtrip_fidelity() {
    let sample_rate = 44_100u32;
    let channels = 2usize;
    let num_frames = 128 * 20;
    let pcm = generate_sine(sample_rate, channels, num_frames);

    let sbc_bytes = encode(&pcm, sample_rate, channels, 32).expect("encode sine");
    let (decoded, info) = decode(&sbc_bytes).expect("decode sine");

    assert_eq!(info.sample_rate, sample_rate);
    assert_eq!(info.channels, channels);
    assert_eq!(decoded.len(), pcm.len());

    // 8-subband filterbank total delay is 73 samples per channel
    let corr_l = pearson_correlation(&pcm[0..], &decoded[0..], 73 * 2);
    assert!(corr_l > 0.98, "left channel correlation too low: {corr_l}");
}

#[test]
fn test_invalid_and_corrupt_inputs() {
    let mut decoder = Decoder::new();
    let mut dst = Vec::new();

    // Empty packet
    assert!(decoder.decode_frame(&[], &mut dst).is_err());

    // Too short header
    assert!(decoder.decode_frame(&[SYNCWORD_SBC, 0x00], &mut dst).is_err());

    // Invalid syncword
    assert!(decoder.decode_frame(&[0x00, 0xBD, 32, 0x00], &mut dst).is_err());

    // CRC mismatch
    let bad_crc_frame = vec![SYNCWORD_SBC, 0xBD, 32, 0x55, 0x00, 0x00, 0x00, 0x00];
    assert!(decoder.decode_frame(&bad_crc_frame, &mut dst).is_err());

    // Bitpool capacity exceeded (Mono 4 subbands capacity is 64; 65 must fail)
    let bad_bitpool_frame = vec![SYNCWORD_SBC, 0x00, 65, 0x00];
    assert!(decoder.decode_frame(&bad_bitpool_frame, &mut dst).is_err());
    assert!(Encoder::new(16000, 1, 4, 4, 65, AllocationMethod::Loudness).is_err());

    // Unsupported 15-block configuration (only 16kHz mono 8-subband 26-bitpool is valid mSBC)
    assert!(Encoder::new(44100, 2, 15, 8, 32, AllocationMethod::Loudness).is_err());

    // Incomplete PCM frames rejected by encoder
    let mut enc = Encoder::new(44100, 2, 16, 8, 32, AllocationMethod::Loudness).expect("enc");
    let mut out = Vec::new();
    assert!(enc.encode(&[0i16; 100], &mut out).is_err());

    // Valid mSBC encoding produces exact 57-byte mSBC frame
    let mut msbc_enc = Encoder::new(16000, 1, 15, 8, 26, AllocationMethod::Loudness).expect("msbc enc");
    let mut msbc_out = Vec::new();
    msbc_enc.encode(&vec![0i16; 120], &mut msbc_out).expect("encode msbc frame");
    assert_eq!(msbc_out.len(), 57);
    assert_eq!(msbc_out[0], SYNCWORD_MSBC);

    // Hostile fuzzed input should never panic
    let mut seed = 0x1234_5678u32;
    for len in [10, 50, 100, 500] {
        let mut random_bytes = vec![0u8; len];
        for b in &mut random_bytes {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *b = seed as u8;
        }
        let _ = decoder.decode(&random_bytes, &mut dst);
    }
}

#[test]
fn test_oracle_ffmpeg_cross_verification() {
    let Some(ffmpeg) = craft_media_testkit::ffmpeg() else {
        return;
    };
    let dir = craft_media_testkit::fixture_subdir("sbc");

    // 1. FFmpeg decodes our encoded SBC files
    for &(sample_rate, channels) in &[(16_000u32, 1usize), (44_100, 2), (48_000, 2)] {
        let pcm = generate_sine(sample_rate, channels, 128 * 20);
        let sbc_bytes = encode(&pcm, sample_rate, channels, 32).expect("encode sbc");

        let sbc_file = dir.join(format!("ours_{sample_rate}_{channels}.sbc"));
        std::fs::write(&sbc_file, &sbc_bytes).expect("write sbc fixture");

        let out = Command::new(&ffmpeg)
            .args(["-v", "error", "-y", "-i"])
            .arg(&sbc_file)
            .args(["-f", "s16le", "-acodec", "pcm_s16le", "-"])
            .output()
            .expect("run ffmpeg");

        assert!(out.status.success(), "ffmpeg failed to decode our SBC: {}", String::from_utf8_lossy(&out.stderr));

        let theirs: Vec<i16> = out.stdout.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();

        assert!(!theirs.is_empty());
        let corr = pearson_correlation(&pcm, &theirs, 73 * channels);
        assert!(corr > 0.95, "FFmpeg decoded output correlation against input too low ({sample_rate} Hz, {channels} ch): {corr}");
    }

    // 2. Our decoder decodes FFmpeg-encoded SBC files
    for &(sample_rate, channels) in &[(16_000u32, 1usize), (44_100, 2), (48_000, 2)] {
        let ff_sbc_file = dir.join(format!("ff_{sample_rate}_{channels}.sbc"));
        let ff_enc = Command::new(&ffmpeg)
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                &format!("sine=frequency=440:duration=0.2:sample_rate={sample_rate}"),
                "-c:a",
                "sbc",
                "-ac",
                &channels.to_string(),
            ])
            .arg(&ff_sbc_file)
            .output()
            .expect("ffmpeg encode sbc");

        assert!(ff_enc.status.success(), "ffmpeg failed to encode SBC: {}", String::from_utf8_lossy(&ff_enc.stderr));

        let ff_sbc_bytes = std::fs::read(&ff_sbc_file).expect("read ff sbc fixture");
        let (decoded, info) = decode(&ff_sbc_bytes).expect("decode ff sbc");
        assert_eq!(info.sample_rate, sample_rate);
        assert_eq!(info.channels, channels);
        assert!(!decoded.is_empty());

        // Decode ff_sbc_file with ffmpeg as well to compare
        let ff_raw = Command::new(&ffmpeg)
            .args(["-v", "error", "-y", "-i"])
            .arg(&ff_sbc_file)
            .args(["-f", "s16le", "-acodec", "pcm_s16le", "-"])
            .output()
            .expect("ffmpeg decode raw");

        let ff_pcm: Vec<i16> = ff_raw.stdout.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();

        let corr = pearson_correlation(&decoded, &ff_pcm, 0);
        assert!(corr > 0.99, "Our decoder output correlation against FFmpeg decoder output too low: {corr}");
    }

    // 3. Our decoder decodes FFmpeg-encoded mSBC wideband speech
    let msbc_file = dir.join("ff_msbc.sbc");
    let ff_msbc_enc = Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=0.2:sample_rate=16000", "-c:a", "sbc", "-msbc", "1"])
        .arg(&msbc_file)
        .output()
        .expect("ffmpeg encode msbc");

    assert!(ff_msbc_enc.status.success(), "ffmpeg failed to encode mSBC: {}", String::from_utf8_lossy(&ff_msbc_enc.stderr));

    let msbc_bytes = std::fs::read(&msbc_file).expect("read msbc fixture");
    let (decoded, info) = decode(&msbc_bytes).expect("decode msbc");
    assert_eq!(info.sample_rate, 16000);
    assert_eq!(info.channels, 1);
    assert_eq!(info.blocks, 15);
    assert_eq!(info.subbands, 8);
    assert!(!decoded.is_empty());

    let ff_msbc_raw = Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-i"])
        .arg(&msbc_file)
        .args(["-f", "s16le", "-acodec", "pcm_s16le", "-"])
        .output()
        .expect("ffmpeg decode msbc raw");

    let ff_msbc_pcm: Vec<i16> = ff_msbc_raw.stdout.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();

    let corr = pearson_correlation(&decoded, &ff_msbc_pcm, 0);
    assert!(corr > 0.99, "mSBC decoded output correlation against FFmpeg decoder too low: {corr}");

    // 4. FFmpeg decodes our encoded mSBC file
    let msbc_sine = generate_sine(16000, 1, 120 * 20);
    let mut our_msbc_enc = Encoder::new(16000, 1, 15, 8, 26, AllocationMethod::Loudness).expect("msbc enc");
    let mut our_msbc_bytes = Vec::new();
    our_msbc_enc.encode(&msbc_sine, &mut our_msbc_bytes).expect("encode msbc");

    let our_msbc_file = dir.join("ours_msbc.sbc");
    std::fs::write(&our_msbc_file, &our_msbc_bytes).expect("write our msbc fixture");

    let our_msbc_out = Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-i"])
        .arg(&our_msbc_file)
        .args(["-f", "s16le", "-acodec", "pcm_s16le", "-"])
        .output()
        .expect("ffmpeg decode our msbc");

    assert!(our_msbc_out.status.success(), "ffmpeg failed to decode our mSBC: {}", String::from_utf8_lossy(&our_msbc_out.stderr));

    let our_msbc_theirs: Vec<i16> = our_msbc_out.stdout.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();

    assert!(!our_msbc_theirs.is_empty());
    let corr_msbc = pearson_correlation(&msbc_sine, &our_msbc_theirs, 73);
    assert!(corr_msbc > 0.95, "FFmpeg decoded output correlation against our mSBC input too low: {corr_msbc}");
}
