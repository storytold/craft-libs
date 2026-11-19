//! ITU-T G.722 SB-ADPCM tests and FFmpeg oracle comparisons.
//!
//! Skipped when FFmpeg is not found, unless `CRAFT_MEDIA_REQUIRE_ORACLES=1` is set.

use craft_audio::g722::{Decoder, Encoder, Error, decode, decode_interleaved, encode, encode_interleaved};
use std::process::Command;

fn generate_signal(samples: usize, freq: f64) -> Vec<i16> {
    (0..samples)
        .map(|i| {
            let t = i as f64 / 16000.0;
            let val = (2.0 * std::f64::consts::PI * freq * t).sin() * 10000.0;
            val as i16
        })
        .collect()
}

#[test]
fn test_silence_roundtrip() {
    let pcm = vec![0i16; 640];
    let encoded = encode(&pcm).expect("encode");
    assert_eq!(encoded.len(), 320);
    let decoded = decode(&encoded).expect("decode");
    assert_eq!(decoded.len(), 640);
    for &s in &decoded[40..] {
        assert!(s.abs() <= 6, "silence sample out of bounds: {s}");
    }
}

#[test]
fn test_sine_wave_roundtrip_fidelity() {
    let pcm = generate_signal(1600, 1000.0);
    let encoded = encode(&pcm).expect("encode");
    let decoded = decode(&encoded).expect("decode");
    assert_eq!(decoded.len(), pcm.len());

    // Compute normalized cross-correlation accounting for filter group delay (approx 22 samples)
    let delay = 22;
    let mut num = 0.0;
    let mut den_orig = 0.0;
    let mut den_dec = 0.0;
    for i in 0..(pcm.len() - delay) {
        let o = pcm[i] as f64;
        let d = decoded[i + delay] as f64;
        num += o * d;
        den_orig += o * o;
        den_dec += d * d;
    }
    let corr = num / (den_orig.sqrt() * den_dec.sqrt());
    assert!(corr > 0.98, "reconstructed sine wave correlation too low: {corr}");
}

#[test]
fn test_stereo_multichannel_roundtrip() {
    let left = generate_signal(800, 440.0);
    let right = generate_signal(800, 880.0);
    let mut interleaved = Vec::with_capacity(1600);
    for i in 0..800 {
        interleaved.push(left[i]);
        interleaved.push(right[i]);
    }
    let encoded = encode_interleaved(&interleaved, 2).expect("encode stereo");
    assert_eq!(encoded.len(), 800);
    let decoded = decode_interleaved(&encoded, 2).expect("decode stereo");
    assert_eq!(decoded.len(), 1600);
}

#[test]
fn test_hostile_inputs_do_not_crash() {
    let garbage = [0xFF, 0x00, 0xAA, 0x55, 0x12, 0x34, 0x89, 0xFE];
    let decoded = decode(&garbage).expect("decode garbage");
    assert_eq!(decoded.len(), 16);

    assert_eq!(encode(&[1, 2, 3]), Err(Error::OddSampleCount(3)));
    assert_eq!(decode_interleaved(&[0; 8], 0), Err(Error::InvalidChannels(0)));
    assert_eq!(decode_interleaved(&[0; 8], 9), Err(Error::InvalidChannels(9)));
}

#[test]
fn test_encoder_and_decoder_reset() {
    let mut enc = Encoder::new();
    let mut dec = Decoder::new();
    let pcm = generate_signal(200, 1000.0);

    let mut out1 = Vec::new();
    enc.encode(&pcm, &mut out1).expect("enc 1");
    let mut dec1 = Vec::new();
    dec.decode(&out1, &mut dec1).expect("dec 1");

    enc.reset();
    dec.reset();

    let mut out2 = Vec::new();
    enc.encode(&pcm, &mut out2).expect("enc 2");
    let mut dec2 = Vec::new();
    dec.decode(&out2, &mut dec2).expect("dec 2");

    assert_eq!(out1, out2);
    assert_eq!(dec1, dec2);
}

