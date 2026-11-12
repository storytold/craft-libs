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
    // The default folder is created fresh (`create_dir` fails on a planted directory or symlink);
    // an explicit one is the operator's choice.
    let out = match args.first() {
        Some(dir) => {
            std::fs::create_dir_all(dir)?;
            std::path::PathBuf::from(dir)
        }
        None => {
            let mut n = 0u32;
            loop {
                let dir = std::env::temp_dir().join(format!("craft-psd-big-psb-{}-{n}", std::process::id()));
                #[cfg_attr(not(unix), allow(unused_mut))]
                let mut builder = std::fs::DirBuilder::new();
                #[cfg(unix)]
                std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
                match builder.create(&dir) {
                    Ok(()) => {
                        // Kept after the run (the fixtures are the point): remove it when done.
                        eprintln!("writing fixtures to {} (remove it when done)", dir.display());
                        break dir;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && n < 1000 => n += 1,
                    Err(e) => return Err(e),
                }
            }
        }
    };
    let wanted = args.get(1..).unwrap_or(&[]);
    for case in PSB_CASES.iter().chain([&PSD_2GB]).filter(|c| wanted.is_empty() || wanted.iter().any(|w| w == c.name)) {
        let path = out.join(case.file_name());
        let t = std::time::Instant::now();
        // In the fresh default folder only: a half-written fixture is removed, but never a file an
        // operator-chosen folder already held.
        let len = case.write_file(&path).inspect_err(|_| {
            if args.is_empty() {
                let _ = std::fs::remove_file(&path);
            }
        })?;
        println!("{}: {:.2} GB in {:.1} s", path.display(), len as f64 / 1e9, t.elapsed().as_secs_f64());
    }
    Ok(())
}
