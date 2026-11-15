use super::*;
use crate::CodecId;
use craft_media_testkit::vmd::{Audio, Frame as F, Spec, build};

fn sample(audio: bool) -> Vec<u8> {
    let mut spec = Spec::new(16, 8);
    spec.audio = audio.then_some(Audio { sample_rate: 22050, frame_length: 20, buffers: 4, flags: 0 });
    let video = |rect, data: &[u8]| F::video(rect, false, data.to_vec());
    let mut blocks = vec![vec![video([0, 0, 15, 7], &[2; 129])], vec![video([4, 2, 7, 3], &[2; 9])], vec![video([0, 0, 15, 7], &[])]];
    if audio {
        for b in &mut blocks {
            b.insert(0, F::audio(1, vec![0x80; 20]));
        }
    }
    build(&spec, &blocks)
}

#[test]
fn reads_header_and_frames() {
    let d = sample(true);
    assert!(sniff(&d));
    let f = open(&d).unwrap();
    let h = &f.header;
    assert_eq!((h.width, h.height, h.blocks, h.frames_per_block), (16, 8, 3, 2));
    assert_eq!(h.video_format(), VideoFormat::Paletted);
    assert_eq!(h.audio, Some(AudioHeader { sample_rate: 22050, frame_length: 20, bits: 8, channels: 1, buffers: 4, flags: 0 }));
    assert_eq!(h.palette[765..], [63, 63, 63]);
    assert_eq!(f.header_bytes(), &d[..HEADER_LEN]);
    assert_eq!(f.frame_rate(), (22050, 20));

    assert_eq!(f.frames.len(), 6);
    let sizes: Vec<u32> = f.frames.iter().map(|f| f.size).collect();
    assert_eq!(sizes, [20, 129, 20, 9, 20, 0]);
    assert_eq!(f.frames.iter().map(|f| f.block).collect::<Vec<_>>(), [0, 0, 1, 1, 2, 2]);
    // frames follow each other from the block's offset
    assert_eq!(f.frames[0].offset, HEADER_LEN as u64);
    assert_eq!(f.frames[1].offset, HEADER_LEN as u64 + 20);
    assert_eq!(f.frames[3].offset, HEADER_LEN as u64 + 20 + 129 + 20);
    assert_eq!(f.frames[0].kind, FrameKind::Audio { flags: 1 });
    assert_eq!(f.frames[3].kind, FrameKind::Video { left: 4, top: 2, right: 7, bottom: 3, new_palette: false });
    assert_eq!(f.read_frame(&d, 3).unwrap(), [2; 9]);
    assert_eq!(f.read_frame(&d, 5).unwrap(), [0u8; 0]);
    assert!(f.read_frame(&d, 6).is_err());

    let v = f.video_params();
    assert_eq!((v.codec, v.width, v.height, v.extradata.len()), (CodecId::VmdVideo, Some(16), Some(8), HEADER_LEN));
    let a = f.audio_params().unwrap();
    assert_eq!((a.codec, a.sample_rate, a.channels), (CodecId::VmdAudio, Some(22050), Some(1)));
}

#[test]
fn silent_file_plays_ten_pictures_a_second() {
    let f = open(&sample(false)).unwrap();
    assert_eq!(f.header.audio, None);
    assert!(f.audio_params().is_none());
    assert_eq!(f.frame_rate(), (10, 1));
    assert_eq!(f.frames.len(), 3);
}

#[test]
fn sound_formats() {
    let open_with = |frame_length, flags| {
        let mut spec = Spec::new(4, 4);
        spec.audio = Some(Audio { sample_rate: 11025, frame_length, buffers: 1, flags });
        let f = open(&build(&spec, &[])).unwrap();
        let a = f.header.audio.unwrap();
        (a.bits, a.channels, a.block_samples(), f.audio_params().unwrap().codec)
    };
    assert_eq!(open_with(-500, 0), (16, 1, 500, CodecId::VmdAudio));
    // new-style stereo counts one channel, old-style both
    assert_eq!(open_with(-500, 0x200), (16, 2, 500, CodecId::VmdAudio));
    assert_eq!(open_with(500, 0x8000), (8, 2, 250, CodecId::VmdAudio));
    assert_eq!(open_with(i16::MIN, 0), (16, 1, 32768, CodecId::VmdAudio));
    assert_eq!(open_with(-500, 0x10).3, CodecId::Unknown);

    // sound needs the header flag and a sample rate
    let mut d = sample(true);
    d[804..806].fill(0);
    assert_eq!(open(&d).unwrap().header.audio, None);
    let mut d = sample(true);
    d[17] &= !0x10;
    assert_eq!(open(&d).unwrap().header.audio, None);
}

