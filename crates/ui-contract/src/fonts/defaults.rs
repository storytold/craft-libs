//! The suite's default fonts: its interface face, Inter, and egui's default faces, stored deflated.

use std::sync::{Arc, LazyLock};

use super::SEMIBOLD;
use egui::{FontData, FontDefinitions, FontFamily, FontTweak};

/// A font deflated by build.rs.
macro_rules! deflated {
    ($name:literal) => {
        include_bytes!(concat!(env!("OUT_DIR"), "/egui_fonts/", $name, ".ttf.deflate"))
    };
}

/// The names of the suite's interface faces among the default faces.
pub(super) const SUITE_FACES: [&str; 2] = ["Inter-Regular", "Inter-SemiBold"];

static HACK: LazyLock<Vec<u8>> = LazyLock::new(|| inflate(deflated!("Hack-Regular")));
static EGUI_ICONS: LazyLock<Vec<u8>> = LazyLock::new(|| inflate(deflated!("egui-icons")));
static UBUNTU_LIGHT: LazyLock<Vec<u8>> = LazyLock::new(|| inflate(deflated!("Ubuntu-Light")));
static INTER_REGULAR: LazyLock<Vec<u8>> = LazyLock::new(|| inflate(deflated!("Inter-Regular")));
static INTER_SEMIBOLD: LazyLock<Vec<u8>> = LazyLock::new(|| inflate(deflated!("Inter-SemiBold")));

/// Inflated fonts are bounded (the largest is 0.42 MB), so corrupt data can't exhaust memory.
const MAX_INFLATED_BYTES: u64 = 16 << 20;

/// Inflates a font. Data that doesn't inflate gives empty bytes (the font is left out), never a panic.
fn inflate(deflated: &[u8]) -> Vec<u8> {
    use std::io::Read as _;
    let mut out = Vec::new();
    match flate2::read::DeflateDecoder::new(deflated).take(MAX_INFLATED_BYTES + 1).read_to_end(&mut out) {
        Ok(_) if out.len() as u64 <= MAX_INFLATED_BYTES => out,
        _ => Vec::new(),
    }
}

/// The suite's default font definitions: the interface face (Inter Regular leads the proportional family, Inter
/// SemiBold the [`SEMIBOLD`] family) ahead of egui's default faces (without egui's optional emoji fonts), with egui's
/// tweaks and family order. Every font stack starts from these, so an app that supplies no face of its own draws in
/// the suite's. The faces are stored deflated, for builds without egui's `default_fonts` (the web size gate); this
/// crate's build script makes the copies from `fonts/inter` and `epaint_default_fonts`.
pub fn deflated_default_definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::empty();
    let faces: [(&str, &'static [u8], f32); 5] = [
        ("Inter-Regular", INTER_REGULAR.as_slice(), 1.0),
        ("Inter-SemiBold", INTER_SEMIBOLD.as_slice(), 1.0),
        ("Hack", HACK.as_slice(), 1.0),
        ("Ubuntu-Light", UBUNTU_LIGHT.as_slice(), 1.0),
        // The handful of icons in `egui::special_emojis`, which no platform font has.
        ("egui-icons", EGUI_ICONS.as_slice(), 0.90),
    ];
    for (name, bytes, scale) in faces {
        if bytes.is_empty() {
            log::error!("egui default font {name} is unavailable");
            continue;
        }
        let data = FontData::from_static(bytes).tweak(FontTweak { scale, ..Default::default() });
        fonts.font_data.insert(name.to_owned(), Arc::new(data));
    }
    let stack = |names: &[&str]| -> Vec<String> { names.iter().filter(|n| fonts.font_data.contains_key(**n)).map(|n| (*n).to_owned()).collect() };
    // egui 0.37's family order: egui-icons last, as a fallback.
    let monospace = stack(&["Hack", "Ubuntu-Light", "egui-icons"]);
    let proportional = stack(&["Inter-Regular", "Ubuntu-Light", "egui-icons"]);
    let semibold = stack(&["Inter-SemiBold", "Ubuntu-Light", "egui-icons"]);
    fonts.families.insert(FontFamily::Monospace, monospace);
    fonts.families.insert(FontFamily::Proportional, proportional);
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), semibold);
    fonts
}
