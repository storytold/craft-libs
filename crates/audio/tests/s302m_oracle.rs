//! SMPTE 302M (feature `s302m`) against ffmpeg (oracle; skipped when ffmpeg is absent, a failure
//! with `CRAFT_MEDIA_REQUIRE_ORACLES=1`): we decode the packets ffmpeg's encoder puts in a transport
//! stream to the samples ffmpeg decodes, our encoder writes the same bytes, and ffmpeg decodes
//! packets of ours (patched into a transport stream) to our samples.

use craft_audio::s302m::{Encoder, decode, parse_header};
use std::path::Path;
use std::process::Command;

fn signal(channels: usize, frames: usize, seed: u32) -> Vec<i32> {
    let mut x = seed | 1;
    let mut out = Vec::with_capacity(frames * channels);
    for i in 0..frames {
        for c in 0..channels {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let tone = ((i as f64) * (0.03 + 0.011 * c as f64)).sin() * 0.6;
            let noise = f64::from(x % 2000) / 2000.0 - 0.5;
            out.push(((tone + 0.3 * noise) * f64::from(i32::MAX)) as i32);
        }
    }
    out
}

#[test]
fn round_trips_every_size_and_channel_count() {
    for channels in [2usize, 4, 6, 8] {
        for bits in [16u32, 20, 24] {
            let max = 1i64 << (bits - 1);
            let mut e = Encoder::new(channels, bits).expect("encoder");
            let frames = 500;
            let samples: Vec<i32> = signal(channels, frames, 3).iter().map(|&v| (i64::from(v) * max / (1 << 31)) as i32).collect();
            let mut all = Vec::new();
            for chunk in samples.chunks(channels * 193) {
                let p = e.encode(chunk).expect("encode");
                let d = decode(&p).expect("decode");
                assert_eq!((d.header.channels, d.header.bits), (channels, bits));
                all.extend(d.samples);
            }
            assert_eq!(all, samples, "{channels}ch {bits} bits");
        }
    }
    // the extremes
    for bits in [16u32, 20, 24] {
        let (lo, hi) = (-(1i32 << (bits - 1)), (1i32 << (bits - 1)) - 1);
        let mut e = Encoder::new(2, bits).expect("encoder");
        assert_eq!(decode(&e.encode(&[lo, hi, hi, lo, 0, -1]).expect("encode")).expect("decode").samples, vec![lo, hi, hi, lo, 0, -1]);
        assert!(e.encode(&[hi + 1, 0]).is_err());
        assert!(e.encode(&[lo - 1, 0]).is_err());
    }
}

#[test]
fn the_packet_size_limit_and_headers_are_checked() {
    let mut e = Encoder::new(2, 16).expect("encoder");
    let max = e.max_frames();
    assert!(e.encode(&vec![0; max * 2]).is_ok());
    assert!(e.encode(&vec![0; (max + 1) * 2]).is_err());
    let p = Encoder::new(8, 24).expect("encoder").channel_id(0x55).encode(&[0; 8]).expect("encode");
    let h = parse_header(&p).expect("header");
    assert_eq!((h.channels, h.channel_id, h.bits, h.audio_bytes), (8, 0x55, 24, 28));
    assert_eq!(h.frame_bytes(), 28);
}

#[test]
fn damaged_packets_are_errors_never_panics() {
    let mut e = Encoder::new(4, 24).expect("encoder");
    let p = e.encode(&signal(4, 40, 5).iter().map(|&v| v >> 8).collect::<Vec<_>>()).expect("encode");
    for n in 0..p.len() {
        let _ = decode(&p[..n]);
    }
    let mut x = 9u32;
    for i in 0..p.len() {
        let mut d = p.clone();
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        d[i] ^= (x >> 8) as u8 | 1;
        let _ = decode(&d);
    }
    assert!(decode(&[]).is_err());
    assert!(decode(&[0xFF, 0xFF, 0, 0, 1, 2, 3]).is_err());
    assert!(decode(&[0, 5, 0, 0x30, 0, 0, 0, 0, 0]).is_err(), "the reserved size code");
}

// ---- ffmpeg as the reference

