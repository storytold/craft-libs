//! Real Bink files against ffmpeg: every decoded frame matches ffmpeg's, sample for sample.
//!
//! No tool here can write a Bink file, and existing files are not ours to commit, so the test
//! reads `*.bik` files from the directory named by `CRAFT_BINK_SAMPLES` (searched
//! recursively) and is skipped when that is not set. `CRAFT_BINK_FRAMES` limits the
//! frames compared per file (default 100; 0 compares all of them).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use craft_video::bink::Decoder;

fn samples(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() && depth < 8 {
            samples(&p, out, depth + 1);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("bik")) {
            out.push(p);
        }
    }
}

#[test]
fn real_files_decode_like_ffmpeg() {
    let Some(dir) = std::env::var_os("CRAFT_BINK_SAMPLES").filter(|d| !d.is_empty()) else {
        return craft_media_testkit::skip_test("bink oracle", "CRAFT_BINK_SAMPLES is not set");
    };
    let Some(ffmpeg) = craft_media_testkit::ffmpeg() else { return };
    let limit = std::env::var("CRAFT_BINK_FRAMES").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(100);
    let mut files = Vec::new();
    samples(Path::new(&dir), &mut files, 0);
    files.sort();
    assert!(!files.is_empty(), "no .bik files under {}", Path::new(&dir).display());
    let mut failures = Vec::new();
    let mut compared = 0usize;
    for path in files {
        let file = std::fs::File::open(&path).unwrap();
        let bink = craft_media::bink::open(&file).unwrap();
        let mut dec = match Decoder::from_params(&bink.video_params()) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("SKIPPED {}: {e}", path.display());
                continue;
            }
        };
        let (w, h) = (bink.header.width as usize, bink.header.height as usize);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let frames = if limit == 0 { bink.frames.len() } else { bink.frames.len().min(limit) };
        let mut child = Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&path)
            .args(["-an", "-frames:v", &frames.to_string(), "-f", "rawvideo", "-pix_fmt", "yuv420p", "-fps_mode", "passthrough", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut out = child.stdout.take().unwrap();
        let mut theirs = vec![0u8; w * h + 2 * cw * ch];
        let mut failed = None;
        for i in 0..frames {
            let frame = bink.read_frame(&file, i).unwrap();
            let packets = bink.split_frame(&frame).unwrap();
            let pic = match dec.decode(packets.video) {
                Ok(p) => p,
                Err(e) => {
                    failed = Some(format!("frame {i}: {e}"));
                    break;
                }
            };
            if out.read_exact(&mut theirs).is_err() {
                failed = Some(format!("frame {i}: ffmpeg gave no picture"));
                break;
            }
            let (y, rest) = theirs.split_at(w * h);
            let (u, v) = rest.split_at(cw * ch);
            // samples past the end of the shorter plane differ too
            let diff = |a: &[u8], b: &[u8]| a.iter().zip(b).filter(|(x, y)| x != y).count() + a.len().abs_diff(b.len());
            let (dy, du, dv) = (diff(&pic.y, y), diff(&pic.u, u), diff(&pic.v, v));
            if dy + du + dv != 0 {
                failed = Some(format!("frame {i}: {dy} luma, {du} Cb and {dv} Cr samples differ"));
                break;
            }
            compared += 1;
        }
        drop(out);
        let _ = child.kill();
        let _ = child.wait();
        match failed {
            None => eprintln!("ok {} ({frames} frames, {w}x{h})", path.display()),
            Some(f) => {
                eprintln!("FAILED {}: {f}", path.display());
                failures.push(format!("{}: {f}", path.display()));
            }
        }
    }
    assert!(failures.is_empty(), "{} file(s) differ from ffmpeg:\n{}", failures.len(), failures.join("\n"));
    // files of a revision the decoder refuses are skipped; a directory of only those proves nothing
    assert!(compared > 0, "no frame was compared: every file under {} was skipped or empty", Path::new(&dir).display());
}
