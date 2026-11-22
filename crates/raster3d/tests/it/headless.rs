//! A test scene (`headless/scene.rs`) rendered natively and in `wasm32-unknown-unknown` modules
//! run without a browser, with the serial build (no `parallel` feature).
//!
//! The shadow texels, the depth and the encoded image rounded to 8 bits per channel hash to the
//! constants in `headless/scene.rs`. Only x86_64 Linux and wasm32 compare them, since those are the
//! targets they were measured on; every target runs the other checks.
//!
//! Natively: `cargo test -p craft-raster3d --test it headless::`. As a wasm32 test module, with
//! `wasm-bindgen-test-runner` of the `wasm-bindgen` version in `Cargo.lock` on the path and
//! Node.js installed:
//!
//! ```text
//! CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
//!   cargo test -p craft-raster3d --no-default-features --target wasm32-unknown-unknown --test it headless::
//! ```
//!
//! The ignored test checks the `headless_scene` example built as a wasm32 module: it is valid, has
//! no imports, renders the scene in `wasmi` with hashes that match, and grows its memory to at
//! most `MAX_PAGES`. Build the example, then run the test with `CRAFT_RASTER3D_HEADLESS_WASM` set
//! to the module's path. With the release build the run takes about a minute in an unoptimized
//! test build:
//!
//! ```text
//! cargo build -p craft-raster3d --no-default-features --release --target wasm32-unknown-unknown --example headless_scene
//! CRAFT_RASTER3D_HEADLESS_WASM=target/wasm32-unknown-unknown/release/examples/headless_scene.wasm \
//!   cargo test -p craft-raster3d --test it headless:: -- --ignored
//! ```

mod scene;

use craft_raster3d::Scene;
use scene::{IMAGE_HASH, Render, SHADOW_HASH, f32_bytes, fnv, hashes, image_bytes, render, to_u8};

/// A hash of every bit of a render, for comparing two renders on one target.
fn exact_hash((shadows, pixels, depth): &Render) -> u64 {
    fnv(f32_bytes(shadows).chain(pixels.iter().flat_map(|p| f32_bytes(p))).chain(f32_bytes(depth)))
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn the_test_scene_renders_the_same_on_every_target() {
    let first = render(scene::scene());
    let (shadows, pixels, _) = &first;
    // The scene draws enough for a match to mean something: most shadow texels hold a caster, a
    // quarter of the pixels are covered, in many colors.
    assert!(shadows.iter().filter(|t| t.is_finite()).count() > 1000);
    assert!(pixels.iter().filter(|p| p[3] > 0.0).count() * 4 > pixels.len());
    let bytes = image_bytes(pixels);
    let mut colors: Vec<&[u8]> = bytes.chunks(4).collect();
    colors.sort_unstable();
    colors.dedup();
    assert!(colors.len() > 100, "{} colors", colors.len());
    // A second render on the same target matches bit for bit.
    assert_eq!(exact_hash(&render(scene::scene())), exact_hash(&first));
    // Other native targets' math libraries can round differently, so only the two measured
    // targets compare the hashes.
    if cfg!(any(target_arch = "wasm32", all(target_os = "linux", target_arch = "x86_64"))) {
        let (shadow_hash, image_hash) = hashes(&first);
        assert_eq!(shadow_hash, SHADOW_HASH, "shadow texels {shadow_hash:016x}");
        assert_eq!(image_hash, IMAGE_HASH, "image {image_hash:016x}");
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn the_mask_the_lights_and_the_shadows_each_change_the_image() {
    let (_, base, _) = render(scene::scene());
    let changed = |f: &dyn Fn(&mut Scene)| {
        let mut s = scene::scene();
        f(&mut s);
        let (_, pixels, _) = render(s);
        pixels.iter().zip(&base).filter(|(a, b)| a.map(to_u8) != b.map(to_u8)).count()
    };
    // The masked card drawn opaque covers the checker's holes.
    assert!(changed(&|s| s.materials[2].alpha_mode = 0) > 20);
    // Without the spot light, or without any shadow maps, the image changes.
    assert!(changed(&|s| s.lights.truncate(1)) > 20);
    assert!(
        changed(&|s| {
            s.lights.iter_mut().for_each(|l| l.shadow = -1);
            if let Some(e) = s.env.as_mut() {
                e.shadow = -1;
            }
        }) > 20
    );
}

/// The largest memory the `headless_scene` module may grow to, in 64 KiB pages (8 MiB). The release
/// build of the example grows to 118 pages.
#[cfg(not(target_arch = "wasm32"))]
const MAX_PAGES: u64 = 128;

#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "needs the headless_scene example built for wasm32 and CRAFT_RASTER3D_HEADLESS_WASM set to it"]
fn the_wasm32_example_has_no_imports_and_renders_the_scene() {
    let path = std::env::var_os("CRAFT_RASTER3D_HEADLESS_WASM")
        .expect("set CRAFT_RASTER3D_HEADLESS_WASM to the headless_scene example built for wasm32-unknown-unknown");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.to_string_lossy()));
    let report = craft_wasm::inspect(&bytes).unwrap();
    assert!(report.validation.valid, "{:?}", report.validation);
    assert!(report.imports.is_empty(), "imports: {:?}", report.imports);

    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, &bytes[..]).unwrap();
    let mut store = wasmi::Store::new(&engine, ());
    let instance = wasmi::Linker::<()>::new(&engine).instantiate_and_start(&mut store, &module).unwrap();
    let main = instance.get_typed_func::<(i32, i32), i32>(&store, "main").unwrap();
    // 0: both hashes match (see `examples/headless_scene.rs`).
    assert_eq!(main.call(&mut store, (0, 0)).unwrap(), 0);
    let pages = instance.get_memory(&store, "memory").unwrap().size(&store);
    assert!(pages <= MAX_PAGES, "{pages} pages");
}
