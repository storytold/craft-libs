//! Embeds the fixture checkout in `../fixture/craft-fonts` the way an app's build.rs embeds
//! `CRAFT_FONTS_DIR`, so the generated tables are compiled by a real `include!`: once embedded and
//! once served from `OUT_DIR/served`.
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default());
    let fonts = || {
        craft_fonts_build::Embed::new()
            .dir(root.join("../fixture/craft-fonts"))
            .required(true)
            .scripts(&["Jpan", "Hant"])
            .filter(|face| face.style == "Regular")
    };
    let embedded = fonts().write("craft_fonts.rs");
    let served = fonts().serve(out.join("served"), "fonts/").write("craft_fonts_served.rs");
    if let Err(e) = embedded.and(served) {
        println!("cargo::error={e}");
    }
}
