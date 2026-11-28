use super::*;
use crate::bitstream::LsbBitReader;
use craft_media_testkit::smacker::{BitWriter, FileSpec, Tree, file, frame};

/// A file of three frames and a ring frame: 8x4, one compressed 16-bit stereo track in slot 0
/// and an uncompressed 8-bit mono track in slot 2.
fn sample() -> Vec<u8> {
    let mut rates = [0u32; 7];
    rates[0] = 0xF0 << 24 | 22_050;
    rates[2] = 0x40 << 24 | 11_025;
    file(&FileSpec {
        signature: *b"SMK2",
        width: 8,
        height: 4,
        frame_rate: -6666,
        flags: Header::RING_FRAME,
        audio_rates: rates,
        tree_sizes: [4, 8, 12, 16],
        trees: &[1, 2, 3, 4, 5],
        frames: &[
            (0b0000_1011, frame(Some(&[0x01, 0x02, 0x03, 0xFF, 0xFF]), &[&[9, 9, 9], &[7, 7]], &[0xAA, 0xBB])),
            (0, vec![]),
            (0b0000_0010, frame(None, &[&[5; 8]], &[0xCC; 4])),
            (0, vec![0xDD; 4]),
        ],
    })
}

#[test]
fn opens_header_tables_and_trees() {
    let bytes = sample();
    assert!(sniff(&bytes));
    let f = open(&bytes).unwrap();
    let h = &f.header;
    assert_eq!((h.version, h.width, h.height, h.frames), (Version::Smk2, 8, 4, 3));
    assert_eq!(h.frame_period(), (6666, 100_000));
    assert!(h.has_ring_frame());
    assert_eq!((h.trees_size, h.tree_sizes), (5, [4, 8, 12, 16]));
    assert_eq!(f.trees, [1, 2, 3, 4, 5]);
    assert!(!f.truncated);

    let a = &h.audio[0];
    assert!(a.is_present());
    assert_eq!((a.sample_rate, a.bits(), a.channels(), a.coding), (22_050, 16, 2, AudioCoding::Dpcm));
    let b = &h.audio[2];
    assert_eq!((b.sample_rate, b.bits(), b.channels(), b.coding), (11_025, 8, 1, AudioCoding::Pcm));
    assert!(!h.audio[1].is_present());

    assert_eq!(f.frames.len(), 3);
    let data_start = (HEADER_LEN + 4 * 5 + 5) as u64;
    assert_eq!(f.frames[0].offset, data_start);
    assert!(f.frames[0].has_palette);
    assert_eq!(f.frames[0].audio_mask, 0b101);
    assert_eq!(f.frames[1].size, 0);
    assert_eq!(f.frames[1].offset, f.frames[2].offset);
    let ring = f.ring_frame.unwrap();
    assert_eq!(ring.offset + u64::from(ring.size), bytes.len() as u64);
    assert_eq!(f.read(&bytes, &ring).unwrap(), [0xDD; 4]);
    assert!(f.read_frame(&bytes, 3).is_err());
}

#[test]
fn splits_a_frame_into_palette_audio_and_video() {
    let bytes = sample();
    let f = open(&bytes).unwrap();
    let data = f.read_frame(&bytes, 0).unwrap();
    let parts = split_frame(&data, &f.frames[0]).unwrap();
    // the palette chunk is padded to a multiple of four with its length byte
    assert_eq!(parts.palette, Some(&[0x01, 0x02, 0x03, 0xFF, 0xFF, 0, 0][..]));
    assert_eq!(parts.audio[0], Some(&[9, 9, 9][..]));
    assert_eq!(parts.audio[1], None);
    assert_eq!(parts.audio[2], Some(&[7, 7][..]));
    assert_eq!(&parts.video[..2], [0xAA, 0xBB]);

    let data = f.read_frame(&bytes, 1).unwrap();
    let empty = split_frame(&data, &f.frames[1]).unwrap();
    assert_eq!((empty.palette, empty.video.len()), (None, 0));

    let data = f.read_frame(&bytes, 2).unwrap();
    let parts = split_frame(&data, &f.frames[2]).unwrap();
    assert_eq!((parts.audio[0], parts.video), (Some(&[5u8; 8][..]), &[0xCC; 4][..]));
}

