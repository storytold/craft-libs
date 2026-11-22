//! Render the headless test scene and compare it with the hashes measured for it.
//!
//! Built for `wasm32-unknown-unknown` with the serial build, the program is a module with no
//! imports that exports `main(argc, argv)` and its memory; `tests/headless.rs` checks the import
//! list and runs the module without a browser or JavaScript. The exit status is 0 when the shadow
//! texels and the image match `SHADOW_HASH` and `IMAGE_HASH`, 1 when the shadow texels differ and
//! 2 when the image differs. A render error stops the program (a trap in the module).
//!
//! `cargo build -p craft-raster3d --no-default-features --release --target wasm32-unknown-unknown --example headless_scene`
// Unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../tests/headless/scene.rs"]
mod scene;

use std::process::ExitCode;

fn main() -> ExitCode {
    let (shadow, image) = scene::hashes(&scene::render(scene::scene()));
    if shadow != scene::SHADOW_HASH {
        ExitCode::from(1)
    } else if image != scene::IMAGE_HASH {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}
