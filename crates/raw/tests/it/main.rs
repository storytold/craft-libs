//! The integration tests, one module per file, linked into one test binary.

#[cfg(feature = "dng")]
mod dng;
mod raf_corpus_manifest;
#[cfg(feature = "dng")]
mod robust;
mod vendor_robust;
