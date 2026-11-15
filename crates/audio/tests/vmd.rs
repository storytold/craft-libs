//! VMD sound decoder tests. ffmpeg reads VMD but cannot write it, so the files come from the
//! test kit's VMD writer. The decoder must return the samples that were packed, and the same
//! samples as ffmpeg (skipped without ffmpeg).

use std::process::Command;

use craft_audio::AudioError;
use craft_audio::vmd::{Decoder, Error};
use craft_media::vmd::{FrameKind, open};
use craft_media_testkit::hostile::Rng;
use craft_media_testkit::vmd::{Audio, Frame, Spec, build, dpcm_pack};

/// (frame length field, audio flags, channels, 16-bit): every layout the decoder handles.
const FORMATS: [(i16, u16, usize, bool); 6] =
    [(50, 0, 1, false), (50, 0x200, 2, false), (50, 0x8000, 2, false), (-50, 0, 1, true), (-50, 0x200, 2, true), (-50, 0x8000, 2, true)];

fn spec(frame_length: i16, flags: u16) -> Spec {
    let mut spec = Spec::new(4, 4);
    spec.audio = Some(Audio { sample_rate: 22050, frame_length, buffers: 5, flags });
    spec
}

fn decoder(frame_length: i16, flags: u16) -> Decoder {
    Decoder::new(&build(&spec(frame_length, flags), &[])).unwrap()
}

/// One block of random sound: its bytes in the file and the samples it decodes to.
fn block(rng: &mut Rng, d: &Decoder) -> (Vec<u8>, Vec<i16>) {
    let n = d.block_samples() * d.channels();
    if d.bits() == 8 {
        let bytes: Vec<u8> = (0..n).map(|_| rng.next_u64() as u8).collect();
        let samples = bytes.iter().map(|&b| (i16::from(b) - 0x80) << 8).collect();
        return (bytes, samples);
    }
    // a random walk, so that the steps are of every size
    let mut last = [0i32; 2];
    let samples: Vec<i16> = (0..n)
        .map(|i| {
            let bits = 1 + rng.below(15);
            let step = rng.below(1 << bits) as i32 - (1 << (bits - 1));
            let s = &mut last[i % d.channels()];
            *s = (*s + step).clamp(-32768, 32767);
            *s as i16
        })
        .collect();
    dpcm_pack(&samples, d.channels())
}

/// A file of sound frames (with a small picture each, as ffmpeg expects video) and the samples
/// expected of it. The first frame is a mask frame: one silent block, then four of sound.
fn movie(seed: u64, frame_length: i16, flags: u16) -> (Vec<u8>, Vec<i16>) {
    let spec = spec(frame_length, flags);
    let d = decoder(frame_length, flags);
    let mut rng = Rng(seed | 1);
    let mut picture = vec![2u8];
    picture.extend([9; 16]);
    let silence = vec![0i16; d.block_samples() * d.channels()];
    let mut want = Vec::new();
    let mut blocks = Vec::new();
    for n in 0..12 {
        let sound = match n {
            0 => {
                let mut data = 1u32.to_le_bytes().to_vec();
                want.extend_from_slice(&silence);
                for _ in 0..4 {
                    let (bytes, samples) = block(&mut rng, &d);
                    data.extend_from_slice(&bytes);
                    want.extend_from_slice(&samples);
                }
                Frame::audio(2, data)
            }
            _ if rng.below(4) == 0 => {
                want.extend_from_slice(&silence);
                Frame::audio(3, vec![])
            }
            _ => {
                let (bytes, samples) = block(&mut rng, &d);
                want.extend_from_slice(&samples);
                Frame::audio(1, bytes)
            }
        };
        blocks.push(vec![sound, Frame::video([0, 0, 3, 3], false, picture.clone())]);
    }
    (build(&spec, &blocks), want)
}

fn decode_all(bytes: &[u8]) -> Vec<i16> {
    let file = open(bytes).unwrap();
    let d = Decoder::new(file.header_bytes()).unwrap();
    let mut out = Vec::new();
    for (i, frame) in file.frames.iter().enumerate() {
        if let FrameKind::Audio { flags } = frame.kind {
            out.extend(d.decode(flags, &file.read_frame(bytes, i).unwrap()).unwrap());
        }
    }
    out
}

#[test]
fn decodes_the_samples_that_were_packed() {
    for (i, (frame_length, flags, channels, sixteen)) in FORMATS.into_iter().enumerate() {
        let d = decoder(frame_length, flags);
        assert_eq!((d.channels(), d.bits(), d.sample_rate()), (channels, if sixteen { 16 } else { 8 }, 22050), "format {i}");
        // old-style stereo counts both channels in the frame length
        assert_eq!(d.block_samples(), if flags & 0x8000 != 0 { 25 } else { 50 }, "format {i}");
        let (bytes, want) = movie(i as u64 + 1, frame_length, flags);
        assert_eq!(want.len(), 16 * d.block_samples() * channels);
        assert!(decode_all(&bytes) == want, "format {i}");
    }
}

