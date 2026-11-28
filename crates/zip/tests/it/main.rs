//! The crate's integration tests, linked into one test binary.

#[path = "../lenient_common/mod.rs"]
mod common;

mod aligned_writer;
mod archive;
mod lenient_zip64_end_record_compatibility;
mod lenient_zip64_extra_boundaries;
mod lenient_zip_decoded_size;
mod lenient_zip_directory_bounds;
mod lenient_zip_entry_detection;
mod lenient_zip_split_volumes;
mod lenient_zip_stored_cap;
mod public_api;
