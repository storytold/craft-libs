//! Compiles the vendored HarfBuzz subsetter amalgamation.

fn main() {
    let mut build = cc::Build::new();
    // HarfBuzz builds as C++11 without exceptions, RTTI or thread-safe
    // statics (its meson.build); with these it needs no libstdc++.
    build
        .cpp(true)
        .std("c++11")
        .warnings(false)
        .flag_if_supported("-fno-exceptions")
        .flag_if_supported("-fno-rtti")
        .flag_if_supported("-fno-threadsafe-statics")
        .flag_if_supported("-fvisibility-inlines-hidden")
        .file("harfbuzz/src/harfbuzz-subset.cc");
    if std::env::var("CARGO_CFG_TARGET_FAMILY").is_ok_and(|f| f.split(',').any(|f| f == "unix")) {
        // pthread mutexes rather than std::mutex, as meson configures it.
        build.define("HAVE_PTHREAD", "1");
    }
    build.compile("harfbuzz-subset");
    println!("cargo:rerun-if-changed=harfbuzz/src");
    println!("cargo:rerun-if-changed=build.rs");
}
