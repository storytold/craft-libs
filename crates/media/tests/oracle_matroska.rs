//! Oracle tests: compare our demuxer's packets, timestamps, keyframes and track parameters with
//! ffprobe on ffmpeg-generated fixtures (skipped when ffmpeg/ffprobe are not installed).

use super::common;
#[path = "common/matroska.rs"]
mod mkv;

use craft_media::matroska::*;
use mkv::*;
use serde_json::Value;
use std::path::Path;

fn assert_streams_match(json: &Value, file: &MkvFile, name: &str) {
    let streams = json["streams"].as_array().unwrap();
    assert_eq!(streams.len(), file.tracks.len(), "{name}: stream count");
    for (index, stream) in streams.iter().enumerate() {
        let track = &file.tracks[index];
        assert_eq!(stream["codec_name"].as_str().unwrap_or(""), track.codec.name(), "{name}: track {index} codec ({})", track.codec_id);
        assert_eq!(stream["time_base"].as_str(), Some(format!("{}/{}", track.timebase.0, track.timebase.1).as_str()), "{name}: time_base");
        match track.kind {
            TrackKind::Video => {
                let video = track.video.as_ref().unwrap();
                assert_eq!(int(&stream["width"]), Some(video.pixel_width as i64), "{name}: width");
                assert_eq!(int(&stream["height"]), Some(video.pixel_height as i64), "{name}: height");
            }
            TrackKind::Audio => {
                let audio = track.audio.as_ref().unwrap();
                assert_eq!(int(&stream["sample_rate"]), Some(audio.sampling_frequency as i64), "{name}: sample_rate");
                assert_eq!(int(&stream["channels"]), Some(audio.channels as i64), "{name}: channels");
            }
            _ => {}
        }
    }
}

fn assert_packets_match(json: &Value, file: &MkvFile, packets: &[Packet], name: &str) {
    let ffprobe_packets = json["packets"].as_array().unwrap();
    assert_eq!(packets.len(), ffprobe_packets.len(), "{name}: packet count");
    let (mut pts_checked, mut duration_checked) = (0, 0);
    for (index, (packet, ffprobe_packet)) in packets.iter().zip(ffprobe_packets).enumerate() {
        let context = format!("{name}: packet {index} (track {})", packet.track);
        assert_eq!(int(&ffprobe_packet["stream_index"]), Some(packet.track as i64), "{context}: stream");
        if file.tracks[packet.track].codec_id.starts_with("D_WEBVTT") {
            // WebM WebVTT blocks carry "id\nsettings\npayload"; FFmpeg moves id/settings to side data.
            assert!(packet.data.len() as i64 >= int(&ffprobe_packet["size"]).unwrap(), "{context}: size");
        } else {
            assert_eq!(int(&ffprobe_packet["size"]), Some(packet.data.len() as i64), "{context}: size");
        }
        assert_eq!(ffprobe_packet["flags"].as_str().unwrap_or("").starts_with('K'), packet.keyframe, "{context}: keyframe");
        compare_packet_pts(ffprobe_packet, packet, &context, &mut pts_checked);
        compare_packet_duration(ffprobe_packet, packet, packets, file, &context, index, &mut duration_checked);
    }
    assert!(pts_checked > 0, "{name}: no pts compared");
    eprintln!("{name}: {} packets, {pts_checked} pts and {duration_checked} durations compared", packets.len());
}

fn compare_packet_pts(ffprobe_packet: &Value, packet: &Packet, context: &str, checked: &mut usize) {
    if let Some(pts) = int(&ffprobe_packet["pts"])
        && !(packet.lace > 0 && packet.duration == 0)
    {
        // FFmpeg truncates per-lace duration to whole ticks; later laces can differ by one tick each.
        assert!((pts - packet.pts).abs() <= packet.lace as i64, "{context}: pts ffprobe {pts} vs {}", packet.pts);
        *checked += 1;
    }
}