fn ffmpeg(ff: &Path, args: &[&str]) {
    let out = Command::new(ff).args(["-v", "error", "-nostdin", "-y"]).args(args).output().expect("run ffmpeg");
    assert!(out.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn s(p: &Path) -> String {
    p.to_str().expect("path").to_string()
}

/// The elementary-stream bytes of every PES packet of PID 0x100 in a transport stream, each as
/// the list of (offset in the file, length) pieces it is spread over.
fn pes_pieces(ts: &[u8]) -> Vec<Vec<(usize, usize)>> {
    let mut out: Vec<Vec<(usize, usize)>> = Vec::new();
    for (i, p) in ts.as_chunks::<188>().0.iter().enumerate() {
        if p[0] != 0x47 || ((usize::from(p[1] & 0x1F) << 8) | usize::from(p[2])) != 0x100 {
            continue;
        }
        let mut off = 4;
        if p[3] & 0x20 != 0 {
            off += 1 + usize::from(p[4]);
        }
        if p[3] & 0x10 == 0 || off >= 188 {
            continue;
        }
        let mut start = off;
        if p[1] & 0x40 != 0 {
            // a new PES packet: skip its header (00 00 01 id, length, flags, flags, header length, header)
            assert_eq!(&p[off..off + 3], &[0, 0, 1]);
            start = off + 9 + usize::from(p[off + 8]);
            out.push(Vec::new());
        }
        if let Some(last) = out.last_mut() {
            last.push((i * 188 + start, 188 - start));
        }
    }
    out
}

fn gather(ts: &[u8], pieces: &[(usize, usize)]) -> Vec<u8> {
    pieces.iter().flat_map(|&(o, n)| ts[o..o + n].iter().copied()).collect()
}

fn pcm_bytes(samples: &[i32], wide: bool) -> Vec<u8> {
    if wide {
        samples.iter().flat_map(|v| v.to_le_bytes()).collect()
    } else {
        samples.iter().flat_map(|&v| ((v >> 16) as i16).to_le_bytes()).collect()
    }
}

fn decoded_by_ffmpeg(ff: &Path, ts: &Path, out: &Path, wide: bool) -> Vec<i32> {
    ffmpeg(ff, &["-i", &s(ts), "-f", if wide { "s32le" } else { "s16le" }, "-acodec", if wide { "pcm_s32le" } else { "pcm_s16le" }, &s(out)]);
    let bytes = std::fs::read(out).expect("read");
    if wide {
        bytes.as_chunks::<4>().0.iter().map(|b| i32::from_le_bytes(*b)).collect()
    } else {
        bytes.as_chunks::<2>().0.iter().map(|b| i32::from(i16::from_le_bytes(*b))).collect()
    }
}

#[test]
fn we_decode_and_encode_what_ffmpeg_encodes_and_ffmpeg_decodes_ours() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("s302m");
    for (channels, bits) in [(2usize, 16u32), (2, 20), (2, 24), (4, 20), (4, 24), (6, 16), (6, 20), (8, 20), (8, 24)] {
        let name = format!("{channels}ch{bits}");
        let frames = 6000;
        let wide = bits > 16;
        let pcm = signal(channels, frames, 17);
        let raw = dir.join(format!("{name}.pcm"));
        std::fs::write(&raw, pcm_bytes(&pcm, wide)).expect("write");
        let ts = dir.join(format!("{name}.ts"));
        let ch_str = channels.to_string();
        let raw_str = s(&raw);
        let ts_str = s(&ts);
        let mut ff_args = vec![
            "-f",
            if wide { "s32le" } else { "s16le" },
            "-ar",
            "48000",
            "-ac",
            &ch_str,
            "-i",
            &raw_str,
            "-af",
            "asetnsamples=n=1024:p=0",
            "-c:a",
            "s302m",
            "-strict",
            "-2",
        ];
        if bits == 20 {
            ff_args.extend(["-bits_per_raw_sample", "20"]);
        }
        ff_args.extend(["-f", "mpegts", &ts_str]);
        ffmpeg(&ff, &ff_args);
        let bytes = std::fs::read(&ts).expect("read");
        let pieces = pes_pieces(&bytes);
        assert!(pieces.len() >= 2, "{name}: {} packets", pieces.len());
        let theirs = decoded_by_ffmpeg(&ff, &ts, &dir.join(format!("{name}.out.pcm")), wide);

        let mut enc = Encoder::new(channels, bits).expect("encoder");
        let mut ours: Vec<i32> = Vec::new();
        let mut frames_seen = 0usize;
        for pes in &pieces {
            // the ES bytes of the packet: its own size field says how much of them is the packet
            let es = gather(&bytes, pes);
            let h = parse_header(&es).expect("header");
            let packet = &es[..4 + h.audio_bytes];
            let d = decode(packet).expect("decode");
            assert_eq!((d.header.channels, d.header.bits), (channels, bits), "{name}");
            // our encoder writes the same bytes ffmpeg's does
            assert!(enc.encode(&d.samples).expect("encode") == packet, "{name}: our packet differs from ffmpeg's at frame {frames_seen}");
            frames_seen += d.samples.len() / channels;
            // 20-bit and 24-bit samples are scaled to 32 bits by ffmpeg's decoder; 16-bit ones are not
            ours.extend(d.samples.iter().map(|&v| if bits == 16 { v } else { v << (32 - bits) }));
        }
        let n = ours.len().min(theirs.len());
        assert!(n >= (frames - 1600) * channels, "{name}: {n} samples");
        assert!(ours[..n] == theirs[..n], "{name}: our decode differs from ffmpeg's");

        // ffmpeg decodes our packets: patch them over the same-sized ones in the stream
        let mine = signal(channels, frames, 23);
        let mut patched = bytes.clone();
        let mut enc = Encoder::new(channels, bits).expect("encoder");
        let mut expect: Vec<i32> = Vec::new();
        for pes in &pieces {
            let es = gather(&bytes, pes);
            let h = parse_header(&es).expect("header");
            let n_frames = h.audio_bytes / h.frame_bytes();
            let at = expect.len();
            let src: Vec<i32> = mine[at..at + n_frames * channels].iter().map(|&v| v >> (32 - bits)).collect();
            let packet = enc.encode(&src).expect("encode");
            assert_eq!(packet.len(), 4 + h.audio_bytes);
            expect.extend(src.iter().map(|&v| if bits == 16 { v } else { v << (32 - bits) }));
            let mut k = 0;
            for &(o, len) in pes {
                let take = len.min(packet.len() - k);
                patched[o..o + take].copy_from_slice(&packet[k..k + take]);
                k += take;
            }
            assert!(k >= packet.len());
        }
        let ours_ts = dir.join(format!("{name}.ours.ts"));
        std::fs::write(&ours_ts, patched).expect("write");
        let theirs = decoded_by_ffmpeg(&ff, &ours_ts, &dir.join(format!("{name}.ours.pcm")), wide);
        let n = expect.len().min(theirs.len());
        assert!(n >= (frames - 1600) * channels, "{name}: {n} samples");
        assert!(expect[..n] == theirs[..n], "{name}: ffmpeg decodes our packets differently");
        println!("{name}: {} PES packets, {n} samples identical both ways", pieces.len());
    }
}
