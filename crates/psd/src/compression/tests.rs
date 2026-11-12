use super::*;

/// The straightforward index-based versions, kept as the oracle for the
/// register-carried rewrites in [`predict`] and [`unpredict`].
mod reference {
    use super::super::*;

    pub fn predict(buf: &mut [u8], layout: &PlaneLayout) {
        let rb = layout.row_bytes();
        let w = layout.width;
        let mut scratch = Vec::new();
        for row in buf.chunks_exact_mut(rb) {
            match layout.depth {
                8 => {
                    for i in (1..row.len()).rev() {
                        row[i] = row[i].wrapping_sub(row[i - 1]);
                    }
                }
                16 => {
                    for i in (1..w).rev() {
                        let a = u16::from_be_bytes([row[2 * i], row[2 * i + 1]]);
                        let b = u16::from_be_bytes([row[2 * i - 2], row[2 * i - 1]]);
                        row[2 * i..2 * i + 2].copy_from_slice(&a.wrapping_sub(b).to_be_bytes());
                    }
                }
                _ => {
                    scratch.clear();
                    scratch.resize(rb, 0);
                    for i in 0..w {
                        for k in 0..4 {
                            scratch[k * w + i] = row[4 * i + k];
                        }
                    }
                    for i in (1..rb).rev() {
                        scratch[i] = scratch[i].wrapping_sub(scratch[i - 1]);
                    }
                    row.copy_from_slice(&scratch);
                }
            }
        }
    }

