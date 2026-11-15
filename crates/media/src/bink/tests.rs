use super::*;

/// One frame of a hand-built file: its audio packets (decoded size and data) and video packet.
struct TestFrame {
    key: bool,
    audio: Vec<Option<(u32, Vec<u8>)>>,
    video: Vec<u8>,
}

/// Builds a Bink file with one `(sample rate, flags)` pair per audio track.
fn bink(signature: &[u8; 4], tracks: &[(u16, u16)], frames: &[TestFrame]) -> Vec<u8> {
    let bodies: Vec<Vec<u8>> = frames
        .iter()
        .map(|f| {
            let mut b = Vec::new();
            for a in &f.audio {
                match a {
                    None => b.extend_from_slice(&0u32.to_le_bytes()),
                    Some((decoded, data)) => {
                        b.extend_from_slice(&(data.len() as u32 + 4).to_le_bytes());
                        b.extend_from_slice(&decoded.to_le_bytes());
                        b.extend_from_slice(data);
                    }
                }
            }
            b.extend_from_slice(&f.video);
            // bit 0 of an index entry is the key frame flag, so a frame starts at an even offset
            assert_eq!(b.len() % 4, 0, "frames are built in whole 32-bit words");
            b
        })
        .collect();
    let data_start = HEADER_LEN + tracks.len() * 12 + (frames.len() + 1) * 4;
    let total = data_start + bodies.iter().map(Vec::len).sum::<usize>();
    let largest = bodies.iter().map(Vec::len).max().unwrap_or(0);
    let mut d = signature.to_vec();
    for v in [total - 8, frames.len(), largest, frames.len(), 320, 240, 2997, 100, 1 << 20, tracks.len()] {
        d.extend_from_slice(&(v as u32).to_le_bytes());
    }
    for &(_, flags) in tracks {
        d.extend_from_slice(&0x0e00u16.to_le_bytes());
        d.extend_from_slice(&(if flags & (1 << 13) != 0 { 2u16 } else { 1 }).to_le_bytes());
    }
    for &(rate, flags) in tracks {
        d.extend_from_slice(&rate.to_le_bytes());
        d.extend_from_slice(&flags.to_le_bytes());
    }
    for t in 0..tracks.len() {
        d.extend_from_slice(&(t as u32 + 7).to_le_bytes());
    }
    let mut at = data_start;
    for (f, b) in frames.iter().zip(&bodies) {
        d.extend_from_slice(&(at as u32 | u32::from(f.key)).to_le_bytes());
        at += b.len();
    }
    d.extend_from_slice(&(at as u32).to_le_bytes());
    for b in &bodies {
        d.extend_from_slice(b);
    }
    assert_eq!(d.len(), total);
    d
}

fn sample() -> Vec<u8> {
    bink(
        b"BIKi",
        &[(22050, 0x7000), (44100, 0x4000)],
        &[
            TestFrame { key: true, audio: vec![Some((1024, vec![1, 2, 3, 4])), Some((512, vec![9; 8]))], video: vec![0xAA; 16] },
            TestFrame { key: false, audio: vec![None, Some((512, vec![8; 4]))], video: vec![0xBB; 12] },
            TestFrame { key: false, audio: vec![None, None], video: vec![0xCC; 20] },
        ],
    )
}

#[test]
fn reads_header_tracks_and_frames() {
    let d = sample();
    assert!(sniff(&d));
    let f = open(&d).unwrap();
    assert_eq!(f.header.signature, *b"BIKi");
    assert_eq!(f.header.revision(), b'i');
    assert_eq!((f.header.frames, f.header.width, f.header.height), (3, 320, 240));
    assert_eq!((f.header.fps_num, f.header.fps_den), (2997, 100));
    assert!(f.header.has_alpha());
    assert!(!f.header.grayscale());
    assert_eq!(f.header.scaling(), 0);
    assert_eq!(f.header.largest_frame, 8 + 4 + 8 + 8 + 16);

    assert_eq!(f.audio.len(), 2);
    assert_eq!((f.audio[0].sample_rate, f.audio[0].stereo(), f.audio[0].dct(), f.audio[0].id), (22050, true, true, 7));
    assert_eq!((f.audio[1].sample_rate, f.audio[1].stereo(), f.audio[1].dct(), f.audio[1].id), (44100, false, false, 8));
    let a = f.audio[0].codec_params();
    assert_eq!((a.codec, a.sample_rate, a.channels), (crate::CodecId::BinkAudioDct, Some(22050), Some(2)));
    let a = f.audio[1].codec_params();
    assert_eq!((a.codec, a.sample_rate, a.channels), (crate::CodecId::BinkAudioRdft, Some(44100), Some(1)));

    let v = f.video_params();
    assert_eq!((v.codec, v.tag, v.width, v.height), (crate::CodecId::BinkVideo, Some(*b"BIKi"), Some(320), Some(240)));
    assert_eq!(v.extradata, (1u32 << 20).to_le_bytes());

    assert_eq!(f.frames.iter().map(|f| f.key).collect::<Vec<_>>(), [true, false, false]);
    assert_eq!(f.frames.iter().map(|f| f.size).collect::<Vec<_>>(), [44, 28, 28]);
    assert_eq!(f.frames.last().map(|f| f.offset + u64::from(f.size)), Some(d.len() as u64));
}