#[test]
fn test_ffmpeg_g722_oracle_encoder_and_decoder() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("g722_oracle");

    // 1. Generate test 1000 Hz tone at 16 kHz mono (800 samples = 0.05s)
    let pcm = generate_signal(800, 1000.0);
    let pcm_path = dir.join("input.s16le");
    let mut pcm_bytes = Vec::with_capacity(pcm.len() * 2);
    for &s in &pcm {
        pcm_bytes.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(&pcm_path, &pcm_bytes).expect("write pcm");

    // 2. FFmpeg encodes the PCM to G.722
    let ff_g722_path = dir.join("ffmpeg.g722");
    let out = Command::new(&ff)
        .args(["-y", "-f", "s16le", "-ar", "16000", "-ac", "1", "-i"])
        .arg(&pcm_path)
        .args(["-f", "g722"])
        .arg(&ff_g722_path)
        .output()
        .expect("ffmpeg encode");
    assert!(out.status.success(), "ffmpeg encode failed: {}", String::from_utf8_lossy(&out.stderr));
    let ff_g722_bytes = std::fs::read(&ff_g722_path).expect("read ff g722");

    // 3. FFmpeg decodes its own G.722 back to PCM
    let ff_pcm_back_path = dir.join("ff_back.s16le");
    let out = Command::new(&ff)
        .args(["-y", "-f", "g722", "-i"])
        .arg(&ff_g722_path)
        .args(["-f", "s16le", "-ar", "16000", "-ac", "1"])
        .arg(&ff_pcm_back_path)
        .output()
        .expect("ffmpeg decode");
    assert!(out.status.success(), "ffmpeg decode failed: {}", String::from_utf8_lossy(&out.stderr));
    let ff_back_bytes = std::fs::read(&ff_pcm_back_path).expect("read ff back pcm");
    let ff_back_samples: Vec<i16> = ff_back_bytes.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect();

    // 4. Our decoder decodes FFmpeg's G.722 stream
    let our_decoded_from_ff = decode(&ff_g722_bytes).expect("our decode of ff stream");
    assert_eq!(our_decoded_from_ff.len(), ff_back_samples.len());

    // Verify sample-by-sample match against FFmpeg decoder: within ±1 LSB tolerance
    let mut max_diff: i32 = 0;
    for (i, (&ours, &theirs)) in our_decoded_from_ff.iter().zip(&ff_back_samples).enumerate() {
        let diff = (ours as i32 - theirs as i32).abs();
        if diff > max_diff {
            max_diff = diff;
        }
        assert!(diff <= 1, "sample {i} mismatch between our decoder and FFmpeg decoder: ours {ours}, theirs {theirs}");
    }
    assert!(max_diff <= 1, "max diff {max_diff} exceeded tolerance");

    // 5. Our encoder encodes the PCM to G.722
    let our_encoded_bytes = encode(&pcm).expect("our encode");
    let our_g722_path = dir.join("ours.g722");
    std::fs::write(&our_g722_path, &our_encoded_bytes).expect("write our g722");

    // 6. FFmpeg decodes our encoded G.722 stream
    let our_decoded_by_ff_path = dir.join("our_by_ff.s16le");
    let out = Command::new(&ff)
        .args(["-y", "-f", "g722", "-i"])
        .arg(&our_g722_path)
        .args(["-f", "s16le", "-ar", "16000", "-ac", "1"])
        .arg(&our_decoded_by_ff_path)
        .output()
        .expect("ffmpeg decode of our stream");
    assert!(out.status.success(), "ffmpeg decode of our stream failed: {}", String::from_utf8_lossy(&out.stderr));
    let our_by_ff_bytes = std::fs::read(&our_decoded_by_ff_path).expect("read our by ff pcm");
    let our_by_ff_samples: Vec<i16> = our_by_ff_bytes.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect();

    // Verify that our decoder and FFmpeg decoder produce identical samples on our stream
    let our_decoded_from_ours = decode(&our_encoded_bytes).expect("our decode of our stream");
    assert_eq!(our_decoded_from_ours.len(), our_by_ff_samples.len());
    let mut max_diff_ours: i32 = 0;
    for (i, (&ours, &theirs)) in our_decoded_from_ours.iter().zip(&our_by_ff_samples).enumerate() {
        let diff = (ours as i32 - theirs as i32).abs();
        if diff > max_diff_ours {
            max_diff_ours = diff;
        }
        assert!(diff <= 1, "sample {i} on our stream mismatch between our decoder and FFmpeg decoder: ours {ours}, theirs {theirs}");
    }
    assert!(max_diff_ours <= 1, "max diff {max_diff_ours} exceeded tolerance");

    // Also check high fidelity correlation of our stream decoded by FFmpeg to original signal
    let delay = 22;
    let mut num = 0.0;
    let mut den_orig = 0.0;
    let mut den_dec = 0.0;
    for i in 0..(pcm.len() - delay) {
        let o = pcm[i] as f64;
        let d = our_by_ff_samples[i + delay] as f64;
        num += o * d;
        den_orig += o * o;
        den_dec += d * d;
    }
    let corr = num / (den_orig.sqrt() * den_dec.sqrt());
    assert!(corr > 0.98, "correlation of our encoded stream decoded by FFmpeg is too low: {corr}");
}