#[test]
fn other_picture_formats_are_not_vmd_video() {
    let params = |patch: fn(&mut Vec<u8>)| {
        let mut d = sample(false);
        patch(&mut d);
        let f = open(&d).unwrap();
        (f.header.video_format(), f.video_params().codec, f.video_params().tag)
    };
    assert_eq!(params(|d| d[4] = 5), (VideoFormat::Rgb24, CodecId::Unknown, None));
    assert_eq!(params(|d| d[4] = 13), (VideoFormat::Rgb16, CodecId::Unknown, None));
    assert_eq!(params(|d| d[4] = 7), (VideoFormat::Indeo3, CodecId::Unknown, Some(*b"IV32")));
    assert_eq!(params(|d| d[24..28].copy_from_slice(b"iv32")).0, VideoFormat::Indeo3);
}

/// The cases ffmpeg was checked on: the first frame as large as the picture places it.
#[test]
fn picture_rect_takes_off_the_picture_position() {
    let rects = |header_origin: (u16, u16), frames: &[[u16; 4]]| {
        let mut spec = Spec::new(20, 10);
        (spec.left, spec.top) = header_origin;
        let blocks: Vec<_> = frames.iter().map(|&rect| vec![F::video(rect, false, vec![])]).collect();
        let f = open(&build(&spec, &blocks)).unwrap();
        (f.origin(), f.frames.iter().map(|fr| f.picture_rect(fr)).collect::<Vec<_>>())
    };
    // a picture at the screen's corner, whatever the header says
    assert_eq!(rects((10, 5), &[[0, 0, 19, 9], [12, 6, 18, 9]]), ((0, 0), vec![Some((0, 0, 19, 9)), Some((12, 6, 18, 9))]));
    // a picture at (3, 2): frames left of or above it, or past its far edges, are outside
    assert_eq!(
        rects((0, 0), &[[3, 2, 22, 11], [3, 2, 5, 5], [0, 0, 5, 5], [3, 2, 23, 11], [3, 2, 22, 12]]),
        ((3, 2), vec![Some((0, 0, 19, 9)), Some((0, 0, 2, 3)), None, None, None])
    );
    // the placing frame need not be the first
    assert_eq!(rects((0, 0), &[[4, 3, 5, 4], [4, 3, 23, 12]]), ((4, 3), vec![Some((0, 0, 1, 1)), Some((0, 0, 19, 9))]));
    // no frame as large as the picture: the corner; inverted and oversized frames are outside
    assert_eq!(rects((7, 7), &[[2, 0, 5, 0], [5, 0, 4, 0], [0, 1, 0, 0], [0, 0, 20, 9]]), ((0, 0), vec![Some((2, 0, 5, 0)), None, None, None]));
}

#[test]
fn rejects_what_is_not_vmd() {
    assert!(matches!(open(&[0u8; 4][..]), Err(Error::NotVmd)));
    assert!(matches!(open(&vec![0u8; 2000]), Err(Error::NotVmd)));
    assert!(!sniff(b"RIFF"));
    // the shorter header of later revisions
    let mut d = sample(false);
    d[0] = 0x32;
    d[1] = 0;
    assert!(matches!(open(&d), Err(Error::NotVmd)));
}

#[test]
fn table_of_contents_must_be_in_the_file() {
    let d = sample(true);
    assert!(matches!(open(&d[..d.len() - 1]), Err(Error::Invalid(_))));
    let mut far = d.clone();
    far[812..816].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(open(&far), Err(Error::Invalid(_))));
    let mut early = d.clone();
    early[812..816].copy_from_slice(&0u32.to_le_bytes());
    assert!(matches!(open(&early), Err(Error::Invalid(_))));
    // 65 535 blocks of 65 535 frames is refused before anything is allocated
    let mut many = d.clone();
    many[6..8].copy_from_slice(&[0xFF; 2]);
    many[18..20].copy_from_slice(&[0xFF; 2]);
    assert!(matches!(open(&many), Err(Error::Invalid(e)) if e.contains("frames in the table")));
}

#[test]
fn frame_past_the_end_is_an_error() {
    let mut d = sample(false);
    let toc = u32::from_le_bytes([d[812], d[813], d[814], d[815]]) as usize;
    // first frame record: its size field
    let record = toc + 3 * 6;
    d[record + 2..record + 6].copy_from_slice(&u32::MAX.to_le_bytes());
    let f = open(&d).unwrap();
    assert!(matches!(f.read_frame(&d, 0), Err(Error::Invalid(_))));
}

#[test]
fn hostile_input_never_panics() {
    for (i, seed) in [sample(true), sample(false)].iter().enumerate() {
        craft_media_testkit::hostile::hammer(&format!("vmd seed {i}"), seed, 30_000, |d| {
            if let Ok(f) = open(d) {
                let n = f.frames.len();
                for k in [0, n / 2, n.saturating_sub(1), n, usize::MAX] {
                    let _ = f.read_frame(d, k);
                }
                for fr in &f.frames {
                    let _ = f.picture_rect(fr);
                }
                let _ = (f.frame_rate(), f.video_params(), f.audio_params());
            }
        });
    }
}
