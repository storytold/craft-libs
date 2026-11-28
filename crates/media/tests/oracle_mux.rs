//! Muxer oracle tests: remux ffmpeg-made files with our writers and validate with ffmpeg/ffprobe.

use super::common;

use common::*;
use craft_media::isobmff::*;
use serde_json::Value;
use std::io::Cursor;
use std::path::{Path, PathBuf};

/// All samples of all tracks in decode order interleaved by decode time: (track, sample index).
fn interleave(f: &Mp4File, tracks: &[usize]) -> Vec<(usize, usize)> {
    let mut v: Vec<(usize, usize)> = tracks.iter().flat_map(|&t| (0..f.tracks[t].samples.len()).map(move |i| (t, i))).collect();
    v.sort_by(|a, b| {
        let ta = &f.tracks[a.0];
        let tb = &f.tracks[b.0];
        let x = ta.samples[a.1].dts as i128 * tb.timescale as i128;
        let y = tb.samples[b.1].dts as i128 * ta.timescale as i128;
        x.cmp(&y).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1))
    });
    v
}

fn track_config(f: &Mp4File, t: &Track, copy_edits: bool) -> TrackConfig {
    let mut cfg = TrackConfig::new(t.entries[0].clone(), t.timescale);
    cfg.language = t.language.clone();
    if copy_edits {
        cfg.edits = t.edits.clone();
    }
    let _ = f;
    cfg
}

enum Finish {
    Normal,
    Faststart,
}

fn remux(input: &Path, out_name: &str, brand: Brand, finish: Finish, timecode: Option<(TimecodeConfig, u32)>) -> Option<(Mp4File, PathBuf)> {
    let data = std::fs::read(input).ok()?;
    let f = open(data.as_slice()).unwrap();
    let mut opts = WriterOptions::new(brand);
    opts.movie_timescale = f.timescale;
    opts.metadata.push(("©nam".into(), "FilmCraft test".into()));
    let mut w = Mp4Writer::new(Cursor::new(Vec::new()), opts).unwrap();
    let av: Vec<usize> = (0..f.tracks.len()).filter(|&i| matches!(f.tracks[i].kind, TrackKind::Video | TrackKind::Audio)).collect();
    let mut map = vec![usize::MAX; f.tracks.len()];
    for &i in &av {
        map[i] = w.add_track(track_config(&f, &f.tracks[i], true)).unwrap();
    }
    if let Some((tc, start)) = timecode {
        let v = av.iter().copied().find(|&i| f.tracks[i].kind == TrackKind::Video).unwrap();
        w.add_timecode_track(map[v], tc, start).unwrap();
    }
    for (t, i) in interleave(&f, &av) {
        let s = f.tracks[t].samples[i];
        let bytes = f.read_sample(data.as_slice(), t, i).unwrap();
        w.write_sample(map[t], WriteSample { data: &bytes, duration: s.duration, composition_offset: (s.pts - s.dts) as i32, is_sync: s.is_sync })
            .unwrap();
    }
    let out = match finish {
        Finish::Normal => w.finish(),
        Finish::Faststart => w.finish_faststart(),
    }
    .unwrap()
    .into_inner();
    let path = out_dir().join(out_name);
    std::fs::write(&path, &out).unwrap();
    // Our own reader must see identical sample tables and bytes.
    let g = open(out.as_slice()).unwrap();
    for (k, &i) in av.iter().enumerate() {
        let (a, b) = (&f.tracks[i], &g.tracks[k]);
        assert_eq!(a.samples.len(), b.samples.len(), "{out_name}: sample count");
        assert_eq!(a.edit_offset, b.edit_offset, "{out_name}: edit offset");
        assert_eq!(a.codec(), b.codec(), "{out_name}: codec");
        for (j, (x, y)) in a.samples.iter().zip(&b.samples).enumerate() {
            assert_eq!((x.size, x.dts, x.pts, x.duration, x.is_sync), (y.size, y.dts, y.pts, y.duration, y.is_sync), "{out_name}: sample {j}");
        }
        for j in 0..a.samples.len() {
            assert_eq!(f.read_sample(data.as_slice(), i, j).unwrap(), g.read_sample(out.as_slice(), k, j).unwrap());
        }
    }
    assert_eq!(g.metadata.title(), Some("FilmCraft test"));
    Some((f, path))
}

