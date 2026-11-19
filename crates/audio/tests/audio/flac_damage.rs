//! A damaged FLAC keeps its length and timing: the frames that were lost are silence.

use craft_audio::{AudioBuffer, BitDepth, DecodeOptions, Decoder, EncodeFormat, EncodeOptions, decode, encode};

const FRAMES: usize = 48_000;
/// The encoder's block size: the frames one FLAC frame holds.
const BLOCK: usize = 4096;

fn tone() -> AudioBuffer {
    let mut b = AudioBuffer::new(48_000, 1, FRAMES);
    for (i, s) in b.channels[0].iter_mut().enumerate() {
        *s = ((8000.0 * (i as f64 * 0.05).sin()).round() / 32768.0) as f32;
    }
    b
}

fn flac() -> Vec<u8> {
    encode(&tone(), &EncodeOptions::new(EncodeFormat::Flac).with_bit_depth(BitDepth::Int16).with_dither(false)).unwrap()
}

fn crc(data: &[u8], bits: u32, poly: u32) -> u32 {
    let mut c = 0u32;
    for &b in data {
        c ^= u32::from(b) << (bits - 8);
        for _ in 0..8 {
            c = if c & (1 << (bits - 1)) != 0 { (c << 1) ^ poly } else { c << 1 };
        }
    }
    c & ((1 << bits) - 1)
}

/// Where each frame of `flac()` starts (and the end of the file): a sync code whose header has a
/// valid CRC-8. Frame numbers below 128 are one byte; the short last block stores its size in two
/// more.
fn frame_starts(bytes: &[u8]) -> Vec<usize> {
    let header = |i: usize| {
        let len = if bytes.get(i + 2)? >> 4 == 7 { 7 } else { 5 };
        (bytes.get(i..i + 2)? == [0xFF, 0xF8] && crc(bytes.get(i..i + len)?, 8, 0x07) == u32::from(*bytes.get(i + len)?)).then_some(i)
    };
    let mut starts: Vec<usize> = (0..bytes.len()).filter_map(header).collect();
    assert_eq!(starts.len(), FRAMES.div_ceil(BLOCK));
    starts.push(bytes.len());
    starts
}

/// `flac()` with `shift` added to the number of every frame from `from` on (the new numbers are 128
/// to 2047: two bytes), checksums corrected: its timestamps jump although no frame is damaged.
fn renumbered(from: usize, shift: u32) -> Vec<u8> {
    let bytes = flac();
    let starts = frame_starts(&bytes);
    let mut out = bytes[..starts[from]].to_vec();
    for (at, w) in starts.windows(2).enumerate().skip(from) {
        let (start, end) = (w[0], w[1]);
        let header = if bytes[start + 2] >> 4 == 7 { 7 } else { 5 };
        let number = at as u32 + shift;
        let mut frame = bytes[start..start + 4].to_vec();
        frame.extend_from_slice(&[0xC0 | (number >> 6) as u8, 0x80 | (number & 0x3F) as u8]);
        frame.extend_from_slice(&bytes[start + 5..start + header]);
        frame.push(crc(&frame, 8, 0x07) as u8);
        frame.extend_from_slice(&bytes[start + header + 1..end - 2]);
        frame.extend_from_slice(&(crc(&frame, 16, 0x8005) as u16).to_be_bytes());
        out.extend_from_slice(&frame);
    }
    out
}

/// `buf` is the tone with exactly the frames in `lost` silent.
fn assert_silent_in(buf: &AudioBuffer, lost: std::ops::Range<usize>) {
    let src = tone();
    assert_eq!(buf.frames(), FRAMES);
    for (i, (a, b)) in src.channels[0].iter().zip(&buf.channels[0]).enumerate() {
        assert_eq!(*b, if lost.contains(&i) { 0.0 } else { *a }, "frame {i}");
    }
}

#[test]
fn an_intact_flac_reports_no_damage() {
    let (info, buf) = decode(&flac(), Some("flac")).unwrap();
    assert_eq!(info.damaged, 0);
    assert_silent_in(&buf, 0..0);
}

