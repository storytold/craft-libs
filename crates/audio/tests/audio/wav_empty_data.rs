//! A WAV whose data chunk is empty and followed by metadata decodes to no audio; a recording
//! that was never finalised (RIFF and data sizes left at 0) still yields what was written (#466).

use craft_audio::{decode, probe};

fn fmt16(channels: u16, sr: u32) -> Vec<u8> {
    let align = channels * 2;
    let mut f = Vec::new();
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&channels.to_le_bytes());
    f.extend_from_slice(&sr.to_le_bytes());
    f.extend_from_slice(&(sr * u32::from(align)).to_le_bytes());
    f.extend_from_slice(&align.to_le_bytes());
    f.extend_from_slice(&16u16.to_le_bytes());
    f
}

fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut c = id.to_vec();
    c.extend_from_slice(&(data.len() as u32).to_le_bytes());
    c.extend_from_slice(data);
    if data.len() % 2 == 1 {
        c.push(0);
    }
    c
}

/// A finished file: correct RIFF size, chunks as given.
fn riff(chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut body = b"WAVE".to_vec();
    for c in chunks {
        body.extend_from_slice(c);
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

#[test]
fn a_recording_that_was_never_finalised_still_gives_its_audio() {
    let samples: Vec<u8> = (0..400i16).flat_map(|i| (i * 40).to_le_bytes()).collect();
    // RIFF size and data size both left at 0, as a recorder writes them until it finishes.
    let mut file = riff(&[chunk(b"fmt ", &fmt16(2, 48_000)), chunk(b"data", &[])]);
    file.extend_from_slice(&samples);
    file[4..8].copy_from_slice(&0u32.to_le_bytes());
    let (info, buf) = decode(&file, Some("wav")).unwrap();
    assert_eq!(info.frames, 200);
    assert_eq!(buf.frames(), 200);
    // Even if the RIFF size is the file's, audio that is not a run of chunks is still audio.
    let mut file = riff(&[chunk(b"fmt ", &fmt16(2, 48_000)), chunk(b"data", &[])]);
    file.extend_from_slice(&samples);
    let riff_size = (file.len() - 8) as u32;
    file[4..8].copy_from_slice(&riff_size.to_le_bytes());
    assert_eq!(probe(&file, Some("wav")).unwrap().frames, 200);
}

/// Whether an empty `data` chunk is followed by chunks alone is decided by walking the rest of the
/// file. Doing that for every empty `data` chunk made parsing quadratic in their number (minutes
/// for this 4 MB file); only the first one is used.
#[test]
fn many_empty_data_chunks_parse_in_linear_time() {
    let mut b = b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0".to_vec();
    b.extend_from_slice(&[1, 0, 1, 0, 0x80, 0xBB, 0, 0, 0, 0x77, 1, 0, 2, 0, 16, 0]);
    b.extend_from_slice(&b"data\0\0\0\0".repeat(100_000));
    b.extend_from_slice(&b"junk\0\0\0\0".repeat(400_000));
    let n = (b.len() - 8) as u32;
    b[4..8].copy_from_slice(&n.to_le_bytes());
    let t = std::time::Instant::now();
    let info = probe(&b, Some("wav")).unwrap();
    assert!(t.elapsed() < std::time::Duration::from_secs(20), "parse took {:?}", t.elapsed());
    // The chunks after the first empty `data` chunk are metadata, not samples.
    assert_eq!(info.frames, 0);
    assert_eq!(decode(&b, Some("wav")).unwrap().1.frames(), 0);
}

/// fmt (mono 16-bit), an empty `data` chunk, then an 8-byte `LIST` chunk (#2123).
fn empty_data_then_list() -> Vec<u8> {
    riff(&[chunk(b"fmt ", &fmt16(1, 8000)), chunk(b"data", &[]), chunk(b"LIST", b"INFOabcd")])
}

/// An empty `data` chunk followed by a `LIST` chunk holds no audio, whether the RIFF size counts
/// the `LIST` chunk or was written before it was appended (#2123): the `LIST` chunk's bytes are
/// never decoded as samples.
#[test]
fn an_empty_data_chunk_followed_by_a_list_chunk_is_no_audio_whatever_the_riff_size() {
    let correct = empty_data_then_list();
    // The RIFF size a writer put down before appending the LIST chunk.
    let mut stale = correct.clone();
    let before_list = (stale.len() - 8 - 16) as u32;
    stale[4..8].copy_from_slice(&before_list.to_le_bytes());
    // A RIFF size past the end of the file.
    let mut past = correct.clone();
    past[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    for (name, file) in [("correct", &correct), ("stale", &stale), ("past the end", &past)] {
        assert_eq!(probe(file, Some("wav")).unwrap().frames, 0, "{name} RIFF size: probe");
        assert_eq!(decode(file, Some("wav")).unwrap().1.frames(), 0, "{name} RIFF size: decode");
    }
}

/// Bytes after an empty `data` chunk that only look like the start of a chunk are a recording that
/// was never finalised: a `LIST` id whose size runs past the file, or a cut-off header, stays
/// audio, as the bytes of PCM can spell any id.
#[test]
fn an_empty_data_chunk_followed_by_a_broken_chunk_header_is_still_audio() {
    let base = riff(&[chunk(b"fmt ", &fmt16(1, 8000)), chunk(b"data", &[])]);
    let stale = |tail: &[u8]| {
        let mut f = base.clone();
        f.extend_from_slice(tail);
        f
    };
    // A LIST id whose size runs past the end of the file: 12 bytes, 6 frames.
    assert_eq!(decode(&stale(b"LIST\0\x10\0\0INFO"), Some("wav")).unwrap().1.frames(), 6);
    // A LIST id with a cut-off size field: 6 bytes, 3 frames.
    assert_eq!(decode(&stale(b"LIST\0\0"), Some("wav")).unwrap().1.frames(), 3);
}

/// A WAV with samples in a sized `data` chunk and a `LIST` chunk after it decodes exactly as
/// before: its samples, not the `LIST` chunk.
#[test]
fn a_sized_data_chunk_followed_by_a_list_chunk_decodes_its_samples() {
    let samples: Vec<i16> = (0..64).map(|i| i * 300 - 9000).collect();
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let file = riff(&[chunk(b"fmt ", &fmt16(1, 8000)), chunk(b"data", &bytes), chunk(b"LIST", b"INFOabcd")]);
    let (info, buf) = decode(&file, Some("wav")).unwrap();
    assert_eq!(info.frames, 64);
    assert_eq!(buf.frames(), 64);
    let mut stale = file.clone();
    let before_list = (stale.len() - 8 - 16) as u32;
    stale[4..8].copy_from_slice(&before_list.to_le_bytes());
    let (_, stale_buf) = decode(&stale, Some("wav")).unwrap();
    assert_eq!(stale_buf.frames(), 64, "a stale RIFF size does not change a sized data chunk");
    assert_eq!(stale_buf, buf, "the same samples");
    let expected: Vec<f32> = samples.iter().map(|s| f32::from(*s) / 32768.0).collect();
    assert_eq!(buf.interleaved(), expected, "the data chunk's samples");
}
