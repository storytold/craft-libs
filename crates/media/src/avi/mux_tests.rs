//! The AVI muxer read back by this module's demuxer, through every index it writes.

use std::io::Cursor;

use super::*;

/// Chunk `i` of a test stream: sizes vary (odd, even, empty) and the bytes name the chunk.
fn payload(stream: usize, i: usize) -> Vec<u8> {
    let len = match i % 7 {
        0 => 1500 + i * 3,
        3 => 0,
        5 => 1,
        _ => 200 + (i * 37) % 900,
    };
    (0..len).map(|k| (k as u32 ^ (i as u32) << 3 ^ (stream as u32) << 7) as u8).collect()
}

fn video() -> VideoSpec {
    let mut v = VideoSpec::new(*b"XVID", 320, 240, 30000, 1001);
    v.format.extra = vec![0, 0, 1, 0xB0, 3];
    v.name = Some("picture".into());
    v
}

fn pcm() -> AudioSpec {
    let mut a = AudioSpec::pcm(48_000, 2, 16);
    a.name = Some("sound".into());
    a
}

/// Each stream's chunks: bytes and key frame.
type Written = [Vec<(Vec<u8>, bool)>; 2];

/// Write `frames` video frames (every 12th a key frame) each followed by an audio chunk of 4·k
/// bytes, returning the file and what was written per stream.
fn write(frames: usize, opts: MuxOptions) -> (Vec<u8>, Written) {
    let mut w = AviWriter::new(Cursor::new(Vec::new()), vec![StreamSpec::Video(video()), StreamSpec::Audio(pcm())], opts).unwrap();
    let mut written: Written = [Vec::new(), Vec::new()];
    for i in 0..frames {
        let v = payload(0, i);
        let key = i % 12 == 0;
        w.write_chunk(0, &v, key).unwrap();
        written[0].push((v, key));
        let a: Vec<u8> = payload(1, i).into_iter().take(4 * (1 + i % 400)).collect();
        let a = a[..a.len() / 4 * 4].to_vec();
        w.write_chunk(1, &a, true).unwrap();
        written[1].push((a, true));
    }
    (w.finish().unwrap().into_inner(), written)
}

fn tables(file: &[u8], f: &AviFile) -> Written {
    std::array::from_fn(|s| (0..f.streams[s].chunks.len()).map(|i| (f.read_chunk(file, s, i).unwrap(), f.streams[s].chunks[i].key)).collect())
}

/// The top-level chunks of a file: (id, form, size).
fn riffs(file: &[u8]) -> Vec<([u8; 4], [u8; 4], u64)> {
    children(&file, 0, file.len() as u64).into_iter().map(|(id, size, data)| (id, fourcc(file, data as usize), size)).collect()
}

#[test]
fn headers_round_trip() {
    let (file, written) = write(30, MuxOptions::default());
    let f = open(&file).unwrap();
    assert_eq!(f.index, IndexSource::OpenDml);
    assert_eq!(f.header, MainHeader { micro_sec_per_frame: 33367, flags: 0x110, total_frames: 30, streams: 2, width: 320, height: 240 });
    let (v, a) = (&f.streams[0], &f.streams[1]);
    assert_eq!((v.kind, v.handler, v.scale, v.rate, v.length, v.sample_size), (StreamKind::Video, *b"XVID", 1001, 30000, 30, 0));
    assert_eq!(v.video.as_ref(), Some(&video().format));
    assert_eq!(v.name.as_deref(), Some("picture"));
    let audio_bytes: usize = written[1].iter().map(|c| c.0.len()).sum();
    assert_eq!((a.kind, a.scale, a.rate, a.sample_size, a.length as usize), (StreamKind::Audio, 4, 192_000, 4, audio_bytes / 4));
    assert_eq!(a.audio.as_ref(), Some(&pcm().format));
    assert_eq!(a.name.as_deref(), Some("sound"));
    assert_eq!(v.codec_params().codec, crate::CodecId::Mpeg4Video);
    assert_eq!(a.codec_params().codec, crate::CodecId::Pcm);
    // one RIFF: hdrl, movi, idx1
    assert_eq!(riffs(&file).iter().map(|r| (r.0, r.1)).collect::<Vec<_>>(), [(*b"RIFF", *b"AVI ")]);
    assert_eq!(riffs(&file)[0].2 + 8, file.len() as u64);
}