/// The FLAC demuxer drops a frame whose checksum fails, and the audio after it used to come out
/// one block early.
#[test]
fn a_corrupt_flac_frame_becomes_silence_and_is_counted() {
    let mut bytes = flac();
    let starts = frame_starts(&bytes);
    bytes[starts[4] + 20] ^= 0xFF;
    let (info, buf) = decode(&bytes, Some("flac")).unwrap();
    assert_eq!((info.frames, info.damaged), (FRAMES as u64, 1));
    assert_silent_in(&buf, 4 * BLOCK..5 * BLOCK);
}

#[test]
fn a_run_of_corrupt_flac_frames_is_one_gap() {
    let mut bytes = flac();
    let starts = frame_starts(&bytes);
    for frame in 3..6 {
        bytes[starts[frame] + 20] ^= 0xFF;
    }
    let (info, buf) = decode(&bytes, Some("flac")).unwrap();
    assert_eq!((info.frames, info.damaged), (FRAMES as u64, 1));
    assert_silent_in(&buf, 3 * BLOCK..6 * BLOCK);
}

/// A seek lands on the right audio before the loss, inside it and after it.
#[test]
fn seeking_a_damaged_flac_keeps_its_timing() {
    let mut bytes = flac();
    let starts = frame_starts(&bytes);
    bytes[starts[4] + 20] ^= 0xFF;
    let src = tone();
    for target in [0, 3 * BLOCK + 7, 4 * BLOCK, 4 * BLOCK + 100, 5 * BLOCK + 1, 9 * BLOCK] {
        let mut dec = Decoder::from_bytes(&bytes, Some("flac"), &DecodeOptions::default()).unwrap();
        assert_eq!(dec.seek(target as u64).unwrap(), target as u64);
        let mut got = Vec::new();
        while let Some(block) = dec.next_block().unwrap() {
            got.extend_from_slice(&block.channels[0]);
        }
        assert_eq!(got.len(), FRAMES - target, "from {target}");
        for (i, (a, b)) in got.iter().zip(&src.channels[0][target..]).enumerate() {
            let lost = (4 * BLOCK..5 * BLOCK).contains(&(target + i));
            assert_eq!(*a, if lost { 0.0 } else { *b }, "from {target}: frame {}", target + i);
        }
    }
}

/// A timestamp that jumps past the declared length is not padded: the file says how long it is.
#[test]
fn a_jump_past_the_declared_length_is_not_silence() {
    let (info, buf) = decode(&renumbered(5, 300), Some("flac")).unwrap();
    assert_eq!(info.damaged, 0);
    assert!(buf.frames() <= FRAMES, "{} frames", buf.frames());
}

/// With no declared length, a jump of up to 2^20 frames is lost audio; a longer one is not believed.
#[test]
fn an_undeclared_length_bounds_the_silence() {
    let unknown = |mut bytes: Vec<u8>| {
        // STREAMINFO's 36-bit total sample count: the low nibble of byte 21 and bytes 22 to 25
        bytes[21] &= 0xF0;
        bytes[22..26].fill(0);
        bytes
    };
    // 200 frames of 4096 (819 200 samples) are missing before frame 5
    let (info, buf) = decode(&unknown(renumbered(5, 200)), Some("flac")).unwrap();
    assert_eq!((info.damaged, buf.frames()), (1, FRAMES + 200 * BLOCK));
    assert!(buf.channels[0][5 * BLOCK..205 * BLOCK].iter().all(|&s| s == 0.0));
    assert_eq!(buf.channels[0][205 * BLOCK..], tone().channels[0][5 * BLOCK..]);
    // 300 frames are 1 228 800 samples, more than 2^20: the audio stays where it was
    let (info, buf) = decode(&unknown(renumbered(5, 300)), Some("flac")).unwrap();
    assert_eq!(info.damaged, 0);
    assert_silent_in(&buf, 0..0);
}
