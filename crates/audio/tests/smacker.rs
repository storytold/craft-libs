//! Smacker audio against ffmpeg on real files.
//!
//! Real Smacker files come from commercial games and are not in the repository. Set
//! `CRAFT_SMACKER_SAMPLES` to a directory of `.smk` files (searched recursively) to run this;
//! without it, or without ffmpeg, the test is skipped.

use std::path::{Path, PathBuf};
use std::process::Command;

use craft_audio::smacker::{decode, pcm};
use craft_media::smacker::{AudioCoding, SmackerFile, open, split_frame};

fn smacker_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            smacker_files(&p, out);
        } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| ["smk", "sm2", "sm4"].contains(&x.to_ascii_lowercase().as_str())) {
            out.push(p);
        }
    }
}

/// Compares one track with ffmpeg. `Ok` is the number of samples that matched.
fn compare(ffmpeg: &Path, path: &Path, bytes: &[u8], file: &SmackerFile, slot: usize, stream: usize) -> Result<usize, String> {
    let track = &file.header.audio[slot];
    let mut got: Vec<i16> = Vec::new();
    for (i, frame) in file.frames.iter().chain(&file.ring_frame).enumerate() {
        let data = file.read(bytes, frame).map_err(|e| format!("frame {i}: {e}"))?;
        let parts = split_frame(&data, frame).map_err(|e| format!("frame {i}: {e}"))?;
        let Some(chunk) = parts.audio[slot] else { continue };
        let decoded = match track.coding {
            // no limit: the header's size field is below the chunks of many real files
            AudioCoding::Dpcm => decode(chunk, 0).map_err(|e| format!("frame {i}: {e}"))?,
            _ => pcm(chunk, track.sixteen_bit, track.stereo),
        };
        // a chunk without data does not say how many channels it has
        if !decoded.samples.is_empty() && decoded.channels != track.channels() {
            return Err(format!("frame {i}: {} channels, the header says {}", decoded.channels, track.channels()));
        }
        got.extend(decoded.samples.to_i16());
    }
    let out = Command::new(ffmpeg)
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-map", &format!("0:a:{stream}"), "-f", "s16le", "-"])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("ffmpeg: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let want: Vec<i16> = out.stdout.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
    if got.len() != want.len() {
        return Err(format!("{} samples, ffmpeg has {}", got.len(), want.len()));
    }
    if got != want {
        return Err("samples differ from ffmpeg".into());
    }
    Ok(got.len())
}

#[test]
fn samples_match_ffmpeg() {
    let Some(dir) = std::env::var_os("CRAFT_SMACKER_SAMPLES") else {
        craft_media_testkit::skip_test("smacker audio", "CRAFT_SMACKER_SAMPLES is not set");
        return;
    };
    let Some(ffmpeg) = craft_media_testkit::ffmpeg_or_skip("smacker audio oracle") else { return };
    let mut files = Vec::new();
    smacker_files(Path::new(&dir), &mut files);
    files.sort();
    assert!(!files.is_empty(), "no Smacker files under {}", Path::new(&dir).display());
    let (mut matched, mut bink) = (0, 0);
    let mut failures = Vec::new();
    for path in files {
        let bytes = std::fs::read(&path).unwrap();
        let file = match open(&bytes) {
            Ok(f) => f,
            Err(e) => {
                failures.push(format!("{}: open: {e}", path.display()));
                continue;
            }
        };
        // ffmpeg numbers the audio streams in slot order
        let tracks = file.header.audio.iter().enumerate().filter(|(_, t)| t.is_present());
        for (stream, (slot, track)) in tracks.enumerate() {
            if !matches!(track.coding, AudioCoding::Dpcm | AudioCoding::Pcm) {
                bink += 1;
                continue;
            }
            match compare(&ffmpeg, &path, &bytes, &file, slot, stream) {
                Ok(n) => {
                    matched += 1;
                    eprintln!("{} track {slot}: {n} samples match", path.display());
                }
                Err(what) => {
                    eprintln!("{} track {slot}: FAILED: {what}", path.display());
                    failures.push(format!("{} track {slot}: {what}", path.display()));
                }
            }
        }
    }
    eprintln!("{matched} tracks match, {bink} Bink audio tracks not compared");
    assert!(failures.is_empty(), "{} tracks differ:\n{}", failures.len(), failures.join("\n"));
    assert!(matched > 0, "no file had a track this crate decodes");
}