fn compare_packet_duration(
    ffprobe_packet: &Value,
    packet: &Packet,
    packets: &[Packet],
    file: &MkvFile,
    context: &str,
    index: usize,
    checked: &mut usize,
) {
    // The final audio/video frame cut by `-t` has a shortened BlockDuration, but ffprobe reports
    // the codec frame duration. Subtitle durations remain exact cue intervals and are compared.
    let last_of_track = packets[index + 1..].iter().all(|next| next.track != packet.track);
    if let Some(duration) = int(&ffprobe_packet["duration"])
        && packet.duration != 0
        && (!last_of_track || file.tracks[packet.track].kind == TrackKind::Subtitle)
    {
        assert!((duration - packet.duration as i64).abs() <= 1, "{context}: duration ffprobe {duration} vs {}", packet.duration);
        *checked += 1;
    }
}

fn assert_packet_index_matches(file: &MkvFile, packets: &[Packet], bytes: &[u8], name: &str) {
    for (track_index, track) in file.tracks.iter().enumerate() {
        let track_packets: Vec<&Packet> = packets.iter().filter(|packet| packet.track == track_index).collect();
        assert_eq!(track.samples.len(), track_packets.len(), "{name}: index size track {track_index}");
        for (sample_index, packet) in track_packets.iter().enumerate() {
            assert_eq!(track.samples[sample_index].pts, packet.pts);
            assert_eq!(track.samples[sample_index].keyframe, packet.keyframe);
            assert_eq!(track.sample_size(sample_index), Some(packet.data.len() as u64));
        }
        if let Some(sample_index) = track_packets.len().checked_sub(1) {
            assert_eq!(file.read_sample(bytes, track_index, sample_index).unwrap(), track_packets[sample_index].data);
        }
    }
}

fn assert_duration_matches(json: &Value, file: &MkvFile, name: &str) {
    if let Some(ffprobe_duration) = json["format"]["duration"].as_str().and_then(|duration| duration.parse::<f64>().ok())
        && let Some(duration_ns) = file.duration_ns()
    {
        assert!((ffprobe_duration - duration_ns as f64 / 1e9).abs() < 0.05, "{name}: duration {ffprobe_duration} vs {duration_ns}");
    }
}

/// Compare every packet (file order) and stream parameter against ffprobe.
fn compare(ffprobe: &Path, path: &Path) -> (MkvFile, Vec<Packet>) {
    let json = ffprobe_json(ffprobe, path);
    let bytes = std::fs::read(path).unwrap();
    let mut demuxer = Demuxer::from_slice(&bytes).unwrap();
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let file = demuxer.file().clone();
    assert_streams_match(&json, &file, &name);
    let packets: Vec<Packet> = demuxer.by_ref().collect::<Result<_>>().unwrap();
    assert_packets_match(&json, &file, &packets, &name);
    assert_packet_index_matches(&file, &packets, &bytes, &name);
    assert_duration_matches(&json, &file, &name);
    (file, packets)
}

fn run(name: &str) -> Option<(Vec<u8>, MkvFile, Vec<Packet>)> {
    let (ffmpeg, ffprobe) = tools()?;
    let path = fixture(&ffmpeg, name)?;
    let (file, packets) = compare(&ffprobe, &path);
    let bytes = std::fs::read(&path).unwrap();
    Some((bytes, file, packets))
}

