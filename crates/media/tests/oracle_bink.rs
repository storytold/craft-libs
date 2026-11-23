//! Real Bink files against ffprobe: every frame is read, and the stream, size and key flag of
//! each packet match the packets ffprobe reads. Packet contents are not compared.
//!
//! No tool here can write a Bink file, and existing files are not ours to commit, so the test
//! reads `*.bik` files from the directory named by `CRAFT_BINK_SAMPLES` (searched
//! recursively) and is skipped when that is not set.

use std::path::{Path, PathBuf};
use std::process::Command;

use craft_media::bink::open;

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

/// `(stream index, size, key)` of every packet ffprobe reads, in file order.
fn ffprobe_packets(ffprobe: &Path, path: &Path) -> Vec<(usize, usize, bool)> {
    let out =
        Command::new(ffprobe).args(["-v", "error", "-show_entries", "packet=stream_index,size,flags", "-of", "csv=p=0"]).arg(path).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut f = l.trim().split(',');
            Some((f.next()?.parse().ok()?, f.next()?.parse().ok()?, f.next()?.starts_with('K')))
        })
        .collect()
}

#[test]
fn real_files_match_ffprobe() {
    let Some(dir) = std::env::var_os("CRAFT_BINK_SAMPLES").filter(|d| !d.is_empty()) else {
        return craft_media_testkit::skip_test("bink oracle", "CRAFT_BINK_SAMPLES is not set");
    };
    let Some(ffprobe) = craft_media_testkit::ffprobe() else { return };
    let mut files = Vec::new();
    samples(Path::new(&dir), &mut files, 0);
    files.sort();
    assert!(!files.is_empty(), "no .bik files under {}", Path::new(&dir).display());
    for path in files {
        let file = std::fs::File::open(&path).unwrap();
        let bink = open(&file).unwrap();
        assert_eq!(bink.frames.len(), bink.header.frames as usize, "{}", path.display());

        // ffprobe numbers the video stream 0 and audio track `t` 1 + t, and its audio packets
        // start at the decoded size
        let mut ours = Vec::new();
        for (i, f) in bink.frames.iter().enumerate() {
            let frame = bink.read_frame(&file, i).unwrap();
            let packets = bink.split_frame(&frame).unwrap();
            for (t, a) in packets.audio.iter().enumerate() {
                if let Some(a) = a {
                    ours.push((1 + t, a.data.len() + 4, true));
                }
            }
            ours.push((0, packets.video.len(), f.key));
        }
        let theirs = ffprobe_packets(&ffprobe, &path);
        assert_eq!(ours.len(), theirs.len(), "{}: packet count", path.display());
        if let Some(at) = ours.iter().zip(&theirs).position(|(a, b)| a != b) {
            panic!("{}: packet {at} is {:?}, ffprobe has {:?}", path.display(), ours[at], theirs[at]);
        }
    }
}