#[test]
fn splits_a_frame_into_its_packets() {
    let d = sample();
    let f = open(&d).unwrap();
    let frame = f.read_frame(&d, 0).unwrap();
    let p = f.split_frame(&frame).unwrap();
    assert_eq!(p.audio[0], Some(AudioPacket { decoded_len: 1024, data: &[1, 2, 3, 4] }));
    assert_eq!(p.audio[1], Some(AudioPacket { decoded_len: 512, data: &[9; 8] }));
    assert_eq!(p.video, [0xAA; 16]);

    let frame = f.read_frame(&d, 1).unwrap();
    let p = f.split_frame(&frame).unwrap();
    assert_eq!(p.audio, [None, Some(AudioPacket { decoded_len: 512, data: &[8; 4] })]);
    assert_eq!(p.video, [0xBB; 12]);

    let frame = f.read_frame(&d, 2).unwrap();
    let p = f.split_frame(&frame).unwrap();
    assert_eq!(p.audio, [None, None]);
    assert_eq!(p.video, [0xCC; 20]);

    assert!(matches!(f.read_frame(&d, 3), Err(Error::Invalid(_))));
}

#[test]
fn a_file_without_audio_has_only_video_packets() {
    let d = bink(b"BIKf", &[], &[TestFrame { key: true, audio: vec![], video: vec![1, 2, 3, 4] }]);
    let f = open(&d).unwrap();
    assert!(f.audio.is_empty());
    let frame = f.read_frame(&d, 0).unwrap();
    assert_eq!(f.split_frame(&frame).unwrap(), FramePackets { audio: vec![], video: &[1, 2, 3, 4] });
}

#[test]
fn a_truncated_file_opens_with_its_whole_frames() {
    let d = sample();
    let f = open(&d[..d.len() - 1]).unwrap();
    assert_eq!(f.frames.len(), 2);
    assert_eq!(f.header.frames, 3);
    // cut inside the index: no frame is known to be whole
    let index_at = HEADER_LEN + 24;
    assert_eq!(open(&d[..index_at + 6]).unwrap().frames.len(), 0);
}

#[test]
fn refuses_what_is_not_a_bink_1_file() {
    assert!(matches!(open(&b"RIFF"[..]), Err(Error::NotBink)));
    assert!(matches!(open(&[][..]), Err(Error::NotBink)));
    assert!(matches!(open(&[0u8; 64][..]), Err(Error::NotBink)));
    assert!(matches!(open(&b"BIKi"[..]), Err(Error::Invalid(_))));
    let mut d = sample();
    d[..4].copy_from_slice(b"KB2g");
    assert!(matches!(open(&d), Err(Error::Bink2)));
    assert!(!sniff(b"KB2g"));
    assert!(!sniff(b"BIK"));
    assert!(!sniff(b"BIK1"));
}

#[test]
fn refuses_invalid_headers() {
    let set = |at: usize, v: u32| {
        let mut d = sample();
        d[at..at + 4].copy_from_slice(&v.to_le_bytes());
        open(&d).map(|_| ())
    };
    assert!(matches!(set(8, 0), Err(Error::Invalid(_))), "no frames");
    assert!(matches!(set(20, 0), Err(Error::Invalid(_))), "zero width");
    assert!(matches!(set(24, 32768), Err(Error::Invalid(_))), "height past the limit");
    assert!(matches!(set(28, 0), Err(Error::Invalid(_))), "zero frame rate");
    assert!(matches!(set(32, 0), Err(Error::Invalid(_))), "zero frame rate divider");
    assert!(matches!(set(40, 257), Err(Error::Invalid(_))), "too many audio tracks");
    assert!(matches!(set(40, 200), Err(Error::Invalid(_))), "audio headers past the end");
    // a frame that starts inside the index, and one that ends before it starts
    let index_at = HEADER_LEN + 24;
    assert!(matches!(set(index_at, 8), Err(Error::Invalid(_))));
    assert!(matches!(set(index_at + 4, 0), Err(Error::Invalid(_))));
}

#[test]
fn a_huge_frame_count_allocates_no_more_than_the_file_holds() {
    let mut d = sample();
    d[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    // the index now claims to run far past the file, so every offset lies inside it
    assert!(matches!(open(&d), Err(Error::Invalid(_))));
}

#[test]
fn split_frame_refuses_cut_off_packets() {
    assert!(matches!(split_frame(&[1, 0, 0], 1), Err(Error::Invalid(_))));
    assert!(matches!(split_frame(&[9, 0, 0, 0, 1, 2], 1), Err(Error::Invalid(_))));
    // a length too short to hold the decoded size
    assert!(matches!(split_frame(&[2, 0, 0, 0, 1, 2], 1), Err(Error::Invalid(_))));
    assert!(matches!(split_frame(&[], usize::MAX), Err(Error::Invalid(_))));
    assert_eq!(split_frame(&[], 0).unwrap().video, [0u8; 0]);
}

#[test]
fn hostile_input_never_panics() {
    let seeds = [sample(), bink(b"BIKf", &[], &[TestFrame { key: true, audio: vec![], video: vec![1, 2, 3, 4] }])];
    for (i, s) in seeds.iter().enumerate() {
        craft_media_testkit::hostile::hammer(&format!("bink seed {i}"), s, 30_000, |d| {
            if let Ok(f) = open(d) {
                let n = f.frames.len();
                for k in [0, n / 2, n.saturating_sub(1), n, usize::MAX] {
                    if let Ok(frame) = f.read_frame(d, k) {
                        let _ = f.split_frame(&frame);
                    }
                }
            }
        });
    }
}
