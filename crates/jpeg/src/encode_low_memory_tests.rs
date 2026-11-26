use super::*;
use std::cell::Cell;

thread_local! {
    static STATS: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}

pub(super) fn record_planes(samples: usize) {
    STATS.with(|s| {
        let (peak, tokens) = s.get();
        s.set((peak.max(samples), tokens));
    });
}
pub(super) fn record_tokens(count: usize) {
    STATS.with(|s| {
        let (peak, tokens) = s.get();
        s.set((peak, tokens + count));
    });
}

fn noise(len: usize) -> Vec<u8> {
    let mut seed = 0x89ab_cdefu32;
    (0..len)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as u8
        })
        .collect()
}

fn compare(src: &[u8], w: usize, h: usize, ch: usize, q: u8, sub: ChromaSubsampling, app: &[(u8, Vec<u8>)]) {
    let expected = encode_baseline(src, w, h, ch, q, sub, app).unwrap();
    STATS.with(|s| s.set((0, 0)));
    let actual = encode_baseline_low_memory(src, w, h, ch, q, sub, app).unwrap();
    assert_eq!(actual, expected, "{w}x{h} channels={ch} quality={q} subsampling={sub:?}");
    let (peak, tokens) = STATS.with(Cell::get);
    assert!(peak <= 24 * w + 8, "MCU-row scratch exceeded: {peak}");
    assert_eq!(tokens, 0, "low-memory mode retained entropy tokens");
}

#[test]
fn all_sampling_and_channels_match_retained_tokens_byte_for_byte() {
    for (w, h) in [(1, 1), (7, 9), (17, 31), (33, 65), (17, 1025)] {
        for ch in [1, 3, 4] {
            let src = noise(w * h * ch);
            for sub in [ChromaSubsampling::S444, ChromaSubsampling::S422, ChromaSubsampling::S420] {
                for q in [0, 1, 25, 50, 90, 100, 255] {
                    compare(&src, w, h, ch, q, sub, &[]);
                }
            }
        }
    }
}

#[test]
fn high_entropy_hd_and_wide_frames_keep_only_one_mcu_row() {
    for (w, h) in [(1920, 1080), (65_535, 1)] {
        let src = noise(w * h * 3);
        compare(&src, w, h, 3, 100, ChromaSubsampling::S444, &[]);
    }
}

#[test]
fn metadata_empty_tables_and_extra_source_bytes_keep_exact_output() {
    let app = [(0xe0, b"custom density".to_vec()), (0xe2, vec![73; 65_533])];
    for ch in [1, 3, 4] {
        let src = vec![127; 19 * 23 * ch + 5];
        compare(&src, 19, 23, ch, 75, ChromaSubsampling::S420, &app);
    }
}

#[test]
fn checked_admission_matches_original_without_panics() {
    for (w, h, ch) in [(0, 1, 3), (1, 0, 3), (1, 1, 0), (1, 1, 2), (usize::MAX, 2, 4), (65_536, 1, 1), (1, 65_536, 1), (4, 4, 3)] {
        let src = vec![0; if w == 65_536 || h == 65_536 { 65_536 } else { 8 }];
        let old = encode_baseline(&src, w, h, ch, 90, ChromaSubsampling::S444, &[]);
        let new = encode_baseline_low_memory(&src, w, h, ch, 90, ChromaSubsampling::S444, &[]);
        assert!(new.is_err());
        assert_eq!(new, old);
    }
    for app in [vec![(0xdf, vec![])], vec![(0xf0, vec![])], vec![(0xe1, vec![0; 65_534])]] {
        assert_eq!(
            encode_baseline_low_memory(&[0; 3], 1, 1, 3, 90, ChromaSubsampling::S444, &app),
            encode_baseline(&[0; 3], 1, 1, 3, 90, ChromaSubsampling::S444, &app)
        );
    }
}

