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

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
    for (i, frame) in frames.iter().take(n).enumerate() {
        let data = file.read(&bytes, frame).map_err(|e| format!("frame {i}: {e}"))?;
        let parts = split_frame(&data, frame).map_err(|e| format!("frame {i}: {e}"))?;
        dec.decode(parts.palette, parts.video).map_err(|e| format!("frame {i}: {e}"))?;
    }

    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-v", "error", "-i"]).arg(path).args(["-an", "-fps_mode", "passthrough"]);
    // ffmpeg stops at `n` pictures only when fewer than all are compared: otherwise it gives
    // every picture it finds, so the size check below also compares the frame counts
    if n < frames.len() {
        cmd.args(["-frames:v", &n.to_string()]);
    }
    let mut child =
        cmd.args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"]).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    // Drain stderr concurrently so diagnostics cannot block the raw-video pipe. Retain a
    // bounded diagnostic prefix even when a corrupt input produces many repeated messages.
    let diagnostics = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stderr.by_ref().take(65_536).read_to_end(&mut bytes).and_then(|_| std::io::copy(&mut stderr, &mut std::io::sink()).map(|_| ()));
        (bytes, result)
    });
    let (w, h) = (file.header.width as usize, file.header.height as usize);
    let compared = (|| -> Result<(), String> {
        let mut dec = Decoder::new(&file.video_params()).map_err(|e| format!("decoder: {e}"))?;
        let mut want = vec![0u8; w * h * 3];
        for (i, frame) in frames.iter().take(n).enumerate() {
            stdout.read_exact(&mut want).map_err(|e| format!("ffmpeg frame {i} of {n}: {e}"))?;
            let data = file.read(&bytes, frame).map_err(|e| format!("frame {i}: {e}"))?;
            let parts = split_frame(&data, frame).map_err(|e| format!("frame {i}: {e}"))?;
            dec.decode(parts.palette, parts.video).map_err(|e| format!("frame {i}: {e}"))?;
            if !dec.pixels().iter().flat_map(|&p| dec.palette()[usize::from(p)]).eq(want.iter().copied()) {
                return Err(format!("frame {i} differs from ffmpeg"));
            }
        }
        let extra = std::io::copy(&mut stdout, &mut std::io::sink()).map_err(|e| e.to_string())?;
        if extra != 0 {
            return Err(format!("ffmpeg gave {extra} extra bytes after {n} frames of {w}x{h}"));
        }
        Ok(())
    })();
    // Reap the process even after a mismatch: dropping a child neither drains it nor waits.
    let drained = std::io::copy(&mut stdout, &mut std::io::sink());
    let status = child.wait().map_err(|e| e.to_string())?;
    let (stderr, diagnostics_result) = diagnostics.join().map_err(|_| "ffmpeg diagnostics reader panicked".to_string())?;
    diagnostics_result.map_err(|e| e.to_string())?;
    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr);
        if stderr.contains("Invalid data found when processing input") {
            return Ok(Outcome::Refused(n));
        }
        return Err(format!("ffmpeg failed: {}", stderr.trim()));
    }
    drained.map_err(|e| e.to_string())?;
    compared?;
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
