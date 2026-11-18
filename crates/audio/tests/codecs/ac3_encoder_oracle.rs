//! Original synthetic PCM encoded here and decoded by an external FFmpeg executable only.
//! Covers every public acmod/LFE combination at all three rates, plus genuine dependent-stream
//! 7.1. These are framing/routing/level regressions, not a perceptual quality certification.
//! Uses the shared oracle discovery rules; set CRAFT_MEDIA_REQUIRE_ORACLES=1 to require FFmpeg.

use std::f64::consts::TAU;
use std::path::Path;
use std::process::Command;

use craft_audio::ac3::encoder::{Codec, CodingMode, Config, Encoder, Packet};

const INPUT_SAMPLES: usize = 4097;
const ANALYSIS_START: usize = 1024;
const ANALYSIS_SAMPLES: usize = 2048;
const AMPLITUDE: f64 = 0.2;

/// Bit-by-bit polynomial division, independent of the encoder's CRC implementation.
fn crc16(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0u16, |mut remainder, &byte| {
        for bit in (0..8).rev() {
            let feedback = ((remainder >> 15) ^ u16::from((byte >> bit) & 1)) != 0;
            remainder <<= 1;
            if feedback {
                remainder ^= 0x8005;
            }
        }
        remainder
    })
}

fn lfe_index(config: Config) -> Option<usize> {
    config.lfe.then_some(match config.mode {
        CodingMode::Mono => 1,
        CodingMode::ThreeFront | CodingMode::ThreeFrontSurround | CodingMode::ThreeFrontTwoSurround | CodingMode::Surround71 => 3,
        _ => 2,
    })
}

fn run_case(ffmpeg: &Path, config: Config) {
    let label = format!("{:?}/{:?}/lfe={}/{}Hz", config.codec, config.mode, config.lfe, config.sample_rate);
    let channels = config.channels();
    // Integer cycles in the analysis window make the channel probes mutually orthogonal.
    // LFE stays below 47 Hz at all rates, avoiding the encoder's intentional 120 Hz roll-off.
    let bins: Vec<usize> = (0..channels).map(|ch| if Some(ch) == lfe_index(config) { 2 } else { [29, 43, 61, 79, 101, 127, 151, 173][ch] }).collect();
    let source: Vec<Vec<f32>> = bins
        .iter()
        .map(|&bin| {
            (0..INPUT_SAMPLES)
                .map(|i| (AMPLITUDE * (i as f64 / 256.0).min(1.0) * (TAU * bin as f64 * i as f64 / ANALYSIS_SAMPLES as f64).sin()) as f32)
                .collect()
        })
        .collect();
    let mut encoder = Encoder::new(config).unwrap_or_else(|e| panic!("{label}: construct: {e}"));
    let mut packets = encoder.push(&source.iter().map(Vec::as_slice).collect::<Vec<_>>()).unwrap_or_else(|e| panic!("{label}: encode: {e}"));
    let finish = encoder.finish().unwrap_or_else(|e| panic!("{label}: finish: {e}"));
    assert_eq!(finish.priming_samples, 256, "{label}: priming");
    packets.extend(finish.packets);
    let decoded_samples = packets.len() * 1536;
    assert_eq!(decoded_samples, INPUT_SAMPLES + 256 + finish.padding_samples as usize, "{label}: trim accounting");
    let stream = check_packets(&packets, config, &label);
    let pcm = external_decode(ffmpeg, config, &label, &stream, decoded_samples);
    check_tones(&pcm, &bins, config, &label);
}

fn check_packets(packets: &[Packet], config: Config, label: &str) -> Vec<u8> {
    let mut stream = Vec::new();
    for (index, packet) in packets.iter().enumerate() {
        assert_eq!(packet.samples, 1536, "{label}: packet {index} duration");
        let mut remaining = packet.data.as_slice();
        let mut frames = 0;
        while !remaining.is_empty() {
            let header = craft_audio::ac3::parse_header(remaining).unwrap_or_else(|e| panic!("{label}: packet {index} header: {e}"));
            assert_eq!(header.sample_rate, config.sample_rate, "{label}: sample rate");
            assert_eq!(header.samples(), 1536, "{label}: block count");
            assert_eq!(header.is_eac3(), config.codec == Codec::Eac3, "{label}: family");
            assert_eq!(header.substreamid, 0, "{label}: programme id");
            assert_eq!(header.strmtyp, u8::from(frames != 0), "{label}: independent/dependent ordering");
            assert!(header.frame_bytes <= remaining.len(), "{label}: truncated frame");
            let frame = &remaining[..header.frame_bytes];
            assert_eq!(crc16(&frame[2..]), 0, "{label}: packet {index} frame {frames} CRC");
            if config.codec == Codec::Ac3 {
                let boundary = (frame.len() / 2 / 2 + frame.len() / 2 / 8) * 2;
                assert_eq!(crc16(&frame[2..boundary]), 0, "{label}: packet {index} CRC1");
            }
            if config.mode == CodingMode::Surround71 {
                assert_eq!(header.channels(), if frames == 0 { 6 } else { 4 }, "{label}: substream channel count");
            } else {
                assert_eq!(header.acmod, config.mode as u8, "{label}: acmod");
                assert_eq!(header.lfeon, config.lfe, "{label}: LFE flag");
            }
            remaining = &remaining[header.frame_bytes..];
            frames += 1;
        }
        assert_eq!(frames, if config.mode == CodingMode::Surround71 { 2 } else { 1 }, "{label}: complete access unit");
        stream.extend_from_slice(&packet.data);
    }

    stream
}

