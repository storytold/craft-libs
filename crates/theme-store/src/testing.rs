//! Shared fixtures for app adapter tests (feature `testing`).
//!
//! Every app writes [`SAMPLE_THEME_JSON`] into a temporary suite folder, follows the suite,
//! resolves in standard contrast and checks that the pinned tokens equal [`sample_colors`] in
//! each mode, so the same theme gives the same colours in every app.

use craft_theme::{ColorToken, Mode, Rgba};

/// The id of [`SAMPLE_THEME_JSON`]'s theme.
pub const SAMPLE_THEME_ID: &str = "sample.navy-cream";

/// A theme with both variants, pins in each mode, and unknown keys at the top level, in
/// `seeds` and under `apps`, which the store must keep byte for byte.
pub const SAMPLE_THEME_JSON: &str = r##"{
  "schema": "craft-design-theme",
  "schema_version": 1,
  "id": "sample.navy-cream",
  "name": "Navy and cream",
  "version": 3,
  "author": { "name": "Craft test fixtures", "url": null },
  "variants": "both",
  "seeds": {
    "accent": "#1F4E9C",
    "neutral": "#161C30",
    "status": { "good": "#2E7D32", "warn": "#9A6700", "bad": "#B3261E", "info": "#1F4E9C" },
    "sparkle": "#123456"
  },
  "light": {
    "surface.background": "#F2F1EE", "surface.panel": "#FBFAF7", "surface.raised": "#FFFFFF",
    "text.normal": "#1B2340", "text.muted": "#4A5068", "line.border": "#D9D6CC",
    "accent.solid": "#1F4E9C", "text.on_accent": "#FFFFFF", "state.selected": "#D6E0F2",
    "line.focus": "#1F4E9C"
  },
  "dark": {
    "surface.background": "#0F1424", "surface.panel": "#161C30", "surface.raised": "#1E2540",
    "text.normal": "#F2EAD8", "text.muted": "#C4BCA8", "line.border": "#2C3452",
    "accent.solid": "#E8C27A", "text.on_accent": "#1B1606", "state.selected": "#2F3A63",
    "line.focus": "#E8C27A"
  },
  "apps": { "pdfcraft": { "tokens_version": 1, "dark": { "surface.canvas": "#101010" } } },
  "family": {}
}
"##;

/// The sample's light pins, lower-case `#rrggbb`.
pub const SAMPLE_LIGHT: [(ColorToken, &str); 10] = [
    (ColorToken::SurfaceBackground, "#f2f1ee"),
    (ColorToken::SurfacePanel, "#fbfaf7"),
    (ColorToken::SurfaceRaised, "#ffffff"),
    (ColorToken::TextNormal, "#1b2340"),
    (ColorToken::TextMuted, "#4a5068"),
    (ColorToken::LineBorder, "#d9d6cc"),
    (ColorToken::AccentSolid, "#1f4e9c"),
    (ColorToken::TextOnAccent, "#ffffff"),
    (ColorToken::StateSelected, "#d6e0f2"),
    (ColorToken::LineFocus, "#1f4e9c"),
];

/// The sample's dark pins, lower-case `#rrggbb`.
pub const SAMPLE_DARK: [(ColorToken, &str); 10] = [
    (ColorToken::SurfaceBackground, "#0f1424"),
    (ColorToken::SurfacePanel, "#161c30"),
    (ColorToken::SurfaceRaised, "#1e2540"),
    (ColorToken::TextNormal, "#f2ead8"),
    (ColorToken::TextMuted, "#c4bca8"),
    (ColorToken::LineBorder, "#2c3452"),
    (ColorToken::AccentSolid, "#e8c27a"),
    (ColorToken::TextOnAccent, "#1b1606"),
    (ColorToken::StateSelected, "#2f3a63"),
    (ColorToken::LineFocus, "#e8c27a"),
];

/// The sample's pinned colours in `mode`, for comparing with an app's tokens.
pub fn sample_colors(mode: Mode) -> Vec<(ColorToken, Rgba)> {
    let table = match mode {
        Mode::Light => &SAMPLE_LIGHT,
        Mode::Dark => &SAMPLE_DARK,
    };
    table.iter().map(|&(token, hex)| (token, Rgba::from_hex(hex).unwrap_or(Rgba::BLACK))).collect()
}

#[cfg(not(target_arch = "wasm32"))]
pub use fault::{FailAt, FailingFs};

#[cfg(not(target_arch = "wasm32"))]
mod fault {
    use std::{
        fs::{File, Permissions},
        io,
        path::Path,
    };

    use craft_atomic::{AtomicFs, NewFilePermissions, RealFs};

    /// The step of an atomic write that [`FailingFs`] fails.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum FailAt {
        #[allow(missing_docs)]
        Create,
        #[allow(missing_docs)]
        Sync,
        #[allow(missing_docs)]
        Rename,
        #[allow(missing_docs)]
        HardLink,
        #[allow(missing_docs)]
        Remove,
    }

    /// The real file system with one step failing, for
    /// [`crate::StoreOptions::file_system`].
    #[derive(Clone, Copy, Debug)]
    pub struct FailingFs(pub FailAt);

    impl FailingFs {
        fn check(&self, at: FailAt) -> io::Result<()> {
            if self.0 == at { Err(io::Error::other(format!("injected failure at {at:?}"))) } else { Ok(()) }
        }
    }

    impl AtomicFs for FailingFs {
        fn create_new(&self, path: &Path, permissions: NewFilePermissions) -> io::Result<File> {
            self.check(FailAt::Create)?;
            RealFs.create_new(path, permissions)
        }

        fn sync(&self, file: &File) -> io::Result<()> {
            self.check(FailAt::Sync)?;
            RealFs.sync(file)
        }

        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.check(FailAt::Rename)?;
            RealFs.rename(from, to)
        }

        fn hard_link(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.check(FailAt::HardLink)?;
            RealFs.hard_link(from, to)
        }

        fn remove(&self, path: &Path) -> io::Result<()> {
            self.check(FailAt::Remove)?;
            RealFs.remove(path)
        }

        fn sync_dir(&self, dir: &Path) -> io::Result<()> {
            RealFs.sync_dir(dir)
        }

        fn set_permissions(&self, file: &File, perms: Permissions) -> io::Result<()> {
            RealFs.set_permissions(file, perms)
        }
    }
}
