//! Writes multi-gigabyte PSB files (and the 2 GB PSD) for large-file tests (photocraft#375, photocraft#216), straight
//! from the Adobe PSD spec and independent of this crate's writer, so the fixtures don't inherit
//! its limits. The generator is `craft_psd::fixtures` (feature `testgen`).
//!
//! `cargo run --release -p craft-psd --example big_psb -- <out dir> [case...]`
//!
//! Every sample follows `fixtures::sample`, so a reader can check any pixel without the file. RLE
//! cases keep each row constant (tiny files, full decoded size); raw cases vary along x too.

use craft_psd::fixtures::{PSB_CASES, PSD_2GB};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.first().map_or_else(|| std::env::temp_dir().join("craft-psd-big-psb"), std::path::PathBuf::from);
    std::fs::create_dir_all(&out)?;
    let wanted = args.get(1..).unwrap_or(&[]);
    for case in PSB_CASES.iter().chain([&PSD_2GB]).filter(|c| wanted.is_empty() || wanted.iter().any(|w| w == c.name)) {
        let path = out.join(case.file_name());
        let t = std::time::Instant::now();
        let len = case.write_file(&path)?;
        println!("{}: {:.2} GB in {:.1} s", path.display(), len as f64 / 1e9, t.elapsed().as_secs_f64());
    }
    Ok(())
}