/// Re-mux a fixture with our writer using each lacing mode; ffprobe must read the result identically
/// to our demuxer (and to the source packets).
#[test]
fn laced_remux() {
    let Some((ffmpeg, ffprobe)) = tools() else { return };
    for (src, modes) in [
        ("vorbis.mkv", &[LacingMode::Xiph, LacingMode::Ebml][..]),
        ("h264_aac.mkv", &[LacingMode::Ebml, LacingMode::Xiph][..]),
        ("prores_pcm.mkv", &[LacingMode::FixedOrEbml][..]),
        ("flac.mkv", &[LacingMode::Ebml][..]),
    ] {
        let Some(path) = fixture(&ffmpeg, src) else { continue };
        let bytes = std::fs::read(&path).unwrap();
        let d = Demuxer::from_slice(&bytes).unwrap();
        let file = d.file().clone();
        let packets: Vec<Packet> = d.collect::<Result<_>>().unwrap();
        for &mode in modes {
            let specs: Vec<TrackSpec> = file
                .tracks
                .iter()
                .enumerate()
                .map(|(ti, t)| {
                    let mut s = TrackSpec::new(t.kind, t.codec_id.clone());
                    s.codec_private = t.codec_private.clone();
                    // BlockDuration overrides DefaultDuration: ffmpeg's Vorbis files declare a short
                    // default and store longer durations per block. Audio takes its median packet
                    // duration (the packet spacing when no packet has one), so its frames can lace.
                    s.default_duration_ns = if t.kind == TrackKind::Audio {
                        let audio: Vec<&Packet> = packets.iter().filter(|p| p.track == ti).collect();
                        let mut durations: Vec<u64> = audio.iter().map(|p| p.duration_ns).filter(|&d| d > 0).collect();
                        if durations.is_empty() {
                            durations = audio.windows(2).filter_map(|w| u64::try_from(w[1].pts_ns - w[0].pts_ns).ok()).filter(|&d| d > 0).collect();
                        }
                        durations.sort_unstable();
                        durations.get(durations.len() / 2).copied().or(t.default_duration_ns)
                    } else {
                        t.default_duration_ns
                    };
                    s.video_size = t.video.as_ref().map(|v| (v.pixel_width, v.pixel_height));
                    s.audio = t.audio.as_ref().map(|a| (a.sampling_frequency, a.channels, a.bit_depth));
                    s.codec_delay_ns = t.codec_delay_ns;
                    s
                })
                .collect();
            let opts = MuxOptions { lacing: mode, ..Default::default() };
            let mut w = MkvWriter::new(std::io::Cursor::new(Vec::new()), specs, opts).unwrap();
            for p in &packets {
                // Keep explicit durations, a short last packet's too: one that differs from the
                // track's default ends the lace.
                w.write_frame(p.track, p.pts_ns, p.keyframe, &p.data, (p.duration_ns > 0).then_some(p.duration_ns)).unwrap();
            }
            let out = w.finish().unwrap().into_inner();
            let laced = {
                let f = open(&out[..]).unwrap();
                f.tracks.iter().any(|t| t.samples.iter().any(|s| s.lace > 0))
            };
            assert!(laced, "{src} {mode:?}: nothing was laced");
            let outp = fixture_dir().join(format!("remux_{mode:?}_{src}"));
            std::fs::write(&outp, &out).unwrap();
            let (_, got) = compare(&ffprobe, &outp);
            assert_eq!(got.len(), packets.len());
            for (a, b) in got.iter().zip(&packets) {
                assert_eq!(a.data, b.data, "{src} {mode:?}: payload");
                assert_eq!((a.track, a.pts), (b.track, b.pts), "{src} {mode:?}: timestamp");
            }
            std::fs::remove_file(&outp).unwrap();
        }
    }
}

/// Damaged file: overwrite bytes in the middle of the clusters; the demuxer must not fail and must
/// resync on a later cluster.
#[test]
fn damaged_resync() {
    let Some((mut bytes, f, packets)) = run("h264_aac.mkv") else { return };
    let mid = f.clusters[f.clusters.len() / 2];
    let at = mid.data_start as usize + 40;
    for b in &mut bytes[at..at + 200] {
        *b = 0x00;
    }
    let d = Demuxer::from_slice(&bytes).unwrap();
    assert!(!d.file().warnings.is_empty());
    let got: Vec<Packet> = d.collect::<Result<_>>().unwrap();
    assert!(got.len() < packets.len());
    assert!(got.len() > packets.len() / 2);
    // packets after the damaged cluster are intact
    let last = packets.last().unwrap();
    assert_eq!(got.last().unwrap(), last);
    // truncated file
    let cut = &bytes[..bytes.len() * 2 / 3];
    let d = Demuxer::from_slice(cut).unwrap();
    let got: Vec<Packet> = d.collect::<Result<_>>().unwrap();
    assert!(!got.is_empty());
    // garbage never panics
    for n in [0usize, 10, 100, 1000] {
        let _ = Demuxer::from_slice(&bytes[..n.min(bytes.len())]).map(|d| d.count());
    }
}

/// Pre-generate every fixture (`cargo test -p craft-media --all-features -- --ignored generate_fixtures`).
#[test]
#[ignore]
fn generate_fixtures() {
    let Some((ffmpeg, _)) = tools() else { return };
    for name in ALL_FIXTURES {
        let out = [fixture_dir().join(name)];
        common::generate_and_report(&format!("matroska/{name}"), &out, || fixture(&ffmpeg, name));
    }
}
