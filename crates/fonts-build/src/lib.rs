#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Embed the optional [craft-fonts](https://github.com/storytold/craft-fonts) faces from an app's
//! `build.rs`.
//!
//! Apps never depend on craft-fonts in `Cargo.toml` (Cargo would fetch the font repository for
//! every build). The build reads a local checkout named by `CRAFT_FONTS_DIR` instead:
//!
//! - **Unset or empty:** the table is empty and the app uses its own and system fonts.
//! - **Set:** every selected face in `fonts/manifest.txt` is embedded with `include_bytes!`.
//! - **Set, but not readable as a checkout:** a `cargo::warning` and an empty table.
//! - **`CRAFT_FONTS_REQUIRED=1`** (release builds; any other value is ignored): a missing or
//!   unreadable checkout, no selected face, or a requested script without a face is an [`Error`].
//!
//! Add this crate both as a build-dependency (for [`Embed`]) and as a dependency (for
//! [`EmbeddedFace`]). The build-dependency enables the `subset` feature for `Embed::subset_to`,
//! which cuts the CJK interface faces to the translation catalogs' characters; the dependency
//! leaves it off, so the subsetter is not part of the app. In `build.rs`'s `main`:
//!
//! ```ignore
//! println!("cargo::rerun-if-changed=build.rs");
//! let wasm = craft_fonts_build::target_is_wasm();
//! let catalogs: Vec<std::path::PathBuf> = /* the app's translation catalogs */;
//! let embedded = craft_fonts_build::Embed::new()
//!     // CJK faces only as the interface faces, cut to the characters the catalogs use.
//!     .subset_to(catalogs)
//!     // Web builds have a size budget: embed only the Japanese interface face there.
//!     .filter(move |face| !wasm || face.has_script("Jpan"))
//!     .write("craft_fonts.rs");
//! if let Err(e) = embedded {
//!     println!("cargo::error={e}");
//! }
//! ```
//!
//! and in the crate:
//!
//! ```ignore
//! use craft_fonts_build::EmbeddedFace;
//!
//! pub static CRAFT_FONTS: &[EmbeddedFace] = include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));
//! ```
//!
//! The generated file is a slice expression of `EmbeddedFace { .. }` literals, so any struct with
//! the same fields and that name in scope works too.
//!
//! An app's document fonts go in a table of their own that names its families,
//! `Embed::documents(&["Noto Sans", "Shippori Mincho"])`: nothing is embedded for documents
//! unless the app names it.
//!
//! Web builds can serve the faces beside the wasm instead: [`Embed::serve`] copies the selected
//! files into a directory and writes a table of [`ServedFace`]s, which carry a URL in place of the
//! bytes, for the app to fetch the faces it needs at run time.

mod cmap;
pub mod coverage;
mod embed;
mod manifest;
#[cfg(feature = "subset")]
mod sha256;
#[cfg(feature = "subset")]
mod subset;
#[cfg(test)]
mod tests;

pub use embed::{Embed, Error, target_is_wasm};
pub use manifest::{CJK_SCRIPTS, ManifestEntry, ParseError, UI_ROLE, parse_manifest};

/// A craft-fonts face embedded by [`Embed`]: one line of `fonts/manifest.txt`.
///
/// [`Embed::serve`] writes [`ServedFace`]s instead.
#[derive(Clone, Copy)]
pub struct EmbeddedFace {
    /// Family name, e.g. `"Noto Sans Arabic"`.
    pub family: &'static str,
    /// Style, e.g. `"Regular"` or `"Bold"`.
    pub style: &'static str,
    /// Script tags: ISO 15924 codes such as `Arab`, `Jpan`, `Hans`, `Hant`, optionally with a
    /// region subtag (`Hant-HK` for Hong Kong forms, which have no ISO 15924 code).
    pub scripts: &'static [&'static str],
    /// Face index in a font collection (`.ttc`); `0` for a single font.
    pub index: u32,
    pub bytes: &'static [u8],
}

impl EmbeddedFace {
    /// `"<family> <style>"`, e.g. `"Noto Sans Arabic Regular"`.
    pub fn name(&self) -> String {
        format!("{} {}", self.family, self.style)
    }

    /// Whether any script tag matches `tag`: `Hant` matches `Hant` and `Hant-HK`.
    pub fn has_script(&self, tag: &str) -> bool {
        self.scripts.iter().any(|s| manifest::script_matches(s, tag))
    }

    /// Whether the face has a CJK script tag ([`CJK_SCRIPTS`]).
    pub fn is_cjk(&self) -> bool {
        CJK_SCRIPTS.iter().any(|tag| self.has_script(tag))
    }

    /// CSS/OpenType weight implied by the style name (`Bold` → 700); 400 when it names none.
    pub fn weight(&self) -> u16 {
        let style = self.style.to_ascii_lowercase().replace([' ', '-'], "");
        const WEIGHTS: [(&str, u16); 14] = [
            ("extralight", 200),
            ("ultralight", 200),
            ("semibold", 600),
            ("demibold", 600),
            ("extrabold", 800),
            ("ultrabold", 800),
            ("thin", 100),
            ("light", 300),
            ("medium", 500),
            ("bold", 700),
            ("black", 900),
            ("heavy", 900),
            ("regular", 400),
            ("book", 400),
        ];
        WEIGHTS.iter().find(|(name, _)| style.contains(name)).map_or(400, |(_, weight)| *weight)
    }
}

/// A craft-fonts face that [`Embed::serve`] copied beside the app, to fetch at run time: one line
/// of `fonts/manifest.txt`.
#[derive(Clone, Copy, Debug)]
pub struct ServedFace {
    /// Family name, e.g. `"Noto Sans Arabic"`.
    pub family: &'static str,
    /// Style, e.g. `"Regular"` or `"Bold"`.
    pub style: &'static str,
    /// Script tags, as in [`EmbeddedFace::scripts`].
    pub scripts: &'static [&'static str],
    /// Face index in a font collection (`.ttc`); `0` for a single font. Faces of one collection
    /// share a URL.
    pub index: u32,
    /// Where the file is served, relative to the page unless the prefix given to
    /// [`Embed::serve`] makes it absolute.
    pub url: &'static str,
    /// File size in bytes.
    pub size: u64,
    /// Subresource Integrity value (`sha256-…`) from the manifest's hash; empty when the manifest
    /// has none.
    pub integrity: &'static str,
    /// The characters the face maps, read from its `cmap` (see [`coverage`]).
    pub coverage: &'static [u8],
}

impl ServedFace {
    /// `"<family> <style>"`, e.g. `"Noto Sans Arabic Regular"`.
    pub fn name(&self) -> String {
        format!("{} {}", self.family, self.style)
    }

    /// Whether any script tag matches `tag`: `Hant` matches `Hant` and `Hant-HK`.
    pub fn has_script(&self, tag: &str) -> bool {
        self.scripts.iter().any(|s| manifest::script_matches(s, tag))
    }

    /// Whether the face has a CJK script tag ([`CJK_SCRIPTS`]).
    pub fn is_cjk(&self) -> bool {
        CJK_SCRIPTS.iter().any(|tag| self.has_script(tag))
    }

    /// Whether the face maps `c`.
    pub fn covers(&self, c: char) -> bool {
        coverage::contains(self.coverage, c)
    }
}

impl std::fmt::Debug for EmbeddedFace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddedFace")
            .field("family", &self.family)
            .field("style", &self.style)
            .field("scripts", &self.scripts)
            .field("index", &self.index)
            .field("bytes", &format_args!("[{} bytes]", self.bytes.len()))
            .finish()
    }
}
