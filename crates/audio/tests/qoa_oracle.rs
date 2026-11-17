//! QOA against ffmpeg's decoder as the reference (oracle; skipped when ffmpeg is absent, a failure
//! with `CRAFT_MEDIA_REQUIRE_ORACLES=1`): ffmpeg decodes the files our encoder writes to exactly
//! the samples our decoder returns for them, over several sizes (partial slices, partial frames,
//! several frames), channel counts and rates.

use craft_audio::qoa::{decode, encode};
use std::process::Command;

fn signal(channels: usize, frames: usize) -> Vec<Vec<i16>> {
    let mut seed = 0x2545_F491u32;
    (0..channels)
        .map(|c| {
            (0..frames)
                .map(|i| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    let tone = ((i as f64) * (0.02 + 0.013 * c as f64)).sin() * 9000.0;
                    // Tones, noise bursts, silence and full-scale peaks.
                    match (i / 700) % 4 {
                        0 => tone as i16,
                        1 => (tone + f64::from(seed % 6000) - 3000.0) as i16,
                        2 => 0,
                        _ => {
                            if i % 2 == 0 {
                                i16::MAX
                            } else {
                                i16::MIN
                            }
                        }
                    }
                })
                .collect()
        })
        .collect()
}

#[test]
fn ffmpeg_decodes_our_files_to_the_same_samples() {
    let Some(ffmpeg) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("qoa");
    for (rate, channels, frames) in
        [(44_100u32, 1usize, 1usize), (8_000, 1, 19), (48_000, 2, 5_121), (22_050, 2, 12_345), (96_000, 1, 20_000), (44_100, 4, 6_000)]
    {
        let src = signal(channels, frames);
        let bytes = encode(rate, &src).expect("encode");
        let ours = decode(&bytes).expect("decode");
        let file = dir.join(format!("ours_{rate}_{channels}_{frames}.qoa"));
        std::fs::write(&file, &bytes).expect("write fixture");
        let out = Command::new(&ffmpeg)
            .args(["-v", "error", "-y", "-i"])
            .arg(&file)
            .args(["-f", "s16le", "-acodec", "pcm_s16le", "-"])
            .output()
            .expect("run ffmpeg");
        assert!(out.status.success(), "ffmpeg failed on {}: {}", file.display(), String::from_utf8_lossy(&out.stderr));
        let theirs: Vec<i16> = out.stdout.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
        let interleaved: Vec<i16> = (0..frames).flat_map(|i| ours.channels.iter().map(move |ch| ch[i])).collect();
        assert_eq!(theirs.len(), interleaved.len(), "{rate} Hz, {channels} ch, {frames} frames: sample count");
        assert!(theirs == interleaved, "{rate} Hz, {channels} ch, {frames} frames: samples differ");
    }
}
