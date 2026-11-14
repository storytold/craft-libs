//! Smacker video against ffmpeg on real files.
//!
//! Real Smacker files come from commercial games and are not in the repository. Set
//! `CRAFT_SMACKER_SAMPLES` to a directory of `.smk` files (searched recursively) to run this;
//! without it, or without ffmpeg, the test is skipped. `CRAFT_SMACKER_FRAMES` is the number of
//! frames compared for each file (default 150, 0 for all).

use std::path::{Path, PathBuf};
use std::process::Command;

use craft_media::smacker::{open, split_frame};
use craft_video::smacker::Decoder;

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

fn samples() -> Option<Vec<PathBuf>> {
    let Some(dir) = std::env::var_os("CRAFT_SMACKER_SAMPLES") else {
        craft_media_testkit::skip_test("smacker video", "CRAFT_SMACKER_SAMPLES is not set");
        return None;
    };
    let mut files = Vec::new();
    smacker_files(Path::new(&dir), &mut files);
    files.sort();
    assert!(!files.is_empty(), "no Smacker files under {}", Path::new(&dir).display());
    Some(files)
}

#[test]
fn pictures_match_ffmpeg() {
    let Some(files) = samples() else { return };
    let Some(ffmpeg) = craft_media_testkit::ffmpeg_or_skip("smacker video oracle") else { return };
    let limit: usize = std::env::var("CRAFT_SMACKER_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(150);
    for path in files {
        let bytes = std::fs::read(&path).unwrap();
        let file = open(&bytes).unwrap();
        assert!(!file.truncated, "{}", path.display());
        let frames: Vec<_> = file.frames.iter().chain(&file.ring_frame).collect();
        let n = if limit == 0 { frames.len() } else { frames.len().min(limit) };

        let out = Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&path)
            .args(["-an", "-fps_mode", "passthrough", "-frames:v", &n.to_string(), "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{}: {}", path.display(), String::from_utf8_lossy(&out.stderr));
        let (w, h) = (file.header.width as usize, file.header.height as usize);
        assert_eq!(out.stdout.len(), n * w * h * 3, "{}: ffmpeg frame count", path.display());

        let mut dec = Decoder::new(&file.video_params()).unwrap();
        for (i, (frame, want)) in frames.iter().zip(out.stdout.chunks_exact(w * h * 3)).enumerate() {
            let data = file.read(&bytes, frame).unwrap();
            let parts = split_frame(&data, frame).unwrap();
            dec.decode(parts.palette, parts.video).unwrap_or_else(|e| panic!("{} frame {i}: {e}", path.display()));
            let got: Vec<u8> = dec.pixels().iter().flat_map(|&p| dec.palette()[usize::from(p)]).collect();
            assert!(got == want, "{} frame {i} differs from ffmpeg", path.display());
        }
        eprintln!("{}: {n} frames match", path.display());
    }
}