fn stream_packets(path: &Path) -> Option<Vec<Vec<Value>>> {
    let pk = packets(path)?;
    let n = pk.iter().filter_map(|p| int(&p["stream_index"])).max().map(|m| m + 1).unwrap_or(0);
    Some((0..n).map(|i| pk.iter().filter(|p| int(&p["stream_index"]) == Some(i)).cloned().collect()).collect())
}

/// Compare ffprobe's view of two files (per-stream packets: pts, dts, size, key flag).
fn assert_same_packets(a: &Path, b: &Path) {
    let (Some(pa), Some(pb)) = (stream_packets(a), stream_packets(b)) else { return };
    assert_eq!(pa.len(), pb.len(), "stream count {} vs {}", a.display(), b.display());
    for (i, (x, y)) in pa.iter().zip(&pb).enumerate() {
        let key = |p: &Value| (int(&p["pts"]), int(&p["dts"]), int(&p["size"]), p["flags"].as_str().map(|f| f.starts_with('K')));
        let kx: Vec<_> = x.iter().map(key).collect();
        let ky: Vec<_> = y.iter().map(key).collect();
        assert_eq!(kx.len(), ky.len(), "stream {i}: packet count {} vs {}", a.display(), b.display());
        for (j, (p, q)) in kx.iter().zip(&ky).enumerate() {
            assert_eq!(p, q, "stream {i} packet {j}");
        }
    }
}

fn assert_same_streams(a: &Path, b: &Path) {
    let (Some(sa), Some(sb)) = (streams(a), streams(b)) else { return };
    // Timecode (data) streams are compared by the tests that add them.
    let media = |s: Vec<Value>| s.into_iter().filter(|x| x["codec_type"] != "data").collect::<Vec<_>>();
    let (sa, sb) = (media(sa), media(sb));
    assert_eq!(sa.len(), sb.len(), "stream count {} vs {}", a.display(), b.display());
    for (x, y) in sa.iter().zip(&sb) {
        for k in ["codec_name", "width", "height", "sample_rate", "channels", "nb_frames", "duration_ts", "time_base", "profile", "pix_fmt"] {
            assert_eq!(x[k], y[k], "stream field {k}");
        }
    }
}

fn decode_clean(path: &Path) {
    let err = ffmpeg_decode_errors(path).unwrap();
    assert!(err.trim().is_empty(), "ffmpeg reported errors for {}: {err}", path.display());
}

#[test]
fn remux_h264_aac_mp4_variants() {
    let (Some(src), Some(_)) = (fixture("h264_bframes.mp4"), ffprobe()) else { return };
    for (name, brand, finish) in [
        ("remux_h264_aac.mp4", Brand::Mp4, Finish::Normal),
        ("remux_h264_aac_fast.mp4", Brand::Mp4, Finish::Faststart),
        ("remux_h264_aac.mov", Brand::Mov, Finish::Normal),
    ] {
        let (_, out) = remux(&src, name, brand, finish, None).unwrap();
        decode_clean(&out);
        assert_same_packets(&src, &out);
        assert_same_streams(&src, &out);
        eprintln!("{name}: ok");
    }
    // Faststart layout: moov before mdat.
    let d = std::fs::read(out_dir().join("remux_h264_aac_fast.mp4")).unwrap();
    let moov = d.windows(4).position(|w| w == b"moov").unwrap();
    let mdat = d.windows(4).position(|w| w == b"mdat").unwrap();
    assert!(moov < mdat);
}