    pub fn unpredict(buf: &mut [u8], layout: &PlaneLayout) {
        let rb = layout.row_bytes();
        let w = layout.width;
        let mut scratch = Vec::new();
        for row in buf.chunks_exact_mut(rb) {
            match layout.depth {
                8 => {
                    for i in 1..row.len() {
                        row[i] = row[i].wrapping_add(row[i - 1]);
                    }
                }
                16 => {
                    for i in 1..w {
                        let a = u16::from_be_bytes([row[2 * i], row[2 * i + 1]]);
                        let b = u16::from_be_bytes([row[2 * i - 2], row[2 * i - 1]]);
                        row[2 * i..2 * i + 2].copy_from_slice(&a.wrapping_add(b).to_be_bytes());
                    }
                }
                _ => {
                    for i in 1..rb {
                        row[i] = row[i].wrapping_add(row[i - 1]);
                    }
                    scratch.clear();
                    scratch.extend_from_slice(row);
                    for i in 0..w {
                        for k in 0..4 {
                            row[4 * i + k] = scratch[k * w + i];
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn prediction_matches_reference_on_random_planes() {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for depth in [8u16, 16, 32] {
        for width in (1..=70).chain([255, 256, 257, 1000, 1023]) {
            for height in [1usize, 2, 3] {
                for fill in 0..4 {
                    let layout = PlaneLayout { planes: 1, width, height, depth, version: Version::Psd };
                    let len = layout.row_bytes() * height;
                    let src: Vec<u8> = (0..len)
                        .map(|_| match fill {
                            0 => next() as u8,
                            1 => 0xFF,
                            2 => [0x00, 0xFF, 0x80, 0x7F][(next() & 3) as usize],
                            _ => (next() as u8) | 0xF0,
                        })
                        .collect();
                    let (mut want, mut got) = (src.clone(), src.clone());
                    reference::predict(&mut want, &layout);
                    predict(&mut got, &layout).unwrap();
                    assert_eq!(got, want, "predict depth {depth} width {width} height {height} fill {fill}");
                    let (mut want, mut got) = (src.clone(), src);
                    reference::unpredict(&mut want, &layout);
                    unpredict(&mut got, &layout).unwrap();
                    assert_eq!(got, want, "unpredict depth {depth} width {width} height {height} fill {fill}");
                }
            }
        }
    }
}

#[test]
fn checked_zip_validates_trailer_size_and_padding_budget() {
    let layout = PlaneLayout { planes: 1, width: 3, height: 1, depth: 8, version: Version::Psd };
    for compression in [Compression::Zip, Compression::ZipPrediction] {
        let data = encode_planes(compression, &[1, 2, 3], &layout).unwrap();
        for cut in 0..data.len() {
            assert!(decode_planes(compression, &data[..cut], &layout).is_err(), "cut {cut}");
            assert!(validate_planes(compression, &data[..cut], &layout).is_err());
        }
        let mut corrupt = data;
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(decode_planes(compression, &corrupt, &layout).is_err());
        assert!(validate_planes(compression, &corrupt, &layout).is_err());
    }
    let padded = zip_compress(&[1, 2, 3, 0]);
    assert_eq!(decode_planes(Compression::Zip, &padded, &layout).unwrap(), [1, 2, 3]);
    let bomb = zip_compress(&vec![0; (1 << 20) + 4]);
    assert!(decode_planes(Compression::Zip, &bomb, &layout).is_err());
    assert!(validate_planes(Compression::Zip, &bomb, &layout).is_err());
}

fn layout(planes: usize, width: usize, height: usize, depth: u16, version: Version) -> PlaneLayout {
    PlaneLayout { planes, width, height, depth, version }
}

/// Verify PackBits round trips empty, literal and repeated-byte inputs.
fn pb_roundtrip(src: &[u8]) -> Vec<u8> {
    let enc = packbits::encode_vec(src);
    assert_eq!(packbits::encoded_len(src), enc.len());
    let dec = packbits::decode(&enc, src.len()).unwrap();
    assert_eq!(dec, src, "encoded: {enc:?}");
    enc
}

/// Compare block scanning with scalar packet boundaries over varied runs and literals.
#[test]
fn packbits_block_scan_preserves_scalar_packets() {
    /// Encode PackBits with the previous scalar packet scan as a byte oracle.
    fn scalar(src: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < src.len() {
            let mut run = 1;
            while i + run < src.len() && run < 128 && src[i + run] == src[i] {
                run += 1;
            }
            if run >= 2 {
                out.extend_from_slice(&[(1i16 - run as i16) as u8, src[i]]);
                i += run;
            } else {
                let start = i;
                while i < src.len() && i - start < 128 {
                    if i + 2 < src.len() && src[i] == src[i + 1] && src[i] == src[i + 2] {
                        break;
                    }
                    i += 1;
                }
                out.push((i - start - 1) as u8);
                out.extend_from_slice(&src[start..i]);
            }
        }
        out
    }
    let check = |src: &[u8]| {
        let expected = scalar(src);
        let mut actual = vec![42];
        packbits::encode(src, &mut actual);
        assert_eq!(&actual[1..], expected, "source {src:?}");
        assert_eq!(packbits::encoded_len(src), expected.len());
        assert!(expected.len() <= src.len() + src.len().div_ceil(128));
    };
    for n in 0..=10 {
        for mut bits in 0..3usize.pow(n) {
            let src: Vec<_> = (0..n)
                .map(|_| {
                    let b = [0, 1, 255][bits % 3];
                    bits /= 3;
                    b
                })
                .collect();
            check(&src);
        }
    }
    for start in 0..260 {
        for run in [2, 3, 7, 8, 9, 127, 128, 129, 256] {
            let mut src: Vec<_> = (0..520).map(|i| (i % 251) as u8).collect();
            src[start..start + run].fill(255);
            check(&src);
        }
    }
    for pair in [[0, 1], [0, 255], [128, 127], [255, 255]] {
        for n in [127, 128, 129, 255, 256, 257, 12000, 12001] {
            let mut src: Vec<_> = pair.into_iter().cycle().take(n).collect();
            check(&src);
            for i in [0, 7, 8, 127.min(n - 1), 128.min(n - 1), n - 1] {
                src[i] ^= 1;
                check(&src);
                src[i] ^= 1;
            }
        }
    }
}

#[test]
fn zip_absurd_declared_size_is_an_error() {
    let z = zip_compress(&[7u8; 64]);
    for expected in [1usize << 40, usize::MAX / 2, usize::MAX] {
        assert!(zip_decompress(&z, expected).is_err(), "expected {expected}");
    }
    // Through decode_planes: a tiny ZIP channel claiming a huge (but capped) plane.
    let l = layout(1, 1 << 16, 1 << 15, 8, Version::Psb);
    assert!(decode_planes(Compression::Zip, &z, &l).is_err());
    assert!(decode_planes(Compression::ZipPrediction, &z, &l).is_err());
    // Past MAX_DECODED_BYTES.
    let l = layout(4, 1 << 20, 1 << 20, 32, Version::Psb);
    assert!(decode_planes(Compression::Zip, &z, &l).is_err());
    // Garbage that is not zlib at all.
    assert!(zip_decompress(&[0xff; 16], 1 << 30).is_err());
}

#[test]
fn zip_decompress_exact_and_truncated() {
    let src: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    let z = zip_compress(&src);
    assert_eq!(zip_decompress(&z, src.len()).unwrap(), src);
    // Asking for fewer bytes returns exactly that many.
    assert_eq!(zip_decompress(&z, 1000).unwrap(), &src[..1000]);
    assert!(zip_decompress(&z, src.len() + 1).is_err());
    assert!(zip_decompress(&z, 0).unwrap().is_empty());
}

#[test]
fn packbits_empty() {
    assert!(pb_roundtrip(&[]).is_empty());
}

#[test]
fn packbits_single() {
    assert_eq!(pb_roundtrip(&[7]), vec![0, 7]);
}

#[test]
fn packbits_run_of_two() {
    assert_eq!(pb_roundtrip(&[5, 5]), vec![0xff, 5]);
}

#[test]
fn packbits_run_128() {
    let src = vec![9u8; 128];
    assert_eq!(pb_roundtrip(&src), vec![0x81, 9]);
}

#[test]
fn packbits_run_129() {
    let src = vec![9u8; 129];
    assert_eq!(pb_roundtrip(&src), vec![0x81, 9, 0, 9]);
}

#[test]
fn packbits_run_130_and_256() {
    pb_roundtrip(&[1u8; 130]);
    let enc = pb_roundtrip(&[1u8; 256]);
    assert_eq!(enc, vec![0x81, 1, 0x81, 1]);
}

#[test]
fn packbits_literal_128_and_129() {
    let src: Vec<u8> = (0..128u32).map(|i| i as u8).collect();
    let enc = pb_roundtrip(&src);
    assert_eq!(enc[0], 127);
    assert_eq!(enc.len(), 129);
    let src: Vec<u8> = (0..129u32).map(|i| i as u8).collect();
    let enc = pb_roundtrip(&src);
    assert_eq!(enc[0], 127);
    assert_eq!(enc[129], 0);
    assert_eq!(enc.len(), 131);
}

#[test]
fn packbits_literal_run_boundary() {
    pb_roundtrip(&[1, 2, 3, 3, 3, 3, 4, 5]);
    pb_roundtrip(&[1, 1, 2, 2, 3, 3]);
    pb_roundtrip(&[1, 2, 2, 3]);
    pb_roundtrip(&[0, 0, 0, 1, 0, 0, 0]);
    let enc = pb_roundtrip(&[1, 2, 3, 3, 3]);
    assert_eq!(enc, vec![1, 1, 2, 0xfe, 3]);
}

#[test]
fn packbits_never_emits_noop() {
    for n in 0..300usize {
        let src: Vec<u8> = (0..n).map(|i| ((i / 3) % 5) as u8).collect();
        let enc = pb_roundtrip(&src);
        // Walk the packets to find headers.
        let mut i = 0;
        while i < enc.len() {
            let h = enc[i] as i8;
            assert_ne!(h, -128);
            i += if h >= 0 { h as usize + 2 } else { 2 };
        }
    }
}

#[test]
fn packbits_decode_noop_skipped() {
    assert_eq!(packbits::decode(&[0x80, 0, 42], 1).unwrap(), vec![42]);
}

#[test]
fn packbits_decode_errors() {
    assert!(packbits::decode(&[], 1).is_err());
    assert!(packbits::decode(&[2, 1, 2], 3).is_err()); // literal truncated
    assert!(packbits::decode(&[0xfe], 3).is_err()); // run truncated
    assert!(packbits::decode(&[0xfd, 1], 3).is_err()); // run overflows (4 > 3)
    assert!(packbits::decode(&[3, 1, 2, 3, 4], 3).is_err()); // literal overflows
}

/// `out.len() + expected` past `usize::MAX` used to panic in debug builds and, in release,
/// wrap and return `Ok` having decoded nothing.
#[test]
fn packbits_decode_into_output_length_overflow_is_an_error() {
    let mut out = vec![7u8];
    let r = packbits::decode_into(&[0x00, 0xaa], usize::MAX, &mut out);
    assert!(matches!(r, Err(PsdError::LimitExceeded(_))), "{r:?}");
    assert_eq!(out, [7]);
    // The largest `expected` that fits still decodes up to the input, then reports it short.
    let r = packbits::decode_into(&[0x00, 0xaa], usize::MAX - 1, &mut out);
    assert!(matches!(r, Err(PsdError::Decompress(_))), "{r:?}");
    assert_eq!(out, [7, 0xaa]);
}

#[test]
fn packbits_zero_expected_ignores_input() {
    assert_eq!(packbits::decode(&[0xff, 1], 0).unwrap(), Vec::<u8>::new());
}

#[test]
fn compression_values() {
    for v in 0..6 {
        assert_eq!(Compression::from_u16(v).as_u16(), v);
    }
    assert_eq!(Compression::from_u16(9), Compression::Unknown(9));
}

fn sample_data(l: &PlaneLayout) -> Vec<u8> {
    let n = l.decoded_len().unwrap();
    (0..n).map(|i| ((i * 7 + i / 5) % 251) as u8).collect()
}

#[test]
fn all_codecs_roundtrip_all_depths() {
    for version in [Version::Psd, Version::Psb] {
        for depth in [1u16, 8, 16, 32] {
            for (w, h) in [(0, 0), (1, 1), (3, 2), (17, 5), (0, 3), (4, 0)] {
                for planes in [1, 3] {
                    let l = layout(planes, w, h, depth, version);
                    let data = sample_data(&l);
                    for c in Compression::ALL {
                        let enc = encode_planes(c, &data, &l).unwrap();
                        validate_planes(c, &enc, &l).unwrap();
                        let dec = decode_planes(c, &enc, &l).unwrap();
                        assert_eq!(dec, data, "{c:?} {depth} {w}x{h} {version:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn rle_count_width_psd_vs_psb() {
    let data = vec![0u8; 4];
    let psd = encode_planes(Compression::Rle, &data, &layout(1, 2, 2, 8, Version::Psd)).unwrap();
    let psb = encode_planes(Compression::Rle, &data, &layout(1, 2, 2, 8, Version::Psb)).unwrap();
    assert_eq!(psd, vec![0, 2, 0, 2, 0xff, 0, 0xff, 0]);
    assert_eq!(psb, vec![0, 0, 0, 2, 0, 0, 0, 2, 0xff, 0, 0xff, 0]);
    assert!(decode_planes(Compression::Rle, &psd, &layout(1, 2, 2, 8, Version::Psb)).is_err());
}

#[test]
fn rle_odd_width() {
    let l = layout(1, 7, 3, 16, Version::Psd);
    let data = sample_data(&l);
    let enc = encode_planes(Compression::Rle, &data, &l).unwrap();
    assert_eq!(decode_planes(Compression::Rle, &enc, &l).unwrap(), data);
}

#[test]
fn rle_truncated_fails() {
    let l = layout(1, 10, 10, 8, Version::Psd);
    let data = sample_data(&l);
    let enc = encode_planes(Compression::Rle, &data, &l).unwrap();
    for cut in 0..enc.len() {
        assert!(decode_planes(Compression::Rle, &enc[..cut], &l).is_err(), "cut {cut}");
        assert!(validate_planes(Compression::Rle, &enc[..cut], &l).is_err(), "cut {cut}");
    }
}

#[test]
fn zip_truncated_fails() {
    let l = layout(1, 10, 10, 8, Version::Psd);
    let data = sample_data(&l);
    let enc = encode_planes(Compression::Zip, &data, &l).unwrap();
    for cut in 0..enc.len() - 4 {
        assert!(decode_planes(Compression::Zip, &enc[..cut], &l).is_err(), "cut {cut}");
        assert!(validate_planes(Compression::Zip, &enc[..cut], &l).is_err(), "cut {cut}");
    }
}

/// photocraft#2926: a ZIP stream declaring more than `MAX_DECODED_BYTES` is not inflated at parse
/// time. Garbage zlib data proves it: decompressing it would fail, skipping it passes, and
/// decoding the layout still reports the size limit.
#[test]
fn zip_over_budget_is_not_decompressed_at_parse() {
    let l = layout(4, 300_000, 300_000, 8, Version::Psb);
    assert!(l.total_bytes().unwrap() > MAX_DECODED_BYTES);
    let garbage = [0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0x03];
    for c in [Compression::Zip, Compression::ZipPrediction] {
        validate_planes(c, &garbage, &l).unwrap();
        assert!(matches!(decode_planes(c, &garbage, &l), Err(PsdError::LimitExceeded(_))), "{c:?}");
    }
    // Within the budget the stream is still checked.
    let small = layout(1, 4, 4, 8, Version::Psd);
    assert!(validate_planes(Compression::Zip, &garbage, &small).is_err());
}

#[test]
fn raw_short_fails() {
    let l = layout(2, 3, 3, 8, Version::Psd);
    assert!(decode_planes(Compression::Raw, &[0; 17], &l).is_err());
    assert!(decode_planes(Compression::Raw, &[0; 18], &l).is_ok());
}

#[test]
fn unknown_compression_errors() {
    let l = layout(1, 1, 1, 8, Version::Psd);
    assert!(decode_planes(Compression::Unknown(7), &[0], &l).is_err());
    assert!(encode_planes(Compression::Unknown(7), &[0], &l).is_err());
}

#[test]
fn encode_rejects_wrong_size() {
    let l = layout(1, 2, 2, 8, Version::Psd);
    assert!(encode_planes(Compression::Raw, &[0; 3], &l).is_err());
}

#[test]
fn decoded_len_limits() {
    let l = layout(56, 300_000, 300_000, 32, Version::Psb);
    assert!(matches!(l.decoded_len(), Err(PsdError::LimitExceeded(_))));
    assert!(decode_planes(Compression::Rle, &[0; 16], &l).is_err());
}

/// Real PSBs go past 2 GiB in one decode (photocraft#375): a 30000² RGB merged image and a 33000²
/// 16-bit layer channel. Only checks the limit, so nothing is allocated.
#[test]
#[cfg(target_pointer_width = "64")]
fn decoded_len_allows_large_psb_planes() {
    assert_eq!(layout(3, 30_000, 30_000, 8, Version::Psb).decoded_len().unwrap(), 2_700_000_000);
    assert_eq!(layout(1, 33_000, 33_000, 16, Version::Psb).decoded_len().unwrap(), 2_178_000_000);
    assert!(layout(4, 40_000, 40_000, 16, Version::Psb).decoded_len().is_err());
}

#[test]
fn prediction_8bit_known_vector() {
    let l = layout(1, 4, 1, 8, Version::Psd);
    let mut v = vec![10, 12, 11, 255];
    predict(&mut v, &l).unwrap();
    assert_eq!(v, vec![10, 2, 255, 244]);
    unpredict(&mut v, &l).unwrap();
    assert_eq!(v, vec![10, 12, 11, 255]);
}

#[test]
fn prediction_16bit_known_vector() {
    let l = layout(1, 3, 1, 16, Version::Psd);
    let mut v = vec![0x01, 0x00, 0x01, 0x05, 0x00, 0x00];
    predict(&mut v, &l).unwrap();
    assert_eq!(v, vec![0x01, 0x00, 0x00, 0x05, 0xfe, 0xfb]);
    unpredict(&mut v, &l).unwrap();
    assert_eq!(v, vec![0x01, 0x00, 0x01, 0x05, 0x00, 0x00]);
}

#[test]
fn prediction_32bit_plane_shuffle() {
    // Two samples: AABBCCDD and 11223344 → planes [AA,11][BB,22][CC,33][DD,44]
    let l = layout(1, 2, 1, 32, Version::Psd);
    let orig = vec![0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44];
    let mut v = orig.clone();
    predict(&mut v, &l).unwrap();
    let planes = [0xAAu8, 0x11, 0xBB, 0x22, 0xCC, 0x33, 0xDD, 0x44];
    let mut expect = planes;
    for i in (1..8).rev() {
        expect[i] = expect[i].wrapping_sub(expect[i - 1]);
    }
    assert_eq!(v, expect);
    unpredict(&mut v, &l).unwrap();
    assert_eq!(v, orig);
}

#[test]
fn prediction_32bit_floats_roundtrip() {
    let l = layout(1, 5, 2, 32, Version::Psd);
    let mut v = Vec::new();
    for i in 0..10 {
        v.extend_from_slice(&(i as f32 * 0.37 - 1.0).to_be_bytes());
    }
    let enc = encode_planes(Compression::ZipPrediction, &v, &l).unwrap();
    assert_eq!(decode_planes(Compression::ZipPrediction, &enc, &l).unwrap(), v);
}

/// Compare fused float prediction with explicit byte-plane shuffling and delta coding.
#[test]
fn fused_prediction_matches_shuffled_byte_deltas() {
    for w in [1, 2, 3, 31, 256, 257] {
        let l = layout(2, w, 3, 32, Version::Psb);
        let mut actual: Vec<_> = (0..l.decoded_len().unwrap()).map(|i| (i * 31 + i / 7) as u8).collect();
        let expected: Vec<_> = actual
            .chunks_exact(w * 4)
            .flat_map(|row| {
                let shuffled: Vec<_> = (0..4).flat_map(|k| row.iter().skip(k).step_by(4).copied()).collect();
                (0..shuffled.len()).map(move |i| shuffled[i].wrapping_sub(if i == 0 { 0 } else { shuffled[i - 1] }))
            })
            .collect();
        predict(&mut actual, &l).unwrap();
        assert_eq!(actual, expected, "width {w}");
    }
}

#[test]
fn zip_bomb_is_bounded() {
    let big = zip_compress(&vec![0u8; 1 << 20]);
    let out = zip_decompress(&big, 10).unwrap();
    assert_eq!(out.len(), 10);
}

#[test]
fn zip_garbage_errors() {
    assert!(zip_decompress(&[1, 2, 3, 4], 4).is_err());
}

/// Compare bounded ZIP output with the original growing writer over varied inputs.
#[test]
fn zip_output_matches_the_growing_writer() {
    let mut seed = 0x12345678u32;
    let noise = (0..1_048_577)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as u8
        })
        .collect();
    for bytes in [Vec::new(), vec![0; 131072], (0..131072).map(|i| (i * 31 + i / 173) as u8).collect(), noise].iter().cycle().take(12) {
        let mut old = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        old.write_all(bytes).unwrap();
        let packed = zip_compress(bytes);
        assert_eq!(packed, old.finish().unwrap());
        #[cfg(not(target_arch = "wasm32"))]
        assert_eq!(packed.capacity(), packed.len());
    }
}

/// Workers retain only private codec state; a checked-out slot supports nested compression.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn zip_encoder_reuse_is_private_and_retains_no_output() {
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..2)
            .map(|byte| {
                scope.spawn(move || {
                    let mut seed = 0x12345678u32;
                    for len in [1_048_577usize, 32769, 0, 259, 258, 257, 1, 32767, 32768, 2, 262145] {
                        let bytes: Vec<_> = (0..len)
                            .map(|_| {
                                seed ^= seed << 13;
                                seed ^= seed >> 17;
                                seed ^= seed << 5;
                                if byte == 0 { 0 } else { seed as u8 }
                            })
                            .collect();
                        let packed = zip_compress(&bytes);
                        if byte == 1 && len == 1_048_577 {
                            assert!(packed.len() > 256 << 10, "the retention oracle must exercise multiple output blocks");
                        }
                        let encoder = ZIP_ENCODER.with(std::cell::Cell::take).unwrap();
                        assert_eq!(encoder.total_in(), len as u64);
                        assert_eq!(encoder.total_out(), packed.len() as u64);
                        assert_eq!(encoder.get_ref().blocks.capacity(), 0);
                        assert_eq!(encoder.get_ref().current.capacity(), 0);
                        assert_eq!(zip_decompress(&zip_compress(b"nested"), 6).unwrap(), b"nested");
                        ZIP_ENCODER.with(|slot| slot.set(Some(encoder)));
                        let mut fresh = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                        fresh.write_all(&bytes).unwrap();
                        assert_eq!(zip_compress(&bytes), fresh.finish().unwrap());
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
    });
}

/// Compression stays byte-identical during teardown, including an unavailable native cache.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn zip_compression_survives_thread_local_teardown() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct OnDrop(Arc<(AtomicBool, AtomicBool)>);
    impl Drop for OnDrop {
        /// Record whether teardown reached the destroyed cache and preserved fresh-stream bytes.
        fn drop(&mut self) {
            let unavailable = ZIP_ENCODER.try_with(|_| ()).is_err();
            let mut fresh = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            fresh.write_all(b"teardown").unwrap();
            let same = zip_compress(b"teardown") == fresh.finish().unwrap();
            self.0.0.store(same, Ordering::SeqCst);
            self.0.1.store(unavailable, Ordering::SeqCst);
        }
    }
    thread_local! { static AFTER_CODEC: std::cell::Cell<Option<OnDrop>> = const { std::cell::Cell::new(None) }; }
    let checked = Arc::new((AtomicBool::new(false), AtomicBool::new(false)));
    let flag = checked.clone();
    std::thread::spawn(move || {
        AFTER_CODEC.with(|slot| slot.set(Some(OnDrop(flag))));
        zip_compress(b"first");
    })
    .join()
    .unwrap();
    assert!(checked.0.load(Ordering::SeqCst), "teardown must preserve the fresh encoder's bytes");
    // OS-key TLS may destroy keys in another order; native TLS runs these destructors in reverse.
    #[cfg(any(target_vendor = "apple", target_os = "linux", target_env = "msvc"))]
    assert!(checked.1.load(Ordering::SeqCst), "teardown must exercise the uncached fallback");
}

/// Verify multiple writes retain their bytes and respect ZIP output block bounds.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn zip_output_blocks_stay_bounded_across_writes() {
    let bytes = vec![42; (1 << 20) + 1];
    let mut output = ZipOutput::default();
    output.write_all(&[]).unwrap();
    for part in bytes.chunks(32767) {
        output.write_all(part).unwrap();
    }
    assert!(output.blocks.iter().chain([&output.current]).all(|b| b.capacity() <= 256 << 10));
    let packed = output.into_vec();
    assert_eq!(packed, bytes);
    assert_eq!(packed.capacity(), packed.len());
}

/// Verify compressible ZIP output retains capacity proportional to encoded size.
#[test]
fn compressible_zip_does_not_retain_a_raw_sized_allocation() {
    let raw = vec![0; 1 << 20];
    let packed = zip_compress(&raw);
    assert!(packed.len() < 1 << 14);
    assert!(packed.capacity() < 1 << 16);
    assert_eq!(zip_decompress(&packed, raw.len()).unwrap(), raw);
}

/// Verify failed RLE allocation hints fall back to incremental growth with identical bytes.
#[test]
fn rle_failed_hint_and_incremental_growth_preserve_bytes() {
    for version in [Version::Psd, Version::Psb] {
        for width in [0, 1, 127, 128, 129, 1000] {
            let l = layout(1, width, 9, 8, version);
            for fill in [false, true] {
                let bytes: Vec<_> = (0..l.decoded_len().unwrap()).map(|i| if fill { 0 } else { (i * 13 + i / 17) as u8 }).collect();
                let expected = encode_planes(Compression::Rle, &bytes, &l).unwrap();
                for hint in [usize::MAX, 0, l.rows().unwrap() * l.count_size()] {
                    assert_eq!(encode_rle_chunk(&bytes, &l, 0..l.rows().unwrap(), hint).unwrap(), expected);
                }
            }
        }
    }
}

/// Verify measured packet lengths match encoding over mixed runs and literals.
#[test]
fn counted_packbits_matches_mixed_runs_and_literals() {
    for n in 0..520 {
        let bytes: Vec<_> = (0..n).map(|i| if i % 23 < 9 { (i / 23) as u8 } else { (i * 31) as u8 }).collect();
        assert_eq!(packbits::encoded_len(&bytes), packbits::encode_vec(&bytes).len());
    }
}

/// Compare parallel prediction with independent serial rows at each supported depth.
#[test]
fn prediction_matches_independent_serial_rows() {
    for depth in [1, 8, 16, 32] {
        let layout = layout(4, if depth == 1 { 8193 } else { 257 }, 1031, depth, Version::Psb);
        let rb = layout.row_bytes();
        let bytes: Vec<_> = (0..layout.decoded_len().unwrap() + 3).map(|i| (i * 13 + i / 17) as u8).collect();
        let mut expected = bytes.clone();
        for row in expected.chunks_exact_mut(rb) {
            let src = row.to_vec();
            if depth == 16 {
                for (i, p) in row.as_chunks_mut::<2>().0.iter_mut().enumerate().skip(1) {
                    let a = u16::from_be_bytes([src[i * 2], src[i * 2 + 1]]);
                    let b = u16::from_be_bytes([src[i * 2 - 2], src[i * 2 - 1]]);
                    *p = a.wrapping_sub(b).to_be_bytes();
                }
            } else {
                let shuffled = if depth == 32 { (0..4).flat_map(|c| src.iter().skip(c).step_by(4).copied()).collect::<Vec<_>>() } else { src };
                for (i, p) in row.iter_mut().enumerate() {
                    *p = shuffled[i].wrapping_sub(if i == 0 { 0 } else { shuffled[i - 1] });
                }
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        for threads in [1, 2, 4] {
            use rayon::prelude::*;
            let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
            pool.install(|| {
                (0..3).into_par_iter().for_each(|_| {
                    let mut got = bytes.clone();
                    predict(&mut got, &layout).unwrap();
                    assert_eq!(got, expected, "depth {depth}, pool {threads}");
                })
            });
        }
        let mut got = bytes;
        predict(&mut got, &layout).unwrap();
        assert_eq!(got, expected);
    }
}

/// Verify RLE chunks use the caller's pool and propagate worker panics as errors.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn rle_chunks_use_the_callers_pool_and_report_panics() {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap();
    pool.install(|| {
        map_chunks(8, (0..64).collect(), |_| assert!(matches!(rayon::current_thread_index(), Some(0 | 1)))).unwrap();
        assert!(map_chunks(2, vec![0, 1], |_| panic!("worker")).is_err());
    });
}
