//! Elapsed (wall-clock) time of the default filtering:
//! `cargo test -p craft-pbr --release --test it -- --ignored --nocapture bench`.
//! The `parallel` feature is on in this test target even without `--features parallel`, since
//! the crate's dev-dependency on itself enables it, so the times are not a serial baseline.

use std::time::Instant;

use craft_pbr::{Environment, Limits, PrefilterOptions, try_prefilter, try_sh9};

#[test]
#[ignore = "timing, not a correctness check"]
fn bench_prefilter_and_sh9() {
    let limits = Limits::default();
    for width in [256, 512, 2048] {
        let env = Environment::Sunset.try_render(width, &limits).unwrap();
        let t = Instant::now();
        let mips = try_prefilter(&env, &PrefilterOptions::default(), &limits).unwrap();
        let filter = t.elapsed();
        let t = Instant::now();
        let sh = try_sh9(&env, &limits).unwrap();
        println!(
            "{width}x{}: prefilter {:.1} ms ({} levels), SH9 {:.1} ms (sh0 {:?})",
            width / 2,
            filter.as_secs_f64() * 1000.0,
            mips.len(),
            t.elapsed().as_secs_f64() * 1000.0,
            sh[0]
        );
    }
}
