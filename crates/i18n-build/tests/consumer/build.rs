//! Embeds the fixture checkout in `../fixture/craft-i18n` and the fixture app catalogs in
//! `../fixture/app` the way an app's build.rs embeds `CRAFT_I18N_DIR`, so the generated table is
//! compiled by a real `include!`.
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    let embedded = craft_i18n_build::Catalogs::new("demo")
        .dir(root.join("../fixture/craft-i18n"))
        .app_dir("../fixture/app")
        .required(true)
        .write("craft_i18n.rs");
    if let Err(e) = embedded {
        println!("cargo::error={e}");
    }
}
