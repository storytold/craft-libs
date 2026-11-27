#![cfg(feature = "g726")]

use craft_audio::g726::{BitPacking, BitRate, Decoder, Encoder, decode, decode_interleaved, encode, encode_interleaved};
use std::process::Command;

fn pearson_correlation(x: &[i16], y: &[i16]) -> f64 {
    let n = x.len().min(y.len());
    if n == 0 {
        return 0.0;
    }
    let x = &x[..n];
    let y = &y[..n];
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

fn find_ffmpeg() -> Option<String> {
    for cmd in ["ffmpeg", "/opt/homebrew/bin/ffmpeg", "/usr/bin/ffmpeg"] {
        if Command::new(cmd).arg("-version").output().is_ok() {
            return Some(cmd.to_string());
        }
    }
    None
}

#[test]
fn test_silence_roundtrip() {
    let pcm = vec![0i16; 80];
    for rate in [BitRate::Rate16k, BitRate::Rate24k, BitRate::Rate32k, BitRate::Rate40k] {
        for packing in [BitPacking::BigEndian, BitPacking::LittleEndian] {
            let enc = encode(&pcm, rate, packing).expect("encode");
            let dec = decode(&enc, rate, packing).expect("decode");
            assert_eq!(dec.len(), pcm.len());
            for &s in &dec {
                assert!(s.abs() <= 16, "silence sample too large: {s}");
            }
        }
    }
}

#[test]
fn test_sine_wave_roundtrip_fidelity() {
    let sample_rate = 8000.0;
    let freq = 300.0;
    let num_samples = 160;
    let mut pcm = Vec::with_capacity(num_samples);
    for i in 0..num_samples {
        let t = i as f64 / sample_rate;
        let s = (4000.0 * (2.0 * std::f64::consts::PI * freq * t).sin()) as i16;
        pcm.push(s);
    }

    for rate in [BitRate::Rate16k, BitRate::Rate24k, BitRate::Rate32k, BitRate::Rate40k] {
        for packing in [BitPacking::BigEndian, BitPacking::LittleEndian] {
            let enc = encode(&pcm, rate, packing).expect("encode sine");
            let dec = decode(&enc, rate, packing).expect("decode sine");
            let corr = pearson_correlation(&pcm, &dec);
            assert!(corr > 0.90, "correlation {corr} too low for rate {rate:?} packing {packing:?}");
        }
    }
}

#[test]
fn test_encoder_and_decoder_reset() {
    let mut enc = Encoder::new(BitRate::Rate32k, BitPacking::BigEndian);
    let mut dec = Decoder::new(BitRate::Rate32k, BitPacking::BigEndian);

    let pcm = vec![1000i16, 2000, 3000, 2000, 1000, 0, -1000, -2000];
    let mut out1 = Vec::new();
    enc.encode(&pcm, &mut out1).expect("enc 1");
    enc.flush(&mut out1).expect("flush 1");

    let mut dec_pcm1 = Vec::new();
    dec.decode(&out1, &mut dec_pcm1).expect("dec 1");

    enc.reset();
    dec.reset();

    let mut out2 = Vec::new();
    enc.encode(&pcm, &mut out2).expect("enc 2");
    enc.flush(&mut out2).expect("flush 2");

    let mut dec_pcm2 = Vec::new();
    dec.decode(&out2, &mut dec_pcm2).expect("dec 2");

    assert_eq!(out1, out2);
    assert_eq!(dec_pcm1, dec_pcm2);
}

#[test]
fn test_stereo_multichannel_roundtrip() {
    let mut stereo_pcm = Vec::new();
    for i in 0..160 {
        let left = (3000.0 * (i as f64 * 0.05).sin()) as i16;
        let right = (2000.0 * (i as f64 * 0.03).cos()) as i16;
        stereo_pcm.push(left);
        stereo_pcm.push(right);
    }

    let enc = encode_interleaved(&stereo_pcm, 2, BitRate::Rate32k, BitPacking::BigEndian).expect("encode stereo");
    let dec = decode_interleaved(&enc, 2, BitRate::Rate32k, BitPacking::BigEndian).expect("decode stereo");

    assert_eq!(dec.len(), stereo_pcm.len());
    let corr = pearson_correlation(&stereo_pcm, &dec);
    assert!(corr > 0.90, "stereo correlation {corr} too low");
}

#[test]
fn test_hostile_inputs_do_not_crash() {
    let mut dec = Decoder::new(BitRate::Rate32k, BitPacking::BigEndian);
    let mut out = Vec::new();
    let hostile = vec![0xEE; 1000];
    let _ = dec.decode(&hostile, &mut out);

    let mut enc = Encoder::new(BitRate::Rate40k, BitPacking::LittleEndian);
    let mut out_enc = Vec::new();
    let hostile_pcm = vec![i16::MIN, i16::MAX, -1, 1, 0];
    let _ = enc.encode(&hostile_pcm, &mut out_enc);
    let _ = enc.flush(&mut out_enc);
}

#[test]
fn test_ffmpeg_g726_oracle_encoder_and_decoder() {
    let ff = match find_ffmpeg() {
        Some(f) => f,
        None => return,
    };

    let sample_rate = 8000;
    let num_samples = 240;
    let mut pcm = Vec::with_capacity(num_samples);
    for i in 0..num_samples {
        let t = i as f64 / sample_rate as f64;
        let s = (5000.0 * (2.0 * std::f64::consts::PI * 350.0 * t).sin()) as i16;
        pcm.push(s);
    }

    let dir = std::env::temp_dir().join("craft_g726_oracle");
    let _ = std::fs::create_dir_all(&dir);
    let in_pcm_path = dir.join("input.s16le");
    let mut in_bytes = Vec::with_capacity(pcm.len() * 2);
    for &s in &pcm {
        in_bytes.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(&in_pcm_path, &in_bytes).expect("write input pcm");

    for rate in [BitRate::Rate16k, BitRate::Rate24k, BitRate::Rate32k, BitRate::Rate40k] {
        for (packing, ff_codec, ff_fmt) in [(BitPacking::BigEndian, "g726", "g726"), (BitPacking::LittleEndian, "g726le", "g726le")] {
            let cs = rate.code_size();
            let ff_g726_path = dir.join(format!("ff_{}_{}.g726", cs, ff_codec));

            // 1. FFmpeg encodes PCM to G.726
            let out = Command::new(&ff)
                .args(["-y", "-f", "s16le", "-ar", "8000", "-ac", "1", "-i"])
                .arg(&in_pcm_path)
                .args(["-c:a", ff_codec, "-code_size", &cs.to_string(), "-f", ff_fmt])
                .arg(&ff_g726_path)
                .output()
                .expect("ffmpeg encode");
            assert!(out.status.success(), "ffmpeg encode failed: {}", String::from_utf8_lossy(&out.stderr));
            let ff_g726_bytes = std::fs::read(&ff_g726_path).expect("read ff g726");

            // 2. FFmpeg decodes its own G.726 stream
            let ff_pcm_back_path = dir.join(format!("ff_back_{}_{}.s16le", cs, ff_codec));
            let out = Command::new(&ff)
                .args(["-y", "-f", ff_fmt, "-code_size", &cs.to_string(), "-i"])
                .arg(&ff_g726_path)
                .args(["-f", "s16le", "-ar", "8000", "-ac", "1"])
                .arg(&ff_pcm_back_path)
                .output()
                .expect("ffmpeg decode");
            assert!(out.status.success(), "ffmpeg decode failed: {}", String::from_utf8_lossy(&out.stderr));
            let ff_back_bytes = std::fs::read(&ff_pcm_back_path).expect("read ff back pcm");
            let ff_back_samples: Vec<i16> = ff_back_bytes.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect();

            // 3. Our decoder decodes FFmpeg's stream
            let our_decoded = decode(&ff_g726_bytes, rate, packing).expect("our decode");
            let corr = pearson_correlation(&our_decoded, &ff_back_samples);
            assert!(corr > 0.998, "correlation {corr} too low between our decoder and FFmpeg decoder for rate {rate:?} packing {packing:?}");

            // 4. Our encoder encodes PCM to G.726
            let our_g726_bytes = encode(&pcm, rate, packing).expect("our encode");
            let our_g726_path = dir.join(format!("our_{}_{}.g726", cs, ff_codec));
            std::fs::write(&our_g726_path, &our_g726_bytes).expect("write our g726");

            // 5. FFmpeg decodes our encoded stream
            let our_decoded_by_ff_path = dir.join(format!("our_by_ff_{}_{}.s16le", cs, ff_codec));
            let out = Command::new(&ff)
                .args(["-y", "-f", ff_fmt, "-code_size", &cs.to_string(), "-i"])
                .arg(&our_g726_path)
                .args(["-f", "s16le", "-ar", "8000", "-ac", "1"])
                .arg(&our_decoded_by_ff_path)
                .output()
                .expect("ffmpeg decode of our stream");
            assert!(out.status.success(), "ffmpeg decode of our stream failed: {}", String::from_utf8_lossy(&out.stderr));
            let our_by_ff_bytes = std::fs::read(&our_decoded_by_ff_path).expect("read our by ff pcm");
            let our_by_ff_samples: Vec<i16> = our_by_ff_bytes.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect();

            let corr_input = pearson_correlation(&pcm, &our_by_ff_samples);
            assert!(
                corr_input > 0.90,
                "correlation {corr_input} too low for input vs FFmpeg-decoded our stream for rate {rate:?} packing {packing:?}"
            );
        }
    }
}
