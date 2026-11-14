//! Smacker audio against ffmpeg on real files.
//!
//! Real Smacker files come from commercial games and are not in the repository. Set
//! `CRAFT_SMACKER_SAMPLES` to a directory of `.smk` files (searched recursively) to run this;
//! without it, or without ffmpeg, the test is skipped.

use std::path::{Path, PathBuf};
use std::process::Command;

use craft_audio::smacker::{decode, pcm};
use craft_media::smacker::{AudioCoding, open, split_frame};

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
    let mut with_audio = 0;
    for path in files {
        let bytes = std::fs::read(&path).unwrap();
        let file = open(&bytes).unwrap();
        // ffmpeg numbers the audio streams in slot order
        let tracks = file.header.audio.iter().enumerate().filter(|(_, t)| t.is_present());
        for (stream, (slot, track)) in tracks.enumerate() {
            if !matches!(track.coding, AudioCoding::Dpcm | AudioCoding::Pcm) {
                eprintln!("{}: track {slot} is Bink audio, not compared", path.display());
                continue;
            }
            let mut got: Vec<i16> = Vec::new();
            for (i, frame) in file.frames.iter().chain(&file.ring_frame).enumerate() {
                let data = file.read(&bytes, frame).unwrap();
                let parts = split_frame(&data, frame).unwrap();
                let Some(chunk) = parts.audio[slot] else { continue };
                let decoded = match track.coding {
                    AudioCoding::Dpcm => decode(chunk).unwrap_or_else(|e| panic!("{} frame {i}: {e}", path.display())),
                    _ => pcm(chunk, track.sixteen_bit, track.stereo),
                };
                assert_eq!(decoded.channels, track.channels(), "{} frame {i}", path.display());
                got.extend(decoded.samples.to_i16());
            }
            let out = Command::new(&ffmpeg)
                .args(["-v", "error", "-i"])
                .arg(&path)
                .args(["-map", &format!("0:a:{stream}"), "-f", "s16le", "-"])
                .output()
                .unwrap();
            assert!(out.status.success(), "{}: {}", path.display(), String::from_utf8_lossy(&out.stderr));
            let want: Vec<i16> = out.stdout.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
            assert_eq!(got.len(), want.len(), "{} track {slot}: sample count", path.display());
            assert!(got == want, "{} track {slot} differs from ffmpeg", path.display());
            with_audio += 1;
            eprintln!("{} track {slot}: {} samples match", path.display(), got.len());
        }
    }
    assert!(with_audio > 0, "no file had an audio track");
}
