use super::*;
use craft_media_testkit::smacker::{BitWriter, Tree};

/// No limit below the module's own.
const LIMIT: u32 = MAX_DECODED_SIZE as u32;

/// Encodes interleaved 8-bit samples as a DPCM chunk.
fn encode_u8(samples: &[u8], channels: usize) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.bit(true);
    w.bit(channels == 2);
    w.bit(false);
    let deltas: Vec<Vec<u8>> = (0..channels)
        .map(|c| samples.iter().skip(c).step_by(channels).collect::<Vec<_>>().windows(2).map(|p| p[1].wrapping_sub(*p[0])).collect())
        .collect();
    let trees: Vec<Tree<u8>> = deltas.iter().map(|d| tree_of(d)).collect();
    for t in &trees {
        t.write_optional(&mut w);
    }
    for c in (0..channels).rev() {
        w.bits(u32::from(samples[c]), 8);
    }
    for i in channels..samples.len() {
        let c = i % channels;
        trees[c].put(&mut w, deltas[c][i / channels - 1]);
    }
    chunk(samples.len(), w)
}

/// Encodes interleaved 16-bit samples as a DPCM chunk.
fn encode_s16(samples: &[i16], channels: usize) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.bit(true);
    w.bit(channels == 2);
    w.bit(true);
    let deltas: Vec<Vec<[u8; 2]>> = (0..channels)
        .map(|c| samples.iter().skip(c).step_by(channels).collect::<Vec<_>>().windows(2).map(|p| p[1].wrapping_sub(*p[0]).to_le_bytes()).collect())
        .collect();
    // for each channel a tree of the low bytes, then one of the high bytes
    let trees: Vec<[Tree<u8>; 2]> = deltas.iter().map(|d| [0, 1].map(|b| tree_of(&d.iter().map(|x| x[b]).collect::<Vec<_>>()))).collect();
    for t in trees.iter().flatten() {
        t.write_optional(&mut w);
    }
    for c in (0..channels).rev() {
        let [low, high] = samples[c].to_le_bytes();
        w.bits(u32::from(high), 8);
        w.bits(u32::from(low), 8);
    }
    for i in channels..samples.len() {
        let c = i % channels;
        let [low, high] = deltas[c][i / channels - 1];
        trees[c][0].put(&mut w, low);
        trees[c][1].put(&mut w, high);
    }
    chunk(samples.len() * 2, w)
}

fn tree_of(values: &[u8]) -> Tree<u8> {
    let mut v = values.to_vec();
    v.sort_unstable();
    v.dedup();
    if v.is_empty() {
        v.push(0);
    }
    Tree::new(&v)
}

fn chunk(size: usize, w: BitWriter) -> Vec<u8> {
    let mut out = (size as u32).to_le_bytes().to_vec();
    out.extend(w.finish());
    out
}

fn wave(n: usize, step: i32) -> Vec<i16> {
    (0..n as i32).map(|i| ((i * step) % 40_000 - 20_000) as i16).collect()
}

#[test]
fn decodes_eight_bit_mono_and_stereo() {
    let mono: Vec<u8> = (0..200u32).map(|i| (128 + (i * 7 % 50)) as u8).collect();
    let got = decode(&encode_u8(&mono, 1), LIMIT).unwrap();
    assert_eq!((got.channels, &got.samples), (1, &Samples::U8(mono)));

    // the channels differ, and the differences wrap around
    let stereo: Vec<u8> = (0..200u32).map(|i| if i % 2 == 0 { (i * 3) as u8 } else { 255 - (i * 5) as u8 }).collect();
    let got = decode(&encode_u8(&stereo, 2), LIMIT).unwrap();
    assert_eq!((got.channels, &got.samples), (2, &Samples::U8(stereo)));
}

#[test]
fn decodes_sixteen_bit_mono_and_stereo() {
    let mono = wave(300, 1237);
    let got = decode(&encode_s16(&mono, 1), LIMIT).unwrap();
    assert_eq!((got.channels, &got.samples), (1, &Samples::S16(mono)));

    let mut stereo = wave(300, 911);
    stereo.iter_mut().skip(1).step_by(2).for_each(|s| *s = s.wrapping_mul(-3));
    stereo[0] = i16::MIN;
    stereo[1] = 0x1234;
    let got = decode(&encode_s16(&stereo, 2), LIMIT).unwrap();
    assert_eq!((got.channels, &got.samples), (2, &Samples::S16(stereo)));
}

#[test]
fn a_constant_signal_takes_no_bits_a_sample() {
    // one-symbol trees: every difference is 0
    let got = decode(&encode_u8(&[77; 1000], 1), LIMIT).unwrap();
    assert_eq!(got.samples, Samples::U8(vec![77; 1000]));
    // only the first sample of each channel
    let got = decode(&encode_s16(&[-5, 9], 2), LIMIT).unwrap();
    assert_eq!(got.samples, Samples::S16(vec![-5, 9]));
}

#[test]
fn a_chunk_without_data_is_empty() {
    let got = decode(&[8, 0, 0, 0, 0], LIMIT).unwrap();
    assert!(got.samples.is_empty());
    assert_eq!(got.samples.len(), 0);
}

#[test]
fn eight_bit_samples_convert_to_sixteen() {
    assert_eq!(Samples::U8(vec![0, 128, 255]).to_i16(), [-32768, 0, 32512]);
    assert_eq!(Samples::S16(vec![-1, 1]).to_i16(), [-1, 1]);
}

