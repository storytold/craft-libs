//! Property tests: arbitrary bytes, and valid files with bytes changed, cut or inserted, are
//! read through every call of the API without a panic and in bounded time.

use std::sync::OnceLock;

use proptest::prelude::*;

use super::writer::{Layout, build};
use super::{exercise, sample};
use crate::{Encryption, Format, Limits};

fn bases() -> &'static [Vec<u8>] {
    static BASES: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    BASES.get_or_init(|| {
        vec![
            build(&sample(Layout::tiny(Format::Unicode, Encryption::Permute), None)),
            build(&sample(Layout::tiny(Format::Ansi, Encryption::Cyclic), None)),
            build(&sample(Layout::new(Format::Unicode, Encryption::None), None)),
        ]
    })
}

/// Small enough that a hostile file can't make a case slow.
fn limits() -> Limits {
    Limits { max_item_bytes: 1 << 20, max_attachment_bytes: 1 << 20, max_inline_value: 1 << 16, ..Default::default() }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, ..ProptestConfig::default() })]

    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        exercise(&bytes, limits());
    }

    #[test]
    fn a_header_with_anything_after_it_never_panics(tail in proptest::collection::vec(any::<u8>(), 0..20_000), which in 0usize..3) {
        let base = &bases()[which];
        let mut bytes = base[..0x4400.min(base.len())].to_vec();
        bytes.extend_from_slice(&tail);
        exercise(&bytes, limits());
    }

    #[test]
    fn changed_bytes_never_panic(which in 0usize..3, changes in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..24)) {
        let mut bytes = bases()[which].clone();
        let len = bytes.len();
        for (at, v) in changes {
            bytes[at % len] = v;
        }
        exercise(&bytes, limits());
    }

    #[test]
    fn changed_bytes_past_the_checksums_never_panic(which in 0usize..3, changes in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..8)) {
        // The changes go to blocks and pages with their CRCs put right, so that what is behind
        // the checksums (the trees, heaps and tables) meets damaged structures.
        let mut bytes = bases()[which].clone();
        let format = if which == 1 { Format::Ansi } else { Format::Unicode };
        for (at, v) in changes {
            let at = 0x4400 + at % (bytes.len() - 0x4400);
            bytes[at] = v;
            super::properties::fix_around(&mut bytes, format, at);
        }
        exercise(&bytes, limits());
    }

    #[test]
    fn cut_files_never_panic(which in 0usize..3, cut in any::<usize>()) {
        let base = &bases()[which];
        exercise(&base[..cut % base.len()], limits());
    }
}

/// Puts right the CRC of the page or block that `at` falls in, when it can tell which.
pub fn fix_around(bytes: &mut [u8], format: Format, at: usize) {
    let ts = format.trailer_size();
    // A page: 512-aligned, whose trailer names a B-tree page type twice.
    let page = at - at % 512;
    if page + 512 <= bytes.len() {
        let t = page + 512 - ts;
        if matches!(bytes[t], 0x80 | 0x81) && bytes[t] == bytes[t + 1] {
            let crc = crc32(&bytes[page..t]);
            let c = t + if format == Format::Unicode { 4 } else { 8 };
            bytes[c..c + 4].copy_from_slice(&crc.to_le_bytes());
            return;
        }
    }
    // A block: scan forward to the first 64-aligned end whose trailer's cb fits.
    let start = at - at % 64;
    for end in (start + 64..=bytes.len().min(start + 8192)).step_by(64) {
        let t = end - ts;
        let cb = usize::from(u16::from_le_bytes([bytes[t], bytes[t + 1]]));
        let Some(block_start) = end.checked_sub((cb + ts).div_ceil(64) * 64) else { continue };
        if block_start <= at && block_start + cb <= t {
            let crc = crc32(&bytes[block_start..block_start + cb]);
            let c = t + if format == Format::Unicode { 4 } else { 8 };
            bytes[c..c + 4].copy_from_slice(&crc.to_le_bytes());
            return;
        }
    }
}

use crate::crc::crc32;
