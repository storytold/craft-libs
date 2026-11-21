//! Throughput check: `cargo test -p craft-codepage --release --test bench -- --ignored --nocapture --test-threads=1`.

use std::borrow::Cow;
use std::time::Instant;

use craft_codepage::vietnamese::{DecodeOptions, LegacyEncoding, decode_text};

#[test]
#[ignore]
fn bench_clean_utf8_fast_path() {
    let sample = "Cộng hòa Xã hội Chủ nghĩa Việt Nam - Độc lập Tự do Hạnh phúc.\n".repeat(20_000);
    let bytes = sample.as_bytes();
    let size_mb = bytes.len() as f64 / (1024.0 * 1024.0);

    let t0 = Instant::now();
    let iters = 100;
    let opts = DecodeOptions::default();
    for _ in 0..iters {
        let res = decode_text(bytes, &opts).expect("clean UTF-8 must decode cleanly");
        assert!(matches!(res, Cow::Borrowed(_)));
    }
    let elapsed = t0.elapsed();
    let total_mb = size_mb * iters as f64;
    let mb_per_sec = total_mb / elapsed.as_secs_f64();
    eprintln!("clean UTF-8 fast-path (Cow::Borrowed): {:.2} MB in {:?} ({:.2} MB/s)", total_mb, elapsed, mb_per_sec);
}

#[test]
#[ignore]
fn bench_tcvn3_decode() {
    // TCVN3 sample: "Cộng hòa Xã hội Chủ nghĩa Việt Nam - Độc lập Tự do Hạnh phúc.\n"
    let tcvn3_line = b"C\xE9ng h\xDFa X\xB7 h\xE9i Ch\xF1 ngh\xDCa Vi\xD6t Nam - \xA7\xE9c l\xCBp T\xF9 do H\xB9nh ph\xF3c.\n";
    let opts = DecodeOptions { force_encoding: Some(LegacyEncoding::Tcvn3), ..DecodeOptions::default() };
    let sample_decoded = decode_text(tcvn3_line, &opts).expect("TCVN3 sample line must decode cleanly");
    assert_eq!(sample_decoded, "Cộng hòa Xã hội Chủ nghĩa Việt Nam - Độc lập Tự do Hạnh phúc.\n");

    let bytes: Vec<u8> = tcvn3_line.repeat(20_000);
    let size_mb = bytes.len() as f64 / (1024.0 * 1024.0);

    let t0 = Instant::now();
    let iters = 50;
    for _ in 0..iters {
        let res = decode_text(&bytes, &opts).expect("TCVN3 must decode cleanly");
        assert!(matches!(res, Cow::Owned(_)));
    }
    let elapsed = t0.elapsed();
    let total_mb = size_mb * iters as f64;
    let mb_per_sec = total_mb / elapsed.as_secs_f64();
    eprintln!("TCVN3 legacy transcoding: {:.2} MB in {:?} ({:.2} MB/s)", total_mb, elapsed, mb_per_sec);
}

#[test]
#[ignore]
fn bench_vni_decode() {
    // VNI sample: "Cộng hòa Xã hội Chủ nghĩa Việt Nam - Độc lập Tự do Hạnh phúc.\n"
    let vni_line = b"Co\xE4ng ho\xF8a Xa\xF5 ho\xE4i Chu\xFB ngh\xF3a Vie\xE4t Nam - \xD1o\xE4c la\xE4p T\xF6\xEF do Ha\xEFnh phu\xF9c.\n";
    let opts = DecodeOptions { force_encoding: Some(LegacyEncoding::Vni), ..DecodeOptions::default() };
    let sample_decoded = decode_text(vni_line, &opts).expect("VNI sample line must decode cleanly");
    assert_eq!(sample_decoded, "Cộng hòa Xã hội Chủ nghĩa Việt Nam - Độc lập Tự do Hạnh phúc.\n");

    let bytes: Vec<u8> = vni_line.repeat(20_000);
    let size_mb = bytes.len() as f64 / (1024.0 * 1024.0);

    let t0 = Instant::now();
    let iters = 50;
    for _ in 0..iters {
        let res = decode_text(&bytes, &opts).expect("VNI must decode cleanly");
        assert!(matches!(res, Cow::Owned(_)));
    }
    let elapsed = t0.elapsed();
    let total_mb = size_mb * iters as f64;
    let mb_per_sec = total_mb / elapsed.as_secs_f64();
    eprintln!("VNI-Windows legacy transcoding: {:.2} MB in {:?} ({:.2} MB/s)", total_mb, elapsed, mb_per_sec);
}

#[test]
#[ignore]
fn bench_cp1258_decode() {
    // CP1258 sample: "Cộng hòa Xã hội Chủ nghĩa Việt Nam - Độc lập Tự do Hạnh phúc.\n"
    let cp1258_line =
        b"C\xF4\xF2ng ho\xCCa Xa\xDE h\xF4\xF2i Chu\xD2 nghi\xDEa Vi\xEA\xF2t Nam - \xD0\xF4\xF2c l\xE2\xF2p T\xFD\xF2 do Ha\xF2nh ph\xFAc.\n";

    let opts = DecodeOptions { force_encoding: Some(LegacyEncoding::Windows1258), ..DecodeOptions::default() };
    let sample_decoded = decode_text(cp1258_line, &opts).expect("CP1258 sample line must decode cleanly");
    assert_eq!(sample_decoded, "Cộng hòa Xã hội Chủ nghĩa Việt Nam - Độc lập Tự do Hạnh phúc.\n");

    let bytes: Vec<u8> = cp1258_line.repeat(20_000);
    let size_mb = bytes.len() as f64 / (1024.0 * 1024.0);

    let t0 = Instant::now();
    let iters = 50;
    for _ in 0..iters {
        let res = decode_text(&bytes, &opts).expect("CP1258 must decode cleanly");
        assert!(matches!(res, Cow::Owned(_)));
    }
    let elapsed = t0.elapsed();
    let total_mb = size_mb * iters as f64;
    let mb_per_sec = total_mb / elapsed.as_secs_f64();
    eprintln!("CP1258 legacy transcoding: {:.2} MB in {:?} ({:.2} MB/s)", total_mb, elapsed, mb_per_sec);
}
