//! Separates a PDF into plates with Ghostscript's `tiffsep` device.
//!
//! Ghostscript is AGPL: it runs here only as an external program on a developer's machine. It is
//! never linked or copied into this crate, and its output is never committed.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write as _;
use std::path::Path;
use std::process::Command;

use crate::Error;
use crate::tiff::{self, Plate};

/// Overrides the Ghostscript program (default `gs` on `PATH`).
pub const GS_ENV: &str = "CRAFT_PLATE_TEST_GS";

/// Plate resolution in dpi; at 72 dpi one pixel is one scene point.
pub const RESOLUTION: f32 = 72.0;

/// The Ghostscript program to run.
pub fn program() -> OsString {
    std::env::var_os(GS_ENV).unwrap_or_else(|| "gs".into())
}

/// Ghostscript's version, or `None` when the program can't be run.
pub fn version() -> Option<String> {
    let output = Command::new(program()).arg("--version").output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Separate the first page of `pdf` into plates, keyed by separation name (`Cyan`, …, spot
/// names). Overprint is simulated as a separating RIP would print it (`-dOverprint=/enable`).
pub fn separate(pdf: &[u8], label: &str) -> Result<BTreeMap<String, Plate>, Error> {
    let work = work_dir(label)?;
    let input = work.path().join("in.pdf");
    // The directory is new and private; `create_new` still refuses anything already at the name.
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&input)
        .and_then(|mut file| file.write_all(pdf))
        .map_err(|e| gs_error(&input, e))?;
    let res = format!("-r{RESOLUTION}");
    let output_file = format!("-sOutputFile={}", work.path().join("p%d.tif").display());
    let run = Command::new(program())
        .args(["-q", "-dSAFER", "-dBATCH", "-dNOPAUSE", "-sDEVICE=tiffsep", &res, "-sCompression=none", "-dOverprint=/enable"])
        .arg(output_file)
        .arg(&input)
        .output()
        .map_err(|e| Error::Ghostscript(format!("{}: {e}", program().to_string_lossy())))?;
    if !run.status.success() {
        let stderr = String::from_utf8_lossy(&run.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(5).collect();
        return Err(Error::Ghostscript(format!("{label}: {}: {}", run.status, tail.into_iter().rev().collect::<Vec<_>>().join(" / "))));
    }
    let mut plates = BTreeMap::new();
    for entry in std::fs::read_dir(work.path()).map_err(|e| gs_error(work.path(), e))? {
        let path = entry.map_err(|e| gs_error(work.path(), e))?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()).and_then(plate_name) else { continue };
        let data = std::fs::read(&path).map_err(|e| gs_error(&path, e))?;
        let plate = tiff::read(&data).map_err(|e| Error::Tiff(format!("{}: {e}", path.display())))?;
        plates.insert(name.to_owned(), plate);
    }
    if plates.is_empty() {
        return Err(Error::Ghostscript(format!("{label}: no plates written")));
    }
    Ok(plates)
}

/// `p1(Cyan).tif` → `Cyan`; the composite `p1.tif` and later pages are not plates.
pub fn plate_name(file: &str) -> Option<&str> {
    file.strip_prefix("p1(")?.strip_suffix(").tif")
}

fn gs_error(path: &Path, e: std::io::Error) -> Error {
    Error::Ghostscript(format!("{}: {e}", path.display()))
}

/// A fresh temporary directory, removed on drop: created with a random name and owner-only
/// permissions, so another local user can't plant files or symlinks in it beforehand.
fn work_dir(label: &str) -> Result<tempfile::TempDir, Error> {
    let safe: String = label.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let prefix = format!("craft-plate-test-{safe}-");
    let mut builder = tempfile::Builder::new();
    builder.prefix(&prefix);
    #[cfg(unix)]
    builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700));
    builder.tempdir().map_err(|e| gs_error(&std::env::temp_dir(), e))
}

#[cfg(test)]
mod tests {
    use super::{plate_name, work_dir};

    #[test]
    fn names_plates_from_tiffsep_files() {
        assert_eq!(plate_name("p1(Cyan).tif"), Some("Cyan"));
        assert_eq!(plate_name("p1(PANTONE 871 C).tif"), Some("PANTONE 871 C"));
        assert_eq!(plate_name("p1.tif"), None);
        assert_eq!(plate_name("p2(Cyan).tif"), None);
    }

    #[test]
    fn work_dirs_are_fresh_and_private() {
        let (a, b) = (work_dir("same label").unwrap(), work_dir("same label").unwrap());
        assert_ne!(a.path(), b.path());
        assert_eq!(std::fs::read_dir(a.path()).unwrap().count(), 0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(a.path()).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0);
            assert_ne!(mode & 0o200, 0, "owner must be able to write");
        }
        let path = a.path().to_path_buf();
        drop(a);
        assert!(!path.exists());
    }
}
