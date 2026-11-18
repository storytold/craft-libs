//! AVI files from [`AviWriter`] read by ffmpeg and ffprobe (skipped without them): ffmpeg makes
//! an MPEG-4 Part 2 elementary stream and PCM, the muxer interleaves them, and ffmpeg must decode
//! the AVI to exactly what it decodes from the sources, in one RIFF and across AVIX extensions.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;

use craft_media::avi::{AudioSpec, AviWriter, MuxOptions, StreamSpec, VideoSpec};

const FRAMES: usize = 50;
/// 25 frames a second of 48 kHz stereo 16-bit PCM: bytes of audio a frame.
const AUDIO_PER_FRAME: usize = 48_000 / 25 * 4;

fn tools() -> Option<(PathBuf, PathBuf)> {
    let ff = craft_media_testkit::ffmpeg_or_skip("avi muxer oracle")?;
    let probe = craft_media_testkit::ffprobe()?;
    Some((ff, probe))
}

fn run(cmd: &mut Command) -> Vec<u8> {
    let out = cmd.output().expect("run tool");
    assert!(out.status.success(), "{:?}: {}", cmd, String::from_utf8_lossy(&out.stderr));
    // A plain WAVEFORMATEX has no channel mask, so ffmpeg notes the layout it assumes for any
    // stereo PCM in AVI, its own files included. Every other line is a failure.
    let diag: Vec<String> =
        String::from_utf8_lossy(&out.stderr).lines().filter(|l| !l.contains("Guessed Channel Layout: stereo")).map(str::to_owned).collect();
    assert!(diag.is_empty(), "{cmd:?} reported: {}", diag.join("\n"));
    out.stdout
}

