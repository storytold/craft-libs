fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    // The Arabic sample story's face from the craft fonts set: only for the stories, and never required, so an app
    // build with `CRAFT_FONTS_REQUIRED=1` doesn't fail over it.
    let stories = std::env::var_os("CARGO_FEATURE_SUITE").is_some();
    let written = craft_fonts_build::Embed::documents(&["Noto Sans Arabic"]).filter(move |_| stories).required(false).write("story_fonts.rs");
    if let Err(e) = written {
        println!("cargo::error={e}");
    }
}
