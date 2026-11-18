//! The offer on Linux and the BSDs.

/// Whether the target or MIME names in `offered` contain a format that can be read as `mime`.
#[allow(dead_code)] // P3(platform-w2): pf.C-P7
pub(super) fn offered_has(offered: &[String], mime: &str) -> bool {
    let _ = (offered, mime);
    todo!("pf.C-P7")
}

/// The target that carries RTF among `offered`.
#[allow(dead_code)] // P3(platform-w2): pf.C-P7
pub(super) fn rtf_target(offered: &[String]) -> Option<&'static str> {
    let _ = offered;
    todo!("pf.C-P7")
}

/// Whether a read of `len` bytes with `remaining` left under `cap` is complete.
#[allow(dead_code)] // P3(platform-w2): pf.C-P7
pub(super) fn complete(len: usize, remaining: usize, cap: usize) -> bool {
    let _ = (len, remaining, cap);
    todo!("pf.C-P7")
}
