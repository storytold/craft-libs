//! Smacker video against ffmpeg on real files.
//!
//! Real Smacker files come from commercial games and are not in the repository. Set
//! `CRAFT_SMACKER_SAMPLES` to a directory of `.smk` files (searched recursively) to run this;
//! without it, or without ffmpeg, the test is skipped. `CRAFT_SMACKER_FRAMES` is the number of
//! frames compared for each file (default 150, 0 for all).
//!
//! Every file is compared, and the test then fails with the list of those that differ. A file
//! ffmpeg refuses as invalid data is run through the decoder and reported, and does not count as
//! a difference unless the decoder fails on it too or more than one file in a hundred is refused.

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

/// How one file compared with ffmpeg.
enum Outcome {
    /// Every compared picture is the same; the number of pictures.
    Match(usize),
    /// ffmpeg refuses the file as invalid data, and the decoder read this many frames of it
    /// without error. There is nothing to compare.
    Refused(usize),
}

/// Compares one file with ffmpeg. `Err` is the first difference.
fn compare(ffmpeg: &Path, path: &Path, limit: usize) -> Result<Outcome, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let file = open(&bytes).map_err(|e| format!("open: {e}"))?;
    if file.truncated {
        return Err("file is cut short".into());
    }
    let frames: Vec<_> = file.frames.iter().chain(&file.ring_frame).collect();
    let n = if limit == 0 { frames.len() } else { frames.len().min(limit) };

    // our pictures first, so a file ffmpeg refuses still runs through the decoder
    let mut dec = Decoder::new(&file.video_params()).map_err(|e| format!("decoder: {e}"))?;
    let mut ours: Vec<Vec<u8>> = Vec::new();
    for (i, frame) in frames.iter().take(n).enumerate() {
        let data = file.read(&bytes, frame).map_err(|e| format!("frame {i}: {e}"))?;
        let parts = split_frame(&data, frame).map_err(|e| format!("frame {i}: {e}"))?;
        dec.decode(parts.palette, parts.video).map_err(|e| format!("frame {i}: {e}"))?;
        ours.push(dec.pixels().iter().flat_map(|&p| dec.palette()[usize::from(p)]).collect());
    }

    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-v", "error", "-i"]).arg(path).args(["-an", "-fps_mode", "passthrough"]);
    // ffmpeg stops at `n` pictures only when fewer than all are compared: otherwise it gives
    // every picture it finds, so the size check below also compares the frame counts
    if n < frames.len() {
        cmd.args(["-frames:v", &n.to_string()]);
    }
    let out = cmd.args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"]).output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        // only a file ffmpeg calls invalid is set aside; any other failure of ffmpeg is one
        if stderr.contains("Invalid data found when processing input") {
            return Ok(Outcome::Refused(n));
        }
        return Err(format!("ffmpeg failed: {}", stderr.trim()));
    }
    let (w, h) = (file.header.width as usize, file.header.height as usize);
    if out.stdout.len() != n * w * h * 3 {
        return Err(format!("ffmpeg gave {} bytes, {n} frames of {w}x{h} are {}", out.stdout.len(), n * w * h * 3));
    }
    for (i, (got, want)) in ours.iter().zip(out.stdout.chunks_exact(w * h * 3)).enumerate() {
        if got != want {
            return Err(format!("frame {i} differs from ffmpeg"));
        }
    }
    Ok(Outcome::Match(n))
}

/// Files ffmpeg may refuse before the test fails: a few real files have no video data at all,
/// which ffmpeg does not accept, but an ffmpeg that refuses everything is not an oracle.
const MAX_REFUSED_SHARE: usize = 100;

#[test]
fn pictures_match_ffmpeg() {
    let Some(files) = samples() else { return };
    let Some(ffmpeg) = craft_media_testkit::ffmpeg_or_skip("smacker video oracle") else { return };
    let limit: usize = std::env::var("CRAFT_SMACKER_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(150);
    let (mut matched, mut refused) = (0usize, 0usize);
    let mut failures = Vec::new();
    for path in &files {
        match compare(&ffmpeg, path, limit) {
            Ok(Outcome::Match(n)) => {
                matched += 1;
                eprintln!("{}: {n} frames match", path.display());
            }
            Ok(Outcome::Refused(n)) => {
                refused += 1;
                eprintln!("{}: ffmpeg cannot decode it; ours: {n} frames", path.display());
            }
            Err(what) => {
                eprintln!("{}: FAILED: {what}", path.display());
                failures.push(format!("{}: {what}", path.display()));
            }
        }
    }
    eprintln!("{matched} files match, ffmpeg refuses {refused}");
    assert!(failures.is_empty(), "{} of {} files differ:\n{}", failures.len(), files.len(), failures.join("\n"));
    assert!(matched > 0, "no file was compared");
    assert!(refused * MAX_REFUSED_SHARE <= files.len(), "ffmpeg refuses {refused} of {} files", files.len());
}
