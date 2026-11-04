//! Throughput check: `cargo test -p craft-i18n --release --test bench -- --ignored --nocapture`.

use std::borrow::Cow;
use std::time::Instant;

use craft_i18n::encoding::{DecodeOptions, LegacyEncoding, decode_text};

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
    // TCVN3 sample: "Céng hoµ X· héi Chñ nghÜa ViÖt Nam"
    let tcvn3_line = b"C\xE9ng ho\xB5 X\xB7 h\xE9i Ch\xF1 ngh\xEFa Vi\xE7t Nam - \xD0\x99c l\xADp T\xF1 do H\x1Bnh ph\xFAc.\n";
    let bytes: Vec<u8> = tcvn3_line.repeat(20_000);
    let size_mb = bytes.len() as f64 / (1024.0 * 1024.0);

    let t0 = Instant::now();
    let iters = 50;
    let opts = DecodeOptions { force_encoding: Some(LegacyEncoding::Tcvn3), ..DecodeOptions::default() };
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
    // VNI sample: "Coäng hoøa Xaõ hoäi Chuû nghóa Vieät Nam"
    let vni_line = b"Co\xE4ng ho\xF8a Xa\xF5 ho\xE4i Chu\xF9 ngh\xF3a Vie\xE4t Nam - \xD1o\xE4c la\xE4p Tu\xF9 do Ha\xEFnh phu\xFAc.\n";
    let bytes: Vec<u8> = vni_line.repeat(20_000);
    let size_mb = bytes.len() as f64 / (1024.0 * 1024.0);

    let t0 = Instant::now();
    let iters = 50;
    let opts = DecodeOptions { force_encoding: Some(LegacyEncoding::Vni), ..DecodeOptions::default() };
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
    // CP1258 sample: "Cộng hòa Xã hội Chủ nghĩa Việt Nam"
    let cp1258_line = b"C\xF4\x88ng h\xF2a X\xE3 h\xF4\x88i Ch\xF7 ngh\xEFa Vi\xEA\x88t Nam - \xD0\xF4\x88c l\xE2\x88p T\xF7 do H\x1Bnh ph\xFAc.\n";
    let bytes: Vec<u8> = cp1258_line.repeat(20_000);
    let size_mb = bytes.len() as f64 / (1024.0 * 1024.0);

    let t0 = Instant::now();
    let iters = 50;
    let opts = DecodeOptions { force_encoding: Some(LegacyEncoding::Windows1258), ..DecodeOptions::default() };
    for _ in 0..iters {
        let res = decode_text(&bytes, &opts).expect("CP1258 must decode cleanly");
        assert!(matches!(res, Cow::Owned(_)));
    }
    let elapsed = t0.elapsed();
    let total_mb = size_mb * iters as f64;
    let mb_per_sec = total_mb / elapsed.as_secs_f64();
    eprintln!("CP1258 legacy transcoding: {:.2} MB in {:?} ({:.2} MB/s)", total_mb, elapsed, mb_per_sec);
}