#[test]
fn matches_ffmpeg() {
    let Some(ffmpeg) = craft_media_testkit::ffmpeg_or_skip("vmd sound oracle") else { return };
    let dir = craft_media_testkit::private_dir("vmd-sound");
    for (i, (frame_length, flags, ..)) in FORMATS.into_iter().enumerate() {
        let (bytes, _) = movie(i as u64 + 1, frame_length, flags);
        let path = dir.join(format!("sound{i}.vmd"));
        std::fs::write(&path, &bytes).unwrap();
        let out = Command::new(&ffmpeg).args(["-v", "error", "-i"]).arg(&path).args(["-vn", "-f", "s16le", "-"]).output().unwrap();
        assert!(out.status.success() && out.stderr.is_empty(), "format {i}: ffmpeg: {}", String::from_utf8_lossy(&out.stderr));
        let theirs: Vec<i16> = out.stdout.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
        assert!(theirs == decode_all(&bytes), "format {i} differs from ffmpeg");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn sixteen_bit_steps() {
    // values ffmpeg gave for this block: the first sample, then signed steps from the table
    let mut data = 1000i16.to_le_bytes().to_vec();
    data.extend([1, 1, 1, 0x81, 0x81, 2, 3, 0x7F, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    let d = decoder(-20, 0);
    assert_eq!(d.block_bytes(), 21);
    assert_eq!(
        d.decode(1, &data).unwrap(),
        [1000, 1008, 1016, 1024, 1016, 1008, 1024, 1056, 17440, 1056, 1056, 1056, 1056, 1056, 1056, 1056, 1056, 1056, 1056, 1064]
    );
    // stereo: the channels take turns
    let mut data = [1000i16.to_le_bytes(), (-1000i16).to_le_bytes()].concat();
    data.extend([1, 0x82].repeat(3));
    let d = decoder(-4, 0x200);
    assert_eq!(d.decode(1, &data).unwrap(), [1000, -1000, 1008, -1016, 1016, -1032, 1024, -1048]);
    // a flags 1 frame of several blocks: each block starts from its own first samples
    let (first, first_samples) = dpcm_pack(&[100, -100, 300, -50, 200, 0, 1000, 500], 2);
    let (second, second_samples) = dpcm_pack(&[-7000, 7000, -6000, 6500, -6100, 6400, -3000, 3000], 2);
    assert_eq!(d.decode(1, &[first, second, vec![1, 2, 3]].concat()).unwrap(), [first_samples, second_samples].concat());
    // steps past 16 bits stop at the ends
    let mut data = 30000i16.to_le_bytes().to_vec();
    data.extend([0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    assert_eq!(decoder(-7, 0).decode(1, &data).unwrap(), [30000, 32767, 16383, -1, -16385, -32768, -32768]);
}

#[test]
fn eight_bit_and_silence() {
    let d = decoder(4, 0);
    assert_eq!(d.decode(1, &[0x80, 0x00, 0xFF, 0x70]).unwrap(), [0, -32768, 0x7F00, -0x1000]);
    assert_eq!(d.decode(3, &[]).unwrap(), [0; 4]);
    assert_eq!(d.decode(1, &[]).unwrap(), [0i16; 0]);
    assert_eq!(decoder(4, 0x200).decode(3, &[]).unwrap(), [0; 8]);
    // a flags 1 frame of more than one block plays every whole block
    assert_eq!(
        d.decode(1, &[0x80, 0x81, 0x82, 0x83, 0x90, 0x91, 0x92, 0x93, 0xA0]).unwrap(),
        [0, 0x100, 0x200, 0x300, 0x1000, 0x1100, 0x1200, 0x1300]
    );
    assert_eq!(decoder(-4, 0x8000).decode(3, &[]).unwrap(), [0; 4]);
}

#[test]
fn silence_mask_runs_bit_by_bit() {
    let d = decoder(2, 0);
    let frame = |mask: u32, data: &[u8]| d.decode(2, &[&mask.to_le_bytes()[..], data].concat());
    let (a, b) = ([0x90, 0x91], [0xA0, 0xA1]);
    let (sa, sb) = ([0x1000, 0x1100], [0x2000, 0x2100]);
    assert_eq!(frame(0, &[a, b].concat()).unwrap(), [sa, sb].concat());
    // silent, sound, silent, sound
    assert_eq!(frame(0b0101, &[a, b].concat()).unwrap(), [[0, 0], sa, [0, 0], sb].concat());
    // game files set a bit for every block when all are silent
    assert_eq!(frame(0b0111, &[]).unwrap(), [0; 6]);
    // a trailing part of a block is left out
    assert_eq!(frame(0, &[0x90, 0x91, 0xA0]).unwrap(), sa);
    assert_eq!(d.decode(2, &[0, 0, 0]), Err(Error::Invalid("the silence mask is cut short")));
}

#[test]
fn bad_input_is_an_error() {
    let d = decoder(-20, 0);
    assert_eq!(d.decode(1, &[0; 20]), Err(Error::Invalid("a block of sound is cut short")));
    assert_eq!(d.decode(4, &[0; 21]), Err(Error::Unsupported("unknown sound frame flags")));
    assert_eq!(d.decode(4, &[]), Err(Error::Unsupported("unknown sound frame flags")));
    // in the file API's error: unsupported stays unsupported, the rest is malformed input
    assert!(matches!(AudioError::from(Error::Unsupported("x")), AudioError::Unsupported(_)));
    assert!(matches!(AudioError::from(Error::Invalid("x")), AudioError::Malformed(_)));

    let good = build(&spec(20, 0), &[]);
    let with = |patch: &dyn Fn(&mut Vec<u8>)| {
        let mut h = good.clone();
        patch(&mut h);
        Decoder::new(&h).err()
    };
    assert_eq!(with(&|_| {}), None);
    assert_eq!(with(&|h| h.truncate(815)), Some(Error::Invalid("the header is shorter than 816 bytes")));
    assert_eq!(with(&|h| h[0] = 0), Some(Error::Invalid("not a VMD header")));
    assert_eq!(with(&|h| h[17] = 0), Some(Error::Invalid("the file has no sound")));
    assert_eq!(with(&|h| h[804..806].fill(0)), Some(Error::Invalid("the file has no sound")));
    assert_eq!(with(&|h| h[810] = 0x10), Some(Error::Unsupported("IMA ADPCM sound")));
    // a frame length of nothing decodes to nothing
    let mut empty = good.clone();
    empty[806..808].fill(0);
    let d = Decoder::new(&empty).unwrap();
    assert_eq!(d.decode(1, &[1, 2, 3]).unwrap(), [0i16; 0]);
    assert_eq!(d.decode(2, &[0, 0, 0, 0, 1, 2]).unwrap(), [0i16; 0]);
}

#[test]
fn hostile_input_never_panics() {
    for (i, (frame_length, flags, ..)) in FORMATS.into_iter().enumerate() {
        let (bytes, _) = movie(i as u64 + 1, frame_length, flags);
        craft_media_testkit::hostile::hammer(&format!("vmd sound format {i}"), &bytes, 10_000, |d| {
            let Ok(file) = open(d) else { return };
            let Ok(decoder) = Decoder::new(file.header_bytes()) else { return };
            for (i, frame) in file.frames.iter().enumerate() {
                let FrameKind::Audio { flags } = frame.kind else { continue };
                if let Ok(data) = file.read_frame(d, i) {
                    let _ = decoder.decode(flags, &data);
                }
            }
        });
    }
}

/// Real files, for whoever has some: the sound of every `.vmd` in the directory
/// `CRAFT_MEDIA_VMD_SAMPLES` names must decode to the samples ffmpeg gets. Game files are not
/// redistributable, so none are in the repository and the test is skipped without the variable.
#[test]
fn real_files_match_ffmpeg() {
    let Some(dir) = std::env::var_os("CRAFT_MEDIA_VMD_SAMPLES").filter(|d| !d.is_empty()) else {
        craft_media_testkit::skip_test("vmd samples", "CRAFT_MEDIA_VMD_SAMPLES is not set");
        return;
    };
    let Some(ffmpeg) = craft_media_testkit::ffmpeg_or_skip("vmd sound samples") else { return };
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("vmd")))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no .vmd files in CRAFT_MEDIA_VMD_SAMPLES");
    let (mut files, mut samples) = (0, 0);
    for path in &paths {
        let name = path.display();
        let source = std::fs::File::open(path).unwrap();
        let file = open(&source).unwrap();
        if file.audio_params().is_none() {
            continue;
        }
        let d = Decoder::new(file.header_bytes()).unwrap();
        let mut ours = Vec::new();
        for (i, frame) in file.frames.iter().enumerate() {
            let FrameKind::Audio { flags } = frame.kind else { continue };
            let mut data = file.read_frame(&source, i).unwrap();
            if flags == 2 && data.len() >= 4 {
                // ffmpeg plays a mask frame's silent blocks before its sound; put ours in that
                // order to compare
                let silent = u32::from_le_bytes([data[0], data[1], data[2], data[3]]).count_ones() as usize;
                ours.resize(ours.len() + silent * d.block_samples() * d.channels(), 0);
                data[..4].fill(0);
            }
            ours.extend(d.decode(flags, &data).unwrap_or_else(|e| panic!("{name}: frame {i}: {e}")));
        }
        let out = Command::new(&ffmpeg).args(["-v", "quiet", "-i"]).arg(path).args(["-vn", "-f", "s16le", "-"]).output().unwrap();
        let theirs: Vec<i16> = out.stdout.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
        if let Some(at) = (0..ours.len().max(theirs.len())).find(|&i| ours.get(i) != theirs.get(i)) {
            panic!("{name}: sample {at} differs from ffmpeg ({} samples here, {} there)", ours.len(), theirs.len());
        }
        files += 1;
        samples += ours.len();
    }
    println!("{files} files, {samples} samples match ffmpeg");
}
