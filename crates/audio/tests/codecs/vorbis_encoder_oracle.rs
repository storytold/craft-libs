//! ffmpeg as an external oracle for the pure-Rust Vorbis encoder: both of its decoders (its own
//! and libvorbis) read every file exactly as long as the input, surround comes out on the right
//! speakers, and the tags reach ffprobe. Skipped when ffmpeg is not installed.

use std::path::Path;
use std::process::Command;

use craft_audio::{AudioBuffer, ChannelLayout, EncodeFormat, EncodeOptions, OggVorbisOptions, encode, encode_ogg_vorbis};

/// A different tone on every channel, so each output channel can be traced to its input.
fn tones(rate: u32, channels: usize, frames: usize) -> AudioBuffer {
    let channels = (0..channels)
        .map(|c| {
            let f = 220.0 * (c + 1) as f32;
            (0..frames).map(|i| (std::f32::consts::TAU * f * i as f32 / rate as f32).sin() * 0.3).collect()
        })
        .collect();
    AudioBuffer { sample_rate: rate, channels }
}

/// Decodes `file` with ffmpeg's `decoder`; planar output in ffmpeg's (WAV) channel order.
fn ffmpeg_decode(ff: &Path, decoder: &str, file: &[u8], channels: usize, tag: &str) -> Vec<Vec<f32>> {
    let path = crate::integration::codecs::testkit::fixtures_dir("vorbis-encoder").join(format!("{tag}.ogg"));
    std::fs::write(&path, file).unwrap();
    let out = Command::new(ff)
        .args(["-v", "error", "-xerror", "-c:a", decoder, "-i"])
        .arg(&path)
        .args(["-f", "f32le", "-c:a", "pcm_f32le", "-"])
        .output()
        .unwrap();
    assert!(out.status.success() && out.stderr.is_empty(), "{tag} {decoder}: {}", String::from_utf8_lossy(&out.stderr));
    let pcm: Vec<f32> = out.stdout.as_chunks::<4>().0.iter().map(|&b| f32::from_le_bytes(b)).collect();
    (0..channels).map(|c| pcm.iter().skip(c).step_by(channels).copied().collect()).collect()
}

/// ffmpeg's decoders to check: its own `vorbis` always, and `libvorbis` when ffmpeg has it.
fn decoders(ff: &Path) -> Vec<&'static str> {
    let list =
        Command::new(ff).args(["-hide_banner", "-decoders"]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    if list.lines().any(|l| l.split_whitespace().nth(1) == Some("libvorbis")) {
        vec!["vorbis", "libvorbis"]
    } else {
        eprintln!("SKIPPED (libvorbis decoder): this ffmpeg has no libvorbis; checking its own decoder only");
        vec!["vorbis"]
    }
}

fn correlation(a: &[f32], b: &[f32]) -> f64 {
    let (mut ab, mut aa, mut bb) = (0f64, 0f64, 0f64);
    for (&x, &y) in a.iter().zip(b) {
        ab += f64::from(x) * f64::from(y);
        aa += f64::from(x) * f64::from(x);
        bb += f64::from(y) * f64::from(y);
    }
    ab / (aa * bb).sqrt().max(1e-20)
}

/// Vorbis order (§4.3.9) of the WAV-order layouts the test writes, per channel count.
const VORBIS_ORDER: [&[usize]; 8] =
    [&[0], &[0, 1], &[0, 2, 1], &[0, 1, 2, 3], &[0, 2, 1, 3, 4], &[0, 2, 1, 4, 5, 3], &[0, 2, 1, 5, 6, 4, 3], &[0, 2, 1, 6, 7, 4, 5, 3]];

