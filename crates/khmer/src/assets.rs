//! Compressed embedded assets. Each buffer initializes once and owns its storage.
use std::{
    io::{self, Write},
    sync::OnceLock,
};

#[derive(Clone, Copy)]
struct Descriptor {
    len: usize,
    packed_crc: u32,
    raw_crc: u32,
}
include!(concat!(env!("OUT_DIR"), "/asset_descriptors.rs"));

struct BoundedOutput {
    data: Vec<u8>,
    limit: usize,
}
impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.data.len()) {
            return Err(io::Error::other("Khmer asset exceeds its expanded length"));
        }
        self.data.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn decode(bytes: &[u8], expected: Descriptor) -> Option<String> {
    // Validate the embedded stream before invoking the decoder: corrupted window
    // headers must not trigger unexpected decoder allocations. Canonical streams
    // are built with lgwin=22; output also has an independent hard ceiling.
    if expected.len > 3 * 1024 * 1024 || crc32fast::hash(bytes) != expected.packed_crc {
        return None;
    }
    let mut output = BoundedOutput { data: Vec::with_capacity(expected.len), limit: expected.len };
    let mut input = bytes;
    brotli_decompressor::BrotliDecompress(&mut input, &mut output).ok()?;
    if output.data.len() != expected.len || crc32fast::hash(&output.data) != expected.raw_crc {
        return None;
    }
    String::from_utf8(output.data).ok()
}
macro_rules! asset {
    ($function:ident, $file:literal, $descriptor:ident) => {
        pub(super) fn $function() -> &'static str {
            static TEXT: OnceLock<String> = OnceLock::new();
            TEXT.get_or_init(|| decode(include_bytes!(concat!(env!("OUT_DIR"), "/", $file, ".br")), $descriptor).unwrap_or_default()).as_str()
        }
    };
}
asset!(frequencies, "frequency.tsv", FREQUENCIES);
asset!(spellings, "km_symspell_dictionary.txt", SPELLINGS);
asset!(variants, "khmer-multiple-spellings.txt", VARIANTS);
asset!(weights, "kcc-weights.tsv", WEIGHTS);
asset!(titles, "titles.txt", TITLES);
asset!(affixes, "affixes.tsv", AFFIXES);

#[cfg(all(test, craft_khmer_data))]
mod tests {
    use super::*;
    #[test]
    fn all_six_assets_are_byte_identical() {
        for (inflated, original) in [
            (frequencies(), include_str!(concat!(env!("OUT_DIR"), "/frequency.tsv"))),
            (spellings(), include_str!(concat!(env!("OUT_DIR"), "/km_symspell_dictionary.txt"))),
            (variants(), include_str!(concat!(env!("OUT_DIR"), "/khmer-multiple-spellings.txt"))),
            (weights(), include_str!(concat!(env!("OUT_DIR"), "/kcc-weights.tsv"))),
            (titles(), include_str!(concat!(env!("OUT_DIR"), "/titles.txt"))),
            (affixes(), include_str!(concat!(env!("OUT_DIR"), "/affixes.tsv"))),
        ] {
            assert_eq!(inflated.as_bytes(), original.as_bytes());
            assert!(!inflated.is_empty());
        }
    }
    #[test]
    fn corrupt_truncated_oversized_and_non_utf8_assets_fail_closed() {
        let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/frequency.tsv.br"));
        assert!(decode(bytes, FREQUENCIES).is_some());
        assert!(decode(bytes, Descriptor { len: FREQUENCIES.len - 1, ..FREQUENCIES }).is_none());
        assert!(decode(bytes, Descriptor { len: FREQUENCIES.len + 1, ..FREQUENCIES }).is_none());
        assert!(decode(bytes, Descriptor { len: usize::MAX, ..FREQUENCIES }).is_none());
        assert!(decode(bytes, Descriptor { raw_crc: FREQUENCIES.raw_crc ^ 1, ..FREQUENCIES }).is_none());
        let truncated = bytes.get(..bytes.len() / 2).unwrap_or_default();
        assert!(decode(truncated, FREQUENCIES).is_none());
        // With matching packed CRC, the decoder still rejects truncation.
        assert!(decode(truncated, Descriptor { packed_crc: crc32fast::hash(truncated), ..FREQUENCIES }).is_none());
        assert!(decode(&[], FREQUENCIES).is_none());
        for at in [0, bytes.len() / 2, bytes.len() - 1] {
            let mut corrupt = bytes.to_vec();
            if let Some(byte) = corrupt.get_mut(at) {
                *byte ^= 0xff;
            }
            assert!(decode(&corrupt, FREQUENCIES).is_none());
        }
        let mut invalid = Vec::new();
        brotli::BrotliCompress(&mut [255u8].as_slice(), &mut invalid, &Default::default()).unwrap();
        assert!(decode(&invalid, Descriptor { len: 1, packed_crc: crc32fast::hash(&invalid), raw_crc: crc32fast::hash(&[255]) }).is_none());
    }
}
