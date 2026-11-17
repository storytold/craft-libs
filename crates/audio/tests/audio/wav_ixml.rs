//! iXML timestamp parity with FilmCraft's streaming WAV reader, including hostile metadata.
use std::io::Cursor;

use craft_audio::{DecodeOptions, Decoder};
use craft_media::ReadSeekSource;

fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = id.to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    if !body.len().is_multiple_of(2) {
        out.push(0);
    }
    out
}

fn wav(metadata: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"RIFF\0\0\0\0WAVE".to_vec();
    out.extend(chunk(b"fmt ", &[1, 0, 1, 0, 0x80, 0xbb, 0, 0, 0, 0x77, 1, 0, 2, 0, 16, 0]));
    for m in metadata {
        out.extend(m);
    }
    out.extend(chunk(b"data", &[0, 0, 1, 0]));
    let len = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&len.to_le_bytes());
    out
}

fn xml(low: &str, high: Option<&str>) -> Vec<u8> {
    let high = high.map(|v| format!("<BWF_TIME_REFERENCE_HIGH>{v}</BWF_TIME_REFERENCE_HIGH>")).unwrap_or_default();
    chunk(b"iXML", format!("<BWFXML><BEXT><BWF_TIME_REFERENCE_LOW>{low}</BWF_TIME_REFERENCE_LOW>{high}</BEXT></BWFXML>").as_bytes())
}

fn check(file: Vec<u8>, expected: Option<u64>) {
    let options = DecodeOptions::default();
    let mut bytes = Decoder::from_bytes(&file, Some("wav"), &options).unwrap();
    assert_eq!(bytes.info().bwf.as_ref().map(|b| b.time_reference), expected);
    let source = ReadSeekSource::new(Cursor::new(file.clone())).unwrap();
    let mut stream = Decoder::open(source, Some("wav"), &options).unwrap();
    assert_eq!(stream.info().bwf.as_ref().map(|b| b.time_reference), expected);
    assert_eq!(bytes.info().frames, 2);
    assert_eq!(stream.next_block().unwrap().unwrap().frames(), 2);
    assert_eq!(bytes.next_block().unwrap().unwrap().frames(), 2);
}

#[test]
fn ixml_fallback_preserves_large_timestamps_and_optional_high_word() {
    check(wav(&[xml(" 5 ", Some("1"))]), Some((1 << 32) + 5));
    check(wav(&[xml("0", None)]), Some(0));
    check(wav(&[xml("4294967295", Some("4294967295"))]), Some(u64::MAX));
}

#[test]
fn bext_wins_in_either_chunk_order_including_timestamp_zero() {
    let bext = chunk(b"bext", &[0; 602]);
    let ixml = xml("5", Some("1"));
    check(wav(&[bext.clone(), ixml.clone()]), Some(0));
    check(wav(&[ixml.clone(), bext.clone()]), Some(0));
    check(wav(&[bext, chunk(b"bext", &[0; 12]), ixml.clone()]), Some(0));
    check(wav(&[chunk(b"bext", &[0; 12]), ixml]), Some((1 << 32) + 5));
}

#[test]
fn invalid_metadata_never_fabricates_a_timestamp_or_blocks_audio() {
    for low in ["4294967296", "-1", "+1", "", "1.5", "<x>5</x>"] {
        check(wav(&[xml(low, Some("0"))]), None);
    }
    for high in ["4294967296", "-1", "", "oops"] {
        check(wav(&[xml("5", Some(high))]), None);
    }
    for text in [
        "<BWFXML><BEXT><BWF_TIME_REFERENCE_LOW>5</BWF_TIME_REFERENCE_LOW>",
        "<BWFXML><!-- <BWF_TIME_REFERENCE_LOW>5</BWF_TIME_REFERENCE_LOW> --></BWFXML>",
        "<BWFXML><BEXT><BWF_TIME_REFERENCE_LOW>5</BWF_TIME_REFERENCE_LOW><BWF_TIME_REFERENCE_LOW>6</BWF_TIME_REFERENCE_LOW></BEXT></BWFXML>",
        "<!DOCTYPE BWFXML><BWFXML><BEXT><BWF_TIME_REFERENCE_LOW>5</BWF_TIME_REFERENCE_LOW></BEXT></BWFXML>",
    ] {
        check(wav(&[chunk(b"iXML", text.as_bytes())]), None);
    }
    check(wav(&[chunk(b"iXML", &vec![b' '; (1 << 20) + 1])]), None);
    let deep = format!("<BWFXML>{}{}</BWFXML>", "<a>".repeat(40), "</a>".repeat(40));
    check(wav(&[chunk(b"iXML", deep.as_bytes())]), None);
}

#[test]
fn only_first_ixml_document_is_considered() {
    check(wav(&[xml("5", None), xml("6", None)]), Some(5));
}