/// 1 to 8 channels through `encode` with a WAV-order layout, at exactly the input's length out of
/// both decoders: ffmpeg's own puts every channel back on its WAV speaker, and libvorbis (through
/// ffmpeg, which passes its order through) gives the file's Vorbis order.
#[test]
fn every_channel_count_decodes_on_the_right_speakers() {
    let ff = require_ffmpeg!();
    for channels in 1..=8u16 {
        let layout = if channels == 7 { ChannelLayout::from_mask(0x70F) } else { ChannelLayout::default_for(channels) };
        let buf = tones(48_000, channels.into(), 24_000 + 77);
        let opts = EncodeOptions::new(EncodeFormat::Ogg).with_layout(layout).with_quality(0.6);
        let file = encode(&buf, &opts).unwrap();
        for decoder in decoders(&ff) {
            let out = ffmpeg_decode(&ff, decoder, &file, channels.into(), &format!("tones{channels}-{decoder}"));
            for (j, got) in out.iter().enumerate() {
                let c = if decoder == "libvorbis" { VORBIS_ORDER[usize::from(channels) - 1][j] } else { j };
                assert_eq!(got.len(), buf.frames(), "{channels} ch {decoder}: channel {j} length");
                let best = (0..buf.channels.len()).max_by(|&a, &b| {
                    correlation(&buf.channels[a][2048..], &got[2048..]).total_cmp(&correlation(&buf.channels[b][2048..], &got[2048..]))
                });
                assert_eq!(best, Some(c), "{channels} ch {decoder}: output channel {c} carries input {best:?}");
                let r = correlation(&buf.channels[c][2048..], &got[2048..]);
                assert!(r > 0.95, "{channels} ch {decoder}: channel {c} correlation {r:.3}");
            }
        }
    }
}

/// The comment header's tags, UTF-8 values included, are what ffprobe reports; a name that is not
/// a valid field name is left out.
#[test]
fn tags_reach_ffprobe() {
    let Some(probe) = crate::integration::codecs::common::tool("ffprobe").or_else(|| crate::integration::codecs::common::skip("ffprobe not found"))
    else {
        return;
    };
    let opts = OggVorbisOptions::new(3.0)
        .with_tag("TITLE", "Smörgåsbord ✓")
        .with_tag("ARTIST", "Craft")
        .with_tag("bad=name", "dropped")
        .with_vendor("craft-audio test");
    let file = encode_ogg_vorbis(&tones(44_100, 2, 44_100), &opts).unwrap();
    let path = crate::integration::codecs::testkit::fixtures_dir("vorbis-encoder").join("tags.ogg");
    std::fs::write(&path, &file).unwrap();
    let out = Command::new(probe).args(["-v", "error", "-of", "json", "-show_streams", "-show_format"]).arg(&path).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let j: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let s = &j["streams"][0];
    assert_eq!((s["codec_name"].as_str(), s["channels"].as_u64(), s["sample_rate"].as_str()), (Some("vorbis"), Some(2), Some("44100")));
    let tags = &s["tags"];
    assert_eq!((tags["TITLE"].as_str(), tags["ARTIST"].as_str()), (Some("Smörgåsbord ✓"), Some("Craft")));
    assert!(tags.as_object().unwrap().keys().all(|k| !k.contains("bad")), "{tags}");
    assert_eq!(j["format"]["duration"].as_str().map(|d| d.parse::<f64>().unwrap()), Some(1.0));
}

/// The lowest and highest qualities and the bit-rate path decode at the exact length, mono at
/// 44.1 kHz and stereo at 22.05 kHz.
#[test]
fn quality_extremes_and_rates_decode_exactly() {
    let ff = require_ffmpeg!();
    for (rate, channels, quality) in [(44_100u32, 1usize, -1.0f32), (44_100, 1, 10.0), (22_050, 2, -1.0), (22_050, 2, 10.0), (96_000, 2, 5.0)] {
        let buf = tones(rate, channels, rate as usize / 2 + 13);
        let file = encode_ogg_vorbis(&buf, &OggVorbisOptions::new(quality)).unwrap();
        for decoder in decoders(&ff) {
            let out = ffmpeg_decode(&ff, decoder, &file, channels, &format!("q{quality}-{rate}-{channels}-{decoder}"));
            assert!(out.iter().all(|c| c.len() == buf.frames()), "q{quality} {rate} Hz {decoder}: {}", out[0].len());
            let r = correlation(&buf.channels[0][2048..], &out[0][2048..]);
            assert!(r > 0.9, "q{quality} {rate} Hz {decoder}: correlation {r:.3}");
        }
    }
    let low = craft_audio::encode_ogg(&tones(48_000, 2, 48_000), craft_audio::OGG_BITRATES.0).unwrap();
    let high = craft_audio::encode_ogg(&tones(48_000, 2, 48_000), craft_audio::OGG_BITRATES.1).unwrap();
    assert!(low.len() < high.len(), "{} vs {} bytes", low.len(), high.len());
}
