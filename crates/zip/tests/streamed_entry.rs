//! Streamed entries ([`craft_zip::Writer::add_with`]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use craft_zip::{Archive, Compression, Limits, Writer};

/// A MiB of JSON-like text.
fn text() -> Vec<u8> {
    br#"{"id":12,"name":"Card","fill":[0.25,0.5,0.75,1.0],"children":[]},"#.iter().copied().cycle().take(1 << 20).collect()
}

#[test]
fn an_entry_written_a_byte_at_a_time_takes_about_as_long_as_one_added_whole() {
    // serde_json writes a token at a time, and each write reached the deflater as a call of its
    // own: a 21 MB FigCraft document took 3.2 s to save this way, 0.1 s compressed in one piece.
    let data = text();
    let t = Instant::now();
    let mut whole = Writer::new();
    whole.add("document.json", &data, Compression::Level(3)).unwrap();
    whole.finish().unwrap();
    let whole_took = t.elapsed();

    let t = Instant::now();
    let mut w = Writer::new();
    w.add_with("document.json", Compression::Level(3), |out| data.iter().try_for_each(|b| out.write_all(std::slice::from_ref(b)))).unwrap();
    let bytes = w.finish().unwrap();
    let streamed_took = t.elapsed();

    // Unbuffered it was over 40x slower in a debug build; buffered it is within 2x.
    assert!(streamed_took < whole_took * 5 + Duration::from_millis(50), "streamed {streamed_took:?}, whole {whole_took:?}");
    let mut z = Archive::open(&bytes, Limits { max_entries: 4, max_total_bytes: 2 << 20 }).unwrap();
    assert_eq!(z.read("document.json", 2 << 20).unwrap(), Some(data), "the same bytes come back");
}

#[test]
fn a_failed_stream_is_an_error_not_a_partial_entry() {
    let mut w = Writer::new();
    let failed = w.add_with("document.json", Compression::Level(3), |out| {
        out.write_all(b"{\"half\":")?;
        Err(std::io::Error::other("serializer gave up"))
    });
    assert!(failed.is_err());
}