fn external_decode(ffmpeg: &Path, config: Config, label: &str, stream: &[u8], decoded_samples: usize) -> Vec<f32> {
    let channels = config.channels();
    let format = if config.codec == Codec::Ac3 { "ac3" } else { "eac3" };
    let path = crate::testkit::fixtures_dir("ac3-encoder-oracle").join(format!(
        "{}-{:?}-lfe{}-{}-{}.{}",
        std::process::id(),
        config.mode,
        config.lfe,
        config.sample_rate,
        config.bitrate_kbps,
        format
    ));
    std::fs::write(&path, stream).unwrap_or_else(|e| panic!("{label}: write fixture: {e}"));
    // Explicit format matters for three-frame files, which can be too short for auto-probing.
    // Do not request -ac or -ar: output must retain the bitstream's actual channel count/rate.
    let result = Command::new(ffmpeg)
        .args(["-nostdin", "-v", "error", "-xerror", "-err_detect", "explode", "-f", format, "-i"])
        .arg(&path)
        .args(["-map", "0:a:0", "-c:a", "pcm_f32le", "-f", "f32le", "pipe:1"])
        .output()
        .unwrap_or_else(|e| panic!("{label}: launch FFmpeg: {e}"));
    assert!(result.status.success(), "{label}: FFmpeg failed: {}", String::from_utf8_lossy(&result.stderr));
    assert!(result.stderr.is_empty(), "{label}: FFmpeg reported: {}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(result.stdout.len(), decoded_samples * channels * 4, "{label}: decoded channel/sample count");
    let pcm: Vec<f32> = result.stdout.as_chunks::<4>().0.iter().map(|bytes| f32::from_le_bytes(*bytes)).collect();
    assert!(pcm.iter().all(|v| v.is_finite()), "{label}: non-finite PCM");
    std::fs::remove_file(path).unwrap_or_else(|e| panic!("{label}: remove generated fixture: {e}"));
    pcm
}

fn check_tones(pcm: &[f32], bins: &[usize], config: Config, label: &str) {
    let channels = config.channels();
    for channel in 0..channels {
        for (probe_channel, &bin) in bins.iter().enumerate() {
            let (mut sine, mut cosine) = (0.0, 0.0);
            for i in ANALYSIS_START..ANALYSIS_START + ANALYSIS_SAMPLES {
                let value = f64::from(pcm[(i + 256) * channels + channel]);
                let angle = TAU * bin as f64 * i as f64 / ANALYSIS_SAMPLES as f64;
                sine += value * angle.sin();
                cosine += value * angle.cos();
            }
            // Quadrature measures LFE gain without mistaking the low-pass phase shift for loss.
            let amplitude = 2.0 * sine.hypot(cosine) / ANALYSIS_SAMPLES as f64;
            if channel == probe_channel {
                assert!((0.18..=0.22).contains(&amplitude), "{label}: channel {channel} lost/rerouted/scaled: amplitude {amplitude}");
                if Some(channel) != lfe_index(config) {
                    assert!(sine > 0.0 && cosine.abs() < sine * 0.05, "{label}: channel {channel} polarity/priming alignment");
                }
            } else {
                assert!(amplitude < 0.01, "{label}: channel {channel} contains channel {probe_channel}: amplitude {amplitude}");
            }
        }
    }
}

#[test]
fn ffmpeg_decodes_encoder_modes_rates_and_channels() {
    let ffmpeg = require_ffmpeg!();
    let modes = [
        CodingMode::DualMono,
        CodingMode::Mono,
        CodingMode::Stereo,
        CodingMode::ThreeFront,
        CodingMode::StereoSurround,
        CodingMode::ThreeFrontSurround,
        CodingMode::StereoTwoSurround,
        CodingMode::ThreeFrontTwoSurround,
    ];
    for sample_rate in [32_000, 44_100, 48_000] {
        for codec in [Codec::Ac3, Codec::Eac3] {
            for mode in modes {
                for lfe in [false, true] {
                    // Dual mono's channels are independent programmes 1 and 2, not L/R speakers.
                    run_case(&ffmpeg, Config { codec, sample_rate, bitrate_kbps: 448, mode, lfe });
                }
            }
        }
        run_case(&ffmpeg, Config { codec: Codec::Eac3, sample_rate, bitrate_kbps: 640, mode: CodingMode::Surround71, lfe: true });
    }
}
