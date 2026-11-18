//! Printing in the browser.

/// Prints `pdf` through one hidden iframe; the wasm `Spool` backend calls it.
pub fn print_pdf(pdf: &[u8]) -> Result<(), String> {
    let _ = pdf;
    todo!("pf.C-PR4")
}
