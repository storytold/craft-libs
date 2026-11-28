//! Channel labels, text metadata and padding in the DSF and DSDIFF readers and writers.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use craft_audio::dsd::{encode_dsdiff, encode_dsf, read_metadata, read_raw};

const RATE: u32 = 2_822_400;

fn planes(channels: usize, len: usize) -> Vec<Vec<u8>> {
    vec![vec![0x69; len]; channels]
}

fn refs(p: &[Vec<u8>]) -> Vec<&[u8]> {
    p.iter().map(Vec::as_slice).collect()
}

#[test]
fn dsdiff_names_surround_channels_by_layout() {
    for (n, names) in [
        (5, vec!["Mono/Left", "Right", "Center", "LeftSurround", "RightSurround"]),
        (6, vec!["Mono/Left", "Right", "Center", "LFE", "LeftSurround", "RightSurround"]),
    ] {
        let p = planes(n, 4);
        let bytes = encode_dsdiff(RATE, &refs(&p), 32).unwrap();
        assert_eq!(read_metadata(&bytes).unwrap().channel_names, names, "{n} channels");
    }
}

#[test]
fn dsdiff_odd_audio_is_padded_and_counted_in_the_file_size() {
    let p = planes(1, 1);
    let bytes = encode_dsdiff(RATE, &refs(&p), 8).unwrap();
    assert_eq!(bytes.len() % 2, 0);
    assert_eq!(u64::from_be_bytes(bytes[4..12].try_into().unwrap()) + 12, bytes.len() as u64);
    assert_eq!(*bytes.last().unwrap(), 0, "pad byte");
    assert_eq!(read_raw(&bytes).unwrap().channel_bytes, vec![vec![0x69]]);
}

fn with_diin(mut bytes: Vec<u8>, sub: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut sub_chunk = Vec::new();
    sub_chunk.extend_from_slice(sub);
    sub_chunk.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    sub_chunk.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        sub_chunk.push(0);
    }
    bytes.extend_from_slice(b"DIIN");
    bytes.extend_from_slice(&(sub_chunk.len() as u64).to_be_bytes());
    bytes.extend_from_slice(&sub_chunk);
    let size = (bytes.len() - 12) as u64;
    bytes[4..12].copy_from_slice(&size.to_be_bytes());
    bytes
}

#[test]
fn dsdiff_title_and_artist_skip_the_character_count() {
    let p = planes(2, 4);
    let base = encode_dsdiff(RATE, &refs(&p), 32).unwrap();
    let mut payload = 4u32.to_be_bytes().to_vec();
    payload.extend_from_slice(b"Test");
    let bytes = with_diin(base.clone(), b"DITI", &payload);
    assert_eq!(read_metadata(&bytes).unwrap().title.as_deref(), Some("Test"));
    let bytes = with_diin(base.clone(), b"DIAR", &payload);
    assert_eq!(read_metadata(&bytes).unwrap().artist.as_deref(), Some("Test"));
    // A count longer than the chunk is not text.
    let mut bad = 99u32.to_be_bytes().to_vec();
    bad.extend_from_slice(b"Test");
    assert_eq!(read_metadata(&with_diin(base, b"DITI", &bad)).unwrap().title, None);
}

#[test]
fn dsf_channel_types_match_the_channel_count() {
    for (n, expect) in [(1, 1), (2, 2), (3, 3), (4, 4), (5, 6), (6, 7)] {
        let p = planes(n, 4);
        let bytes = encode_dsf(RATE, &refs(&p), 32, 4096).unwrap();
        assert_eq!(u32::from_le_bytes(bytes[48..52].try_into().unwrap()), expect, "{n} channels");
        assert_eq!(read_metadata(&bytes).unwrap().channel_names.len(), n);
    }
    let p = planes(7, 4);
    assert!(encode_dsf(RATE, &refs(&p), 32, 4096).is_err());
}

#[test]
fn dsf_type_5_names_the_fourth_channel_lfe() {
    let p = planes(4, 4);
    let mut bytes = encode_dsf(RATE, &refs(&p), 32, 4096).unwrap();
    bytes[48..52].copy_from_slice(&5u32.to_le_bytes());
    assert_eq!(read_metadata(&bytes).unwrap().channel_names, ["FrontLeft", "FrontRight", "Center", "LFE"]);
}

fn id3(version: u8, frames: &[u8]) -> Vec<u8> {
    let n = frames.len();
    let mut tag = vec![b'I', b'D', b'3', version, 0, 0];
    tag.extend_from_slice(&[(n >> 21) as u8 & 0x7F, (n >> 14) as u8 & 0x7F, (n >> 7) as u8 & 0x7F, n as u8 & 0x7F]);
    tag.extend_from_slice(frames);
    tag
}

fn frame(version: u8, id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let n = body.len();
    let mut f = id.to_vec();
    if version >= 4 {
        f.extend_from_slice(&[(n >> 21) as u8 & 0x7F, (n >> 14) as u8 & 0x7F, (n >> 7) as u8 & 0x7F, n as u8 & 0x7F]);
    } else {
        f.extend_from_slice(&(n as u32).to_be_bytes());
    }
    f.extend_from_slice(&[0, 0]);
    f.extend_from_slice(body);
    f
}

fn dsf_with_tag(tag: &[u8]) -> Vec<u8> {
    let p = planes(2, 4);
    let mut bytes = encode_dsf(RATE, &refs(&p), 32, 4096).unwrap();
    let offset = bytes.len() as u64;
    bytes[20..28].copy_from_slice(&offset.to_le_bytes());
    bytes.extend_from_slice(tag);
    let file_size = bytes.len() as u64;
    bytes[12..20].copy_from_slice(&file_size.to_le_bytes());
    bytes
}

#[test]
fn id3v24_frame_sizes_are_synchsafe() {
    // A 128-byte frame has the synchsafe size 00 00 01 00; read as plain big-endian it is 256.
    let mut frames = frame(4, b"APIC", &[0u8; 128]);
    let mut title = vec![3];
    title.extend_from_slice(b"After art");
    frames.extend(frame(4, b"TIT2", &title));
    let m = read_metadata(&dsf_with_tag(&id3(4, &frames))).unwrap();
    assert_eq!(m.title.as_deref(), Some("After art"));
}

#[test]
fn id3v23_frame_sizes_stay_big_endian() {
    let mut frames = frame(3, b"APIC", &[0u8; 128]);
    let mut title = vec![3];
    title.extend_from_slice(b"Three");
    frames.extend(frame(3, b"TIT2", &title));
    let m = read_metadata(&dsf_with_tag(&id3(3, &frames))).unwrap();
    assert_eq!(m.title.as_deref(), Some("Three"));
}

#[test]
fn id3_text_encodings() {
    let cases: [(&[u8], &str); 5] = [
        (b"\x00Caf\xe9", "Café"),
        (b"\x03Caf\xc3\xa9", "Café"),
        (b"\x02\x00A\x00B", "AB"),
        (b"\x01\xFF\xFEA\x00B\x00", "AB"),
        (b"\x01\xFE\xFF\x00A\x00B", "AB"),
    ];
    for (body, expect) in cases {
        let m = read_metadata(&dsf_with_tag(&id3(4, &frame(4, b"TIT2", body)))).unwrap();
        assert_eq!(m.title.as_deref(), Some(expect), "{body:?}");
    }
}
