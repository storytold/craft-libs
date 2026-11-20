//! #2938: `Limits::max_alloc` budgets the decoded pixel buffer, so the PNM decoder must not build a
//! wider scratch copy (it used to collect one `u32` per sample, four times an 8-bit image) next to it.
//!
//! This probe re-runs the test in an isolated process: it reads the process's peak RSS
//! (`VmHWM`, Linux only) and nothing else may allocate meanwhile.

fn peak_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    let kib: u64 = line.trim_start_matches("VmHWM:").trim().trim_end_matches("kB").trim().parse().ok()?;
    Some(kib * 1024)
}

#[test]
fn pnm_decode_peak_memory_stays_near_the_output_size() {
    const CHILD: &str = "CRAFT_IMAGE_CODECS_PNM_RSS_CHILD";
    const DONE: &str = "PNM RSS probe completed";
    if std::env::var_os(CHILD).is_some() {
        measure_peak_memory();
        println!("{DONE}");
        return;
    }

    // Cargo can run other tests concurrently in this harness. Start a fresh process
    // with only this test selected so its allocations are the only ones being measured.
    let output = std::process::Command::new(std::env::current_exe().expect("image codec test executable"))
        .args(["--exact", "pnm_peak_memory::pnm_decode_peak_memory_stays_near_the_output_size", "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .output()
        .expect("start isolated PNM RSS probe");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // A stale filter can select zero tests and exit successfully.
    assert!(output.status.success() && stdout.lines().any(|line| line.ends_with(DONE)), "{stdout}\n{stderr}");
}

fn measure_peak_memory() {
    use craft_image_codecs::*;

    const SIDE: u32 = 4096; // 16 MiB of 8-bit gray output
    let out_bytes = (SIDE * SIDE) as u64;
    let mut file = format!("P5\n{SIDE} {SIDE}\n255\n").into_bytes();
    file.extend((0..out_bytes).map(|i| (i % 251) as u8));
    // Writing 5 resets the peak RSS to the current RSS; skip where the kernel doesn't allow it.
    if std::fs::write("/proc/self/clear_refs", "5").is_err() {
        return;
    }
    let Some(before) = peak_rss_bytes() else { return };
    let img = decode(&file).unwrap();
    let after = peak_rss_bytes().unwrap();
    assert_eq!(img.data().len() as u64, out_bytes);
    assert_eq!(img.data()[300], (300 % 251) as u8);
    // The output buffer plus slack; a `u32`-per-sample scratch copy alone would be 4× it.
    let grown = after.saturating_sub(before);
    assert!(grown < 2 * out_bytes, "decoding a {out_bytes} byte image raised peak RSS by {grown} bytes");
}
