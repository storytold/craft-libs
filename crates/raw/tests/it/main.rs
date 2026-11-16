//! The integration tests, one module per file, linked into one test binary.

#[cfg(feature = "dng")]
mod dng;
#[cfg(feature = "line-dng")]
mod dng_lines;
mod raf_corpus_manifest;
#[cfg(feature = "dng")]
mod robust;
mod vendor_robust;