/// Chapters written by [`Mp4Writer`] (QuickTime chapter track + Nero `chpl`) are what ffprobe
/// lists with `-show_chapters`: the right count, starts, ends and titles, in MP4 and MOV, and the
/// audio/video packets are unchanged.
#[test]
fn chapters_are_listed_by_ffprobe() {
    let (Some(src), Some(_)) = (fixture("h264_bframes.mp4"), ffprobe()) else { return };
    let data = std::fs::read(&src).unwrap();
    let f = open(data.as_slice()).unwrap();
    let av: Vec<usize> = (0..f.tracks.len()).filter(|&i| matches!(f.tracks[i].kind, TrackKind::Video | TrackKind::Audio)).collect();
    for (name, brand, late_first) in
        [("chapters.mp4", Brand::Mp4, false), ("chapters.mov", Brand::Mov, false), ("chapters_late.mp4", Brand::Mp4, true)]
    {
        let mut opts = WriterOptions::new(brand);
        // the copied edit lists are in the source's movie timescale
        opts.movie_timescale = f.timescale;
        let ms = |v: u64| v * u64::from(f.timescale) / 1000;
        let first = if late_first { 250 } else { 0 };
        opts.chapters = vec![
            Chapter { start: ms(first), title: "Opening".into() },
            Chapter { start: ms(500), title: "Mitte — ü 日本".into() },
            Chapter { start: ms(800), title: "End".into() },
        ];
        let mut w = Mp4Writer::new(Cursor::new(Vec::new()), opts).unwrap();
        let mut map = vec![usize::MAX; f.tracks.len()];
        for &i in &av {
            map[i] = w.add_track(track_config(&f, &f.tracks[i], true)).unwrap();
        }
        for (t, i) in interleave(&f, &av) {
            let s = f.tracks[t].samples[i];
            let bytes = f.read_sample(data.as_slice(), t, i).unwrap();
            w.write_sample(
                map[t],
                WriteSample { data: &bytes, duration: s.duration, composition_offset: (s.pts - s.dts) as i32, is_sync: s.is_sync },
            )
            .unwrap();
        }
        let out = w.finish_faststart().unwrap().into_inner();
        let path = out_dir().join(name);
        std::fs::write(&path, &out).unwrap();
        let Some(v) = probe_json(&path, &["-show_chapters", "-show_format"]) else { return };
        let ch = v["chapters"].as_array().cloned().unwrap_or_default();
        let got: Vec<(f64, String)> = ch
            .iter()
            .map(|c| (c["start_time"].as_str().and_then(|s| s.parse().ok()).unwrap_or(-1.0), c["tags"]["title"].as_str().unwrap_or("").to_string()))
            .collect();
        let want = [(first as f64 / 1000.0, "Opening"), (0.5, "Mitte — ü 日本"), (0.8, "End")];
        assert_eq!(got.len(), want.len(), "{name}: {v}");
        for ((s, t), (ws, wt)) in got.iter().zip(want) {
            assert!((s - ws).abs() < 0.002, "{name}: start {s} vs {ws}");
            assert_eq!(t, wt, "{name}");
        }
        // The last chapter runs to the end of the movie. With a late first chapter ffprobe ends it
        // early by the empty edit (it takes the end from the chapter track's media duration,
        // without the edit); the starts, which players use, are exact.
        let end: f64 = ch.last().and_then(|c| c["end_time"].as_str()).and_then(|s| s.parse().ok()).unwrap_or(-1.0);
        let dur: f64 = v["format"]["duration"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let early = if late_first { 0.25 } else { 0.0 };
        assert!((end + early - dur).abs() < 0.05, "{name}: last chapter ends at {end}, movie lasts {dur}");
        // Our own reader sees the same list in both forms.
        let g = open(out.as_slice()).unwrap();
        let ours: Vec<String> = g.read_chapters(out.as_slice()).unwrap().into_iter().map(|c| c.title).collect();
        assert_eq!(ours, ["Opening", "Mitte — ü 日本", "End"], "{name}");
        assert_eq!(g.chpl_chapters.len(), 3, "{name}");
        assert_same_packets(&src, &path);
        decode_clean(&path);
    }
}
