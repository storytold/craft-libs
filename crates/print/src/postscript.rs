//! Windows: the PostScript route. A job drawn into the XPS print API reaches the driver as RGB,
//! so the document's CMYK is turned into RGB and back by the driver and prints paler than from a
//! program that sends PostScript (Acrobat does). When Ghostscript is installed and the driver
//! takes `POSTSCRIPT_PASSTHROUGH`, each sheet is converted to EPS by Ghostscript (colours as in
//! the document: CMYK, spot colours, overprint) and passed through the driver, which still
//! applies the job's validated settings. Without either, the job takes the XPS route as before.
//! This is the one place a program is started, and only an installed Ghostscript: where it is
//! blocked or fails, the job takes the XPS route.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The newest `gswin64c.exe` among Ghostscript's install folders (`gs10.04.0`, …, each with
/// `bin\gswin64c.exe`), by version; folders that aren't a version are skipped.
pub(crate) fn newest_ghostscript(folders: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    folders
        .into_iter()
        .filter_map(|folder| {
            let name = folder.file_name()?.to_str()?.strip_prefix("gs")?.to_string();
            let version: Vec<u32> = name.split('.').map(str::parse).collect::<Result<_, _>>().ok()?;
            Some((version, folder.join("bin").join("gswin64c.exe")))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, exe)| exe)
}

/// Ghostscript's arguments: every page of `pdf` to its own EPS (`pattern` holds `%d`), its
/// colours unchanged — or grey for a greyscale job.
pub(crate) fn ghostscript_args(pdf: &Path, pattern: &Path, grayscale: bool) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["-q", "-dNOPAUSE", "-dBATCH", "-dSAFER", "-sDEVICE=eps2write"].map(OsString::from).into();
    if grayscale {
        args.extend(["-sColorConversionStrategy=Gray", "-dProcessColorModel=/DeviceGray"].map(OsString::from));
    }
    let mut output = OsString::from("-sOutputFile=");
    output.push(pattern);
    args.push(output);
    args.push(pdf.into());
    args
}

/// Where a sheet goes on the paper (points from its top-left): centred, at 100%, smaller only
/// when the paper is.
pub(crate) fn sheet_box(paper: (f64, f64), sheet: (f64, f64)) -> (f64, f64, f64, f64) {
    let scale = (paper.0 / sheet.0).min(paper.1 / sheet.1).min(1.0);
    let (w, h) = (sheet.0 * scale, sheet.1 * scale);
    ((paper.0 - w) / 2.0, (paper.1 - h) / 2.0, w, h)
}

/// A DEVMODE as the PrintTicket conversion returns it, cut to the length its header gives
/// (`dmSize` + `dmDriverExtra`): the conversion's buffer can be longer.
pub(crate) fn devmode(mut bytes: Vec<u8>) -> Vec<u8> {
    let word = |at: usize| bytes.get(at..at + 2).map(|w| usize::from(u16::from_le_bytes([w[0], w[1]])));
    if let (Some(size), Some(extra)) = (word(68), word(70)) {
        bytes.truncate(size + extra);
    }
    bytes
}