#[test]
fn reads_uncompressed_chunks() {
    let c = pcm(&[1, 2, 3, 4, 5], true, true);
    assert_eq!((c.channels, c.samples), (2, Samples::S16(vec![0x0201, 0x0403])));
    let c = pcm(&[1, 2, 3], false, false);
    assert_eq!((c.channels, c.samples), (1, Samples::U8(vec![1, 2, 3])));
    // a trailing part of a stereo frame is dropped, as half a 16-bit sample is
    assert_eq!(pcm(&[1, 2, 3], false, true).samples, Samples::U8(vec![1, 2]));
    assert_eq!(pcm(&[1, 2, 3, 4, 5, 6, 7], true, true).samples, Samples::S16(vec![0x0201, 0x0403]));
    assert_eq!(pcm(&[1, 2, 3], true, false).samples, Samples::S16(vec![0x0201]));
    assert!(pcm(&[], true, true).samples.is_empty());
}

#[test]
fn bad_chunks_are_errors() {
    assert_eq!(decode(&[1, 0, 0], LIMIT), Err(Error::Truncated));
    assert_eq!(decode(&[1, 0, 0, 0], LIMIT), Err(Error::Truncated));
    let good = encode_s16(&wave(64, 999), 2);
    for n in 4..good.len() {
        assert_eq!(decode(&good[..n], LIMIT), Err(Error::Truncated), "{n} bytes");
    }
    // a size over the limit, however little data follows
    let mut big = good.clone();
    big[..4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(decode(&big, u32::MAX), Err(Error::TooLarge(u32::MAX)));
    // a size that is not a whole number of stereo frames, and one with no first sample
    let mut odd = good.clone();
    odd[..4].copy_from_slice(&6u32.to_le_bytes());
    assert_eq!(decode(&odd, LIMIT), Err(Error::Invalid("sample count")));
    let mut none = good;
    none[..4].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(decode(&none, LIMIT), Err(Error::Invalid("sample count")));
}

#[test]
fn a_declared_size_is_bounded_whatever_the_data_holds() {
    // one-symbol trees code a sample in no bits, so the size field alone sets the output
    let mut chunk = encode_u8(&[1; 4], 1);
    chunk[..4].copy_from_slice(&(MAX_DECODED_SIZE as u32).to_le_bytes());
    assert_eq!(decode(&chunk, LIMIT).unwrap().samples.len(), MAX_DECODED_SIZE);
    chunk[..4].copy_from_slice(&(MAX_DECODED_SIZE as u32 + 1).to_le_bytes());
    assert_eq!(decode(&chunk, u32::MAX), Err(Error::TooLarge(MAX_DECODED_SIZE as u32 + 1)));
    // the caller's limit, to the byte
    chunk[..4].copy_from_slice(&1000u32.to_le_bytes());
    assert_eq!(decode(&chunk, 1000).unwrap().samples.len(), 1000);
    assert_eq!(decode(&chunk, 999), Err(Error::TooLarge(1000)));
    // 0 is a header that does not say: no limit but the module's own
    assert_eq!(decode(&chunk, 0).unwrap().samples.len(), 1000);
    chunk[..4].copy_from_slice(&(MAX_DECODED_SIZE as u32 + 1).to_le_bytes());
    assert_eq!(decode(&chunk, 0), Err(Error::TooLarge(MAX_DECODED_SIZE as u32 + 1)));
}

#[test]
fn a_chunk_cut_short_does_not_reserve_its_declared_size() {
    // real trees, so every sample takes at least a bit: the buffer follows the data, not the size
    let mut chunk = encode_s16(&wave(64, 999), 1);
    chunk.truncate(40);
    chunk[..4].copy_from_slice(&LIMIT.to_le_bytes());
    assert_eq!(decode(&chunk, LIMIT), Err(Error::Truncated));
    let bits = (chunk.len() - 4) * 8;
    assert!(super::capacity_for(LIMIT as usize / 2, bits, 1) <= bits + 1);
}

#[test]
fn hostile_input_never_panics() {
    let mut stereo = wave(400, 911);
    stereo.iter_mut().skip(1).step_by(2).for_each(|s| *s = s.wrapping_mul(-3));
    let mono: Vec<u8> = (0..400u32).map(|i| (i * 7 % 50) as u8).collect();
    for (name, seed) in [("smacker audio s16", encode_s16(&stereo, 2)), ("smacker audio u8", encode_u8(&mono, 1))] {
        craft_media_testkit::hostile::hammer(name, &seed, 20_000, |d| {
            let _ = decode(d, LIMIT);
            let _ = pcm(d, true, true);
        });
    }
}

#[test]
fn declared_size_requires_complete_sample_frames() {
    for (channels, sixteen, seed) in
        [(1, false, encode_u8(&[7], 1)), (2, false, encode_u8(&[7, 9], 2)), (1, true, encode_s16(&[7], 1)), (2, true, encode_s16(&[7, 9], 2))]
    {
        let frame_bytes = channels * if sixteen { 2 } else { 1 };
        assert!(decode(&seed, LIMIT).is_ok());
        for size in 0..2 * frame_bytes {
            if size == frame_bytes {
                continue;
            }
            let mut malformed = seed.clone();
            malformed[..4].copy_from_slice(&(size as u32).to_le_bytes());
            assert_eq!(decode(&malformed, LIMIT), Err(Error::Invalid("sample count")), "channels={channels}, sixteen={sixteen}, size={size}");
        }
    }
}
