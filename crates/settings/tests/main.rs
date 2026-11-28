//! Integration tests, built into the library's own test executable.

// These use `tempfile`, the file system and `MemoryFiles`, none of which a browser build has.
#[cfg(not(target_arch = "wasm32"))]
mod common;
#[cfg(not(target_arch = "wasm32"))]
mod merge;
#[cfg(not(target_arch = "wasm32"))]
mod migrate;
#[cfg(not(target_arch = "wasm32"))]
mod recovery;
#[cfg(not(target_arch = "wasm32"))]
mod store;

mod local_storage;
