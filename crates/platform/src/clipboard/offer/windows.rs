//! The offer on Windows.

/// `fragment` wrapped as a CF_HTML clipboard payload.
#[allow(dead_code)] // P3(platform-w2): pf.C-P7
pub(super) fn wrap_cf_html(fragment: &str) -> Vec<u8> {
    let _ = fragment;
    todo!("pf.C-P7")
}