#[test]
fn every_index_describes_the_same_chunks() {
    let (file, written) = write(40, MuxOptions::default());
    for (opts, source) in [
        (OpenOptions::default(), IndexSource::OpenDml),
        (OpenOptions { ignore_odml: true, ignore_idx1: false }, IndexSource::Idx1),
        (OpenOptions { ignore_odml: true, ignore_idx1: true }, IndexSource::Scan),
    ] {
        let f = open_with(&file, opts).unwrap();
        assert_eq!(f.index, source);
        let got = tables(&file, &f);
        assert_eq!(got[1], written[1], "{source:?}: audio");
        if source == IndexSource::Scan {
            // a scan cannot tell key frames
            assert_eq!(got[0].iter().map(|c| &c.0).collect::<Vec<_>>(), written[0].iter().map(|c| &c.0).collect::<Vec<_>>());
        } else {
            assert_eq!(got[0], written[0], "{source:?}: video");
        }
    }
}

#[test]
fn large_files_continue_in_avix_extensions() {
    let opts = MuxOptions { riff_limit: 24 << 10, max_riffs: 64 };
    let (file, written) = write(200, opts);
    let r = riffs(&file);
    assert!(r.len() > 3, "{} RIFFs", r.len());
    assert_eq!((r[0].0, r[0].1), (*b"RIFF", *b"AVI "));
    assert!(r[1..].iter().all(|x| (x.0, x.1) == (*b"RIFF", *b"AVIX")));
    assert!(r.iter().all(|x| x.2 + 8 <= opts.riff_limit), "a RIFF is over the limit: {r:?}");
    assert_eq!(r.iter().map(|x| x.2 + 8).sum::<u64>(), file.len() as u64);

    let f = open(&file).unwrap();
    assert_eq!(f.index, IndexSource::OpenDml);
    assert_eq!(tables(&file, &f), written);
    assert_eq!(f.streams[0].length, 200);
    // avih counts the first RIFF's frames; strh the whole stream
    let first = f.streams[0].chunks.iter().filter(|c| c.offset < r[0].2).count() as u32;
    assert_eq!(f.header.total_frames, first);
    assert!(first < 200);
    // idx1 covers the first RIFF and the AVIX extensions are scanned
    let f = open_with(&file, OpenOptions { ignore_odml: true, ignore_idx1: false }).unwrap();
    assert_eq!(f.index, IndexSource::Idx1);
    assert_eq!(tables(&file, &f)[1], written[1]);
    assert_eq!(f.streams[0].chunks.len(), 200);
}

#[test]
fn a_file_can_start_inside_the_output() {
    let mut out = Cursor::new(b"prefix".to_vec());
    out.set_position(6);
    let mut w = AviWriter::new(out, vec![StreamSpec::Video(video())], MuxOptions::default()).unwrap();
    w.write_chunk(0, &[1, 2, 3], true).unwrap();
    let bytes = w.finish().unwrap().into_inner();
    assert_eq!(&bytes[..6], b"prefix");
    let file = &bytes[6..];
    let f = open(&file).unwrap();
    assert_eq!(f.read_chunk(&file, 0, 0).unwrap(), [1, 2, 3]);
    assert_eq!(f.header.total_frames, 1);
}