#[test]
fn split_frame_rejects_chunks_that_overrun_the_frame() {
    let f = |has_palette, audio_mask| Frame { offset: 0, size: 0, keyframe: false, has_palette, audio_mask };
    assert!(split_frame(&[], &f(true, 0)).is_err());
    // a palette length of 0 units, and one past the frame
    assert!(split_frame(&[0, 1, 2, 3], &f(true, 0)).is_err());
    assert!(split_frame(&[2, 1, 2, 3], &f(true, 0)).is_err());
    // an audio length below its own four bytes, past the frame, and cut short
    assert!(split_frame(&[3, 0, 0, 0, 1], &f(false, 1)).is_err());
    assert!(split_frame(&[9, 0, 0, 0, 1], &f(false, 1)).is_err());
    assert!(split_frame(&[4, 0], &f(false, 1)).is_err());
    assert!(split_frame(&[4, 0, 0, 0], &f(false, 0b11)).is_err());
    let ok = split_frame(&[4, 0, 0, 0, 8], &f(false, 1)).unwrap();
    assert_eq!((ok.audio[0], ok.video), (Some(&[][..]), &[8][..]));
}

#[test]
fn the_size_table_carries_the_key_frame_flag_in_its_low_bits() {
    let mut bytes = sample();
    let plain = open(&bytes).unwrap();
    // bit 0 on the first frame, bit 1 (not a key frame) on the third
    bytes[HEADER_LEN] |= 1;
    bytes[HEADER_LEN + 8] |= 2;
    let f = open(&bytes).unwrap();
    assert_eq!(f.frames.iter().map(|x| x.keyframe).collect::<Vec<_>>(), [true, false, false]);
    for (a, b) in f.frames.iter().zip(&plain.frames) {
        assert_eq!((a.offset, a.size), (b.offset, b.size));
    }
    assert_eq!(f.ring_frame.map(|r| r.offset), plain.ring_frame.map(|r| r.offset));
}

#[test]
fn audio_tracks_carry_the_header_size_field() {
    let mut bytes = sample();
    bytes[24..28].copy_from_slice(&1234u32.to_le_bytes());
    bytes[32..36].copy_from_slice(&77u32.to_le_bytes());
    let h = open(&bytes).unwrap().header;
    assert_eq!(h.audio.map(|a| a.size_hint), [1234, 0, 77, 0, 0, 0, 0]);
}

#[test]
fn frame_period_forms() {
    let mut h = open(&sample()).unwrap().header;
    for (raw, want) in [(0, (1, 10)), (100, (100, 1000)), (-3333, (3333, 100_000)), (i32::MIN, (1 << 31, 100_000))] {
        h.frame_rate = raw;
        assert_eq!(h.frame_period(), want);
    }
}

#[test]
fn video_params_carry_the_tree_sizes_and_data() {
    let f = open(&sample()).unwrap();
    let p = f.video_params();
    assert_eq!((p.codec, p.tag, p.width, p.height), (crate::CodecId::SmackerVideo, Some(*b"SMK2"), Some(8), Some(4)));
    assert_eq!(p.extradata, [4, 0, 0, 0, 8, 0, 0, 0, 12, 0, 0, 0, 16, 0, 0, 0, 1, 2, 3, 4, 5]);
    let a = f.header.audio[0].codec_params();
    assert_eq!((a.codec, a.sample_rate, a.channels), (crate::CodecId::SmackerAudio, Some(22_050), Some(2)));
    assert_eq!(a.pcm, Some(crate::PcmFormat::int(16, true, false)));
    assert_eq!(f.header.audio[2].codec_params().pcm, Some(crate::PcmFormat::int(8, false, false)));
}