// Generated with the immutable pre-change encoder at craft-libs
// 937ac93c5cf3cd4b6a81e195b078fe161eb081b9, not the refactored token-retaining path.
// Odd chroma edges and 1025/2049-row inputs exercise multi-row restart bands.
#[test]
fn prechange_encoded_bytes_remain_pinned_across_band_boundaries() {
    let cases = [
        (1, 1, 1, 1, 444, 159, 0xe3bd59145381ba57),
        (1, 1, 1, 75, 422, 159, 0xcb2b103914bbdbd3),
        (1, 1, 1, 100, 420, 160, 0xd9f640613efb6abf),
        (1, 1, 3, 1, 444, 284, 0x61d2772b8fd9df17),
        (1, 1, 3, 75, 422, 286, 0xa0e03ab7efa23296),
        (1, 1, 3, 100, 420, 288, 0x5660667f1abd0a63),
        (1, 1, 4, 1, 444, 284, 0x61d2772b8fd9df17),
        (1, 1, 4, 75, 422, 286, 0xa0e03ab7efa23296),
        (1, 1, 4, 100, 420, 288, 0x5660667f1abd0a63),
        (17, 31, 1, 1, 444, 240, 0xf8d277e9499253d4),
        (17, 31, 1, 75, 422, 515, 0xa6946d4f144bf243),
        (17, 31, 1, 100, 420, 793, 0xbb3db259708d8951),
        (17, 31, 3, 1, 444, 363, 0xff2b5b41ffb3f81b),
        (17, 31, 3, 75, 422, 877, 0x0374ffb527378dfe),
        (17, 31, 3, 100, 420, 1220, 0x2feeedfa90885737),
        (17, 31, 4, 1, 444, 366, 0xde7f8d1330e436eb),
        (17, 31, 4, 75, 422, 879, 0xb1559fbfc3ee0cf6),
        (17, 31, 4, 100, 420, 1214, 0xaf5dcf12d28c8e88),
        (33, 65, 1, 1, 444, 388, 0xbee80491e42c8fd2),
        (33, 65, 1, 75, 422, 1470, 0x284f4eb78b785ca5),
        (33, 65, 1, 100, 420, 2576, 0xa81cc94015f326dc),
        (33, 65, 3, 1, 444, 526, 0xf75b4036acc30874),
        (33, 65, 3, 75, 422, 2322, 0x6fbc56bd9a1eec70),
        (33, 65, 3, 100, 420, 3796, 0xcad25051562ac816),
        (33, 65, 4, 1, 444, 527, 0x4bfedc11153de4c1),
        (33, 65, 4, 75, 422, 2309, 0x47bf11bf32bac6a4),
        (33, 65, 4, 100, 420, 3775, 0x64c2303fde273cd1),
        (17, 1025, 1, 1, 444, 2017, 0x242f4a95d8579612),
        (17, 1025, 1, 75, 422, 10728, 0xfe49408f379bdffb),
        (17, 1025, 1, 100, 420, 19686, 0x1355b995e38d6467),
        (17, 1025, 3, 1, 444, 2137, 0x4e319440cb6946c2),
        (17, 1025, 3, 75, 422, 17682, 0xf456c63b731dde8f),
        (17, 1025, 3, 100, 420, 29015, 0x0631001154b128cf),
        (17, 1025, 4, 1, 444, 2155, 0xa2de9c1d77e62aa6),
        (17, 1025, 4, 75, 422, 17648, 0x7d85fbdf56061f0c),
        (17, 1025, 4, 100, 420, 29010, 0x4848eeb93daf1df2),
        (19, 2049, 1, 1, 444, 3531, 0x9b93cf219ff85796),
        (19, 2049, 1, 75, 422, 26283, 0xb160523444397608),
        (19, 2049, 1, 100, 420, 51883, 0xc5c26836c0a976c6),
        (19, 2049, 3, 1, 444, 3303, 0x26eaac2272e1fdb8),
        (19, 2049, 3, 75, 422, 42745, 0x38b9843fb54db698),
        (19, 2049, 3, 100, 420, 79868, 0xd6a7917af19682f7),
        (19, 2049, 4, 1, 444, 3343, 0x6bf577903528692f),
        (19, 2049, 4, 75, 422, 42706, 0x0e62366274593a93),
        (19, 2049, 4, 100, 420, 79804, 0x867d05cf9d944599),
    ];
    for (w, h, ch, q, sampling, len, hash) in cases {
        let sub = match sampling {
            444 => ChromaSubsampling::S444,
            422 => ChromaSubsampling::S422,
            _ => ChromaSubsampling::S420,
        };
        let src = noise(w * h * ch);
        for actual in [encode_baseline(&src, w, h, ch, q, sub, &[]).unwrap(), encode_baseline_low_memory(&src, w, h, ch, q, sub, &[]).unwrap()] {
            let actual_hash = actual.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100000001b3));
            assert_eq!((actual.len(), actual_hash), (len, hash), "{w}x{h} channels={ch} quality={q} sampling={sampling}");
        }
    }
}