#[test]
fn packet_timed_audio_counts_chunks() {
    let mut mp3 = AudioSpec::pcm(44_100, 2, 0);
    mp3.format = WaveFormat {
        format_tag: 0x55,
        channels: 2,
        sample_rate: 44_100,
        avg_bytes_per_sec: 16_000,
        block_align: 1,
        bits_per_sample: 0,
        extra: vec![0; 12],
    };
    mp3.timing = AudioTiming::Packets { samples_per_packet: 1152 };
    let mut w = AviWriter::new(Cursor::new(Vec::new()), vec![StreamSpec::Audio(mp3)], MuxOptions::default()).unwrap();
    for i in 0..10 {
        w.write_chunk(0, &payload(0, i), true).unwrap();
    }
    let file = w.finish().unwrap().into_inner();
    let f = open(&file).unwrap();
    let a = &f.streams[0];
    assert_eq!((a.scale, a.rate, a.sample_size, a.length), (1152, 44_100, 0, 10));
    assert_eq!(a.codec_params().codec, crate::CodecId::Mp3);
    assert_eq!(f.header.width, 0);
}

#[test]
fn codec_descriptions_pick_their_fourcc() {
    let p = crate::CodecParams::new(crate::CodecId::Mpeg4Video).with_size(640, 360).with_extradata(vec![0, 0, 1, 0xB0]);
    let v = VideoSpec::for_codec(&p, 25, 1).unwrap();
    assert_eq!((v.format.compression, v.format.width, v.format.height, &v.format.extra[..]), (*b"FMP4", 640, 360, &[0, 0, 1, 0xB0][..]));
    assert!(VideoSpec::for_codec(&crate::CodecParams::new(crate::CodecId::H264), 25, 1).is_none(), "no size");
    assert!(VideoSpec::for_codec(&crate::CodecParams::new(crate::CodecId::Aac).with_size(1, 1), 25, 1).is_none());
}

#[test]
fn bad_input_is_an_error() {
    let new = |streams: Vec<StreamSpec>, opts| AviWriter::new(Cursor::new(Vec::new()), streams, opts).err();
    let d = MuxOptions::default();
    assert!(new(Vec::new(), d).is_some());
    assert!(new(vec![StreamSpec::Video(video()); 101], d).is_some());
    assert!(new(vec![StreamSpec::Video(video())], MuxOptions { riff_limit: 1000, ..d }).is_some());
    assert!(new(vec![StreamSpec::Video(video())], MuxOptions { riff_limit: 3 << 30, ..d }).is_some());
    assert!(new(vec![StreamSpec::Video(video())], MuxOptions { max_riffs: 0, ..d }).is_some());
    assert!(new(vec![StreamSpec::Video(VideoSpec::new(*b"XVID", 16, 16, 0, 1))], d).is_some());
    let mut a = pcm();
    a.format.block_align = 0;
    assert!(new(vec![StreamSpec::Audio(a)], d).is_some());
    let mut a = pcm();
    a.timing = AudioTiming::Packets { samples_per_packet: 0 };
    assert!(new(vec![StreamSpec::Audio(a)], d).is_some());
    // max_riffs large enough that the header list cannot fit in half a small RIFF
    assert!(new(vec![StreamSpec::Video(video())], MuxOptions { riff_limit: 16 << 10, max_riffs: 1000 }).is_some());

    let mut w = AviWriter::new(Cursor::new(Vec::new()), vec![StreamSpec::Video(video())], MuxOptions { riff_limit: 16 << 10, max_riffs: 2 }).unwrap();
    assert!(w.write_chunk(1, &[0], true).is_err(), "no such stream");
    assert!(w.write_chunk(0, &vec![0; 20 << 10], true).is_err(), "larger than a RIFF");
    w.write_chunk(0, &vec![0; 10 << 10], true).unwrap();
    w.write_chunk(0, &vec![0; 10 << 10], true).unwrap(); // the second RIFF
    assert!(w.write_chunk(0, &vec![0; 10 << 10], true).is_err(), "a third RIFF is past max_riffs");
    // what was written still closes into a readable file
    let file = w.finish().unwrap().into_inner();
    assert_eq!(open(&file).unwrap().streams[0].chunks.len(), 2);
}