#[test]
fn bink_audio_tracks_are_told_apart() {
    for (flags, want) in [(0xC8u32, AudioCoding::BinkRdft), (0xC4, AudioCoding::BinkDct), (0xCC, AudioCoding::BinkRdft)] {
        let t = AudioTrack::parse(flags << 24 | 44_100, 0);
        assert_eq!(t.coding, want);
        assert_eq!(t.codec_params().codec, crate::CodecId::Unknown);
    }
}

#[test]
fn rejects_what_is_not_a_smacker_file() {
    assert!(matches!(open(&b"RIFF"[..]), Err(Error::NotSmacker)));
    assert!(matches!(open(&b"SM"[..]), Err(Error::NotSmacker)));
    assert!(matches!(open(&b"SMK2 short"[..]), Err(Error::Invalid(_))));
    let mut zero = sample();
    zero[4..8].fill(0);
    assert!(matches!(open(&zero), Err(Error::Invalid(_))));
    // a frame count and a tree size the file cannot hold
    let mut many = sample();
    many[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(open(&many), Err(Error::Invalid(_))));
    let mut trees = sample();
    trees[52..56].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(open(&trees), Err(Error::Invalid(_))));
}

#[test]
fn a_file_cut_short_opens_with_the_frames_it_has() {
    let bytes = sample();
    let f = open(&bytes[..bytes.len() - 6]).unwrap();
    assert!(f.truncated);
    assert_eq!(f.frames.len(), 2);
    assert_eq!(f.ring_frame, None);
    // an SMK4 signature
    let mut v4 = bytes;
    v4[3] = b'4';
    assert_eq!(open(&v4).unwrap().header.version, Version::Smk4);
}

#[test]
fn byte_tree_reads_what_the_writer_wrote() {
    let symbols: Vec<u8> = (0..=255).rev().collect();
    for n in [1usize, 2, 3, 7, 256] {
        let tree = Tree::new(&symbols[..n]);
        let mut w = BitWriter::new();
        tree.write_optional(&mut w);
        for &s in &symbols[..n] {
            tree.put(&mut w, s);
        }
        let bytes = w.finish();
        let mut r = LsbBitReader::new(&bytes);
        let read = ByteTree::read_optional(&mut r).unwrap();
        for &s in &symbols[..n] {
            assert_eq!(read.decode(&mut r), Ok(s), "{n} symbols");
        }
    }
    // an absent tree decodes 0 from no bits
    let mut r = LsbBitReader::new(&[0]);
    let none = ByteTree::read_optional(&mut r).unwrap();
    assert_eq!((none.decode(&mut r), r.position()), (Ok(0), 1));
    assert_eq!(ByteTree::single(9).decode(&mut r), Ok(9));
}

#[test]
fn byte_tree_bounds_its_size_and_the_data() {
    // branches without end
    assert_eq!(ByteTree::read(&mut LsbBitReader::new(&[0xFF; 100])), Err(TreeError::TooLarge));
    assert_eq!(ByteTree::read(&mut LsbBitReader::new(&[0xFF; 4])), Err(TreeError::Eof));
    assert_eq!(ByteTree::read(&mut LsbBitReader::new(&[])), Err(TreeError::Eof));
    // a code that runs out of data
    let tree = Tree::new(&[1u8, 2, 3]);
    let mut w = BitWriter::new();
    tree.write(&mut w);
    let bytes = w.finish();
    let mut r = LsbBitReader::new(&bytes);
    let read = ByteTree::read(&mut r).unwrap();
    let mut end = LsbBitReader::new(&[]);
    assert!(read.decode(&mut end).is_err());
}

#[test]
fn hostile_input_never_panics() {
    craft_media_testkit::hostile::hammer("smacker", &sample(), 30_000, |d| {
        if let Ok(f) = open(d) {
            for frame in f.frames.iter().chain(&f.ring_frame) {
                if let Ok(data) = f.read(d, frame) {
                    let _ = split_frame(&data, frame);
                }
            }
            let _ = f.video_params();
            let _ = f.read_frame(d, usize::MAX);
        }
        let mut r = LsbBitReader::new(d);
        if let Ok(t) = ByteTree::read_optional(&mut r) {
            for _ in 0..64 {
                let _ = t.decode(&mut r);
            }
        }
    });
}
