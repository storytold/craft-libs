//! Embeds the fixture checkout in `../fixture/craft-fonts` the way an app's build.rs embeds
//! `CRAFT_FONTS_DIR`, so the generated table is compiled by a real `include!`.
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    let embedded = craft_fonts_build::Embed::new()
        .dir(root.join("../fixture/craft-fonts"))
        .required(true)
        .scripts(&["Jpan", "Hant"])
        .filter(|face| face.style == "Regular")
        .write("craft_fonts.rs");
    if let Err(e) = embedded {
        println!("cargo::error={e}");
    }
}