/// The sources: an MPEG-4 Part 2 elementary stream (I-VOP every 12 frames, no B-VOPs) and raw
/// PCM, both from ffmpeg.
fn sources(ff: &Path, dir: &Path) -> (Vec<u8>, Vec<u8>) {
    let m4v = dir.join("video.m4v");
    run(Command::new(ff)
        .args(["-v", "warning", "-y", "-f", "lavfi", "-i", "testsrc2=s=160x120:r=25"])
        .args(["-frames:v", &FRAMES.to_string(), "-c:v", "mpeg4", "-threads", "1", "-q:v", "5", "-g", "12", "-bf", "0", "-f", "m4v"])
        .arg(&m4v));
    let pcm = run(Command::new(ff)
        .args(["-v", "warning", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000"])
        .args(["-t", "2", "-ac", "2", "-f", "s16le", "-"]));
    (std::fs::read(&m4v).expect("read m4v"), pcm)
}

/// The VOPs of an elementary stream: each runs from its start code (the first from the start of
/// the stream, so it carries the configuration headers) to the next. An I-VOP is a key frame.
fn vops(es: &[u8]) -> Vec<(&[u8], bool)> {
    let starts: Vec<usize> = es.windows(4).enumerate().filter(|(_, w)| *w == [0, 0, 1, 0xB6]).map(|(i, _)| i).collect();
    let mut out = Vec::new();
    for (k, &s) in starts.iter().enumerate() {
        let from = if k == 0 { 0 } else { s };
        let to = starts.get(k + 1).copied().unwrap_or(es.len());
        out.push((&es[from..to], es.get(s + 4).is_some_and(|b| b >> 6 == 0)));
    }
    out
}

fn mux(video: &[(&[u8], bool)], pcm: &[u8], opts: MuxOptions) -> Vec<u8> {
    let streams = vec![StreamSpec::Video(VideoSpec::new(*b"XVID", 160, 120, 25, 1)), StreamSpec::Audio(AudioSpec::pcm(48_000, 2, 16))];
    let mut w = AviWriter::new(Cursor::new(Vec::new()), streams, opts).expect("writer");
    for (i, (vop, key)) in video.iter().enumerate() {
        w.write_chunk(0, vop, *key).expect("video");
        let a = pcm.get(i * AUDIO_PER_FRAME..((i + 1) * AUDIO_PER_FRAME).min(pcm.len())).unwrap_or_default();
        w.write_chunk(1, a, true).expect("audio");
    }
    w.finish().expect("finish").into_inner()
}

fn decode(ff: &Path, input: &Path, format: Option<&str>, map: &str, out: &[&str]) -> Vec<u8> {
    let mut c = Command::new(ff);
    c.args(["-v", "warning", "-nostdin"]);
    if let Some(f) = format {
        c.args(["-f", f]);
    }
    c.arg("-i").arg(input).args(["-map", map]).args(out).arg("-");
    run(&mut c)
}

#[test]
fn ffmpeg_decodes_muxed_files_like_their_sources() {
    let Some((ff, probe)) = tools() else { return };
    let dir = craft_media_testkit::private_dir("avi-mux");
    let (es, pcm) = sources(&ff, &dir);
    let video = vops(&es);
    assert_eq!(video.len(), FRAMES);
    assert!(video.iter().filter(|v| v.1).count() >= 4);
    let want_video = decode(&ff, &dir.join("video.m4v"), Some("m4v"), "0:v", &["-f", "rawvideo", "-pix_fmt", "yuv420p"]);

    for (name, opts) in [("one_riff.avi", MuxOptions::default()), ("avix.avi", MuxOptions { riff_limit: 64 << 10, max_riffs: 64 })] {
        let file = mux(&video, &pcm, opts);
        let path = dir.join(name);
        std::fs::write(&path, &file).expect("write avi");
        if name == "avix.avi" {
            assert!(file.windows(4).filter(|w| *w == b"AVIX").count() >= 3, "{name}: AVIX extensions");
        }
        let got_video = decode(&ff, &path, None, "0:v", &["-f", "rawvideo", "-pix_fmt", "yuv420p"]);
        assert!(got_video == want_video, "{name}: decoded pictures differ from the elementary stream's");
        // ffmpeg reads an interleaved file front to back; a seek goes through the index (OpenDML
        // when the file has it). 1.0 s lands after key frame 24, so frame 25 comes out first.
        for at in ["1.0", "1.6"] {
            let frame = (at.parse::<f64>().unwrap_or(0.0) * 25.0).round() as usize;
            let size = 160 * 120 * 3 / 2;
            let mut c = Command::new(&ff);
            c.args(["-v", "warning", "-nostdin", "-ss", at, "-i"]).arg(&path).args([
                "-map",
                "0:v",
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "yuv420p",
                "-",
            ]);
            let got = run(&mut c);
            assert!(got == want_video[frame * size..(frame + 1) * size], "{name}: seeking to {at} s gave another picture than frame {frame}");
        }
        let got_audio = decode(&ff, &path, None, "0:a", &["-f", "s16le"]);
        assert!(got_audio == pcm, "{name}: decoded audio differs from the PCM ({} vs {} bytes)", got_audio.len(), pcm.len());

        let info = String::from_utf8_lossy(&run(Command::new(&probe)
            .args([
                "-v",
                "error",
                "-count_packets",
                "-show_entries",
                "stream=codec_name,codec_tag_string,width,height,avg_frame_rate,nb_read_packets,sample_rate,channels",
                "-of",
                "default=nw=1",
            ])
            .arg(&path)))
        .into_owned();
        for want in [
            "codec_name=mpeg4",
            "codec_tag_string=XVID",
            "width=160",
            "height=120",
            "avg_frame_rate=25/1",
            "nb_read_packets=50",
            "codec_name=pcm_s16le",
            "sample_rate=48000",
            "channels=2",
        ] {
            assert!(info.lines().any(|l| l == want), "{name}: ffprobe lacks {want}:\n{info}");
        }
        let duration = String::from_utf8_lossy(&run(Command::new(&probe)
            .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
            .arg(&path)))
        .trim()
        .parse::<f64>()
        .unwrap_or(0.0);
        assert!((duration - 2.0).abs() < 0.05, "{name}: duration {duration}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
