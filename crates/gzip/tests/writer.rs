use std::io::{self, Read, Write};

use craft_gzip::Gz;
use flate2::{Compression, read::GzDecoder, write::GzEncoder};

fn decode(bytes: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    GzDecoder::new(bytes).read_to_end(&mut result).unwrap();
    result
}

#[test]
fn matches_flate2_without_mode_changes() {
    let data = b"The quick brown fox jumps over the lazy dog. ".repeat(10000);
    let mut expected = GzEncoder::new(Vec::new(), Compression::default());
    let mut actual = Vec::new();
    {
        let mut gz = Gz::new(&mut actual).unwrap();
        for chunk in data.chunks(131) {
            expected.write_all(chunk).unwrap();
            gz.write_all(chunk).unwrap();
        }
        assert_eq!(gz.written(), data.len() as u64);
        gz.finish().unwrap();
    }
    assert_eq!(actual, expected.finish().unwrap());
    assert_eq!(decode(&actual), data);
}

#[test]
fn mixed_segments_round_trip_and_stored_bytes_remain_uncompressed() {
    let plain = b"repeat me ".repeat(20000);
    let packed: Vec<u8> = (0..200_000).map(|i| (i * 31) as u8).collect();
    let mut bytes = Vec::new();
    {
        let mut gz = Gz::new(&mut bytes).unwrap();
        gz.write_all(&plain).unwrap();
        gz.packed(true).unwrap();
        gz.packed(true).unwrap();
        gz.write_all(&packed).unwrap();
        gz.packed(false).unwrap();
        gz.write_all(&plain).unwrap();
        assert_eq!(gz.written(), (plain.len() * 2 + packed.len()) as u64);
        gz.finish().unwrap();
    }
    let expected = [plain.as_slice(), packed.as_slice(), plain.as_slice()].concat();
    assert_eq!(decode(&bytes), expected);
    assert!(bytes.windows(1024).any(|window| window == &packed[..1024]));
    assert!(bytes.len() < expected.len());
}

struct FailAfter {
    remaining: usize,
}
impl Write for FailAfter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::Error::other("injected failure"));
        }
        let n = self.remaining.min(data.len());
        self.remaining -= n;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("injected flush failure"))
    }
}

#[test]
fn propagates_header_body_and_flush_failures() {
    assert!(Gz::new(FailAfter { remaining: 0 }).is_err());
    let mut body = Gz::new(FailAfter { remaining: 10 }).unwrap();
    body.packed(true).unwrap();
    assert!(body.write_all(&vec![42; 200_000]).is_err());
    let empty = Gz::new(FailAfter { remaining: usize::MAX }).unwrap();
    assert_eq!(empty.finish().unwrap_err().to_string(), "injected flush failure");
}
