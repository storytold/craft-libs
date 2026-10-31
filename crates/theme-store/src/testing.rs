//! Shared fixtures for app adapter tests (feature `testing`).
//!
//! Every app writes [`SAMPLE_THEME_JSON`] into a temporary suite folder, follows the suite,
//! resolves and checks that its core-mapped tokens equal [`sample_roles`] in each variant, so the
//! same theme gives the same core colours in every app.

use craft_theme::{CoreRoles, Rgba, Role, Variant};

pub const SAMPLE_THEME_ID: &str = "sample.navy-cream";

/// A complete theme with both variants and unknown keys at the top level, in a variant and under
/// `apps`, which the store must keep byte for byte.
pub const SAMPLE_THEME_JSON: &str = r##"{
  "schema": "craft-theme",
  "schema_version": 1,
  "id": "sample.navy-cream",
  "name": "Navy and cream",
  "version": 3,
  "author": { "name": "Craft test fixtures", "url": null },
  "light": {
    "background": "#F2F1EE", "surface": "#FBFAF7", "raised": "#FFFFFF",
    "text": "#1B2340", "text_muted": "#4A5068", "line": "#D9D6CC",
    "accent": "#1F4E9C", "on_accent": "#FFFFFF", "selection": "#D6E0F2",
    "focus": "#1F4E9C", "good": "#2E7D32", "warn": "#9A6700", "bad": "#B3261E",
    "sparkle": "#123456"
  },
  "dark": {
    "background": "#0F1424", "surface": "#161C30", "raised": "#1E2540",
    "text": "#F2EAD8", "text_muted": "#C4BCA8", "line": "#2C3452",
    "accent": "#E8C27A", "on_accent": "#1B1606", "selection": "#2F3A63",
    "focus": "#E8C27A", "good": "#7CC47F", "warn": "#E0B34A", "bad": "#F08A80"
  },
  "apps": { "pdfcraft": { "tokens_version": 1, "titlebar": "#101010" } },
  "family": {}
}
"##;

/// The sample's light roles, lower-case `#rrggbb`, in [`Role::ALL`] order.
pub const SAMPLE_LIGHT: [(Role, &str); 13] = [
    (Role::Background, "#f2f1ee"),
    (Role::Surface, "#fbfaf7"),
    (Role::Raised, "#ffffff"),
    (Role::Text, "#1b2340"),
    (Role::TextMuted, "#4a5068"),
    (Role::Line, "#d9d6cc"),
    (Role::Accent, "#1f4e9c"),
    (Role::OnAccent, "#ffffff"),
    (Role::Selection, "#d6e0f2"),
    (Role::Focus, "#1f4e9c"),
    (Role::Good, "#2e7d32"),
    (Role::Warn, "#9a6700"),
    (Role::Bad, "#b3261e"),
];

/// The sample's dark roles, lower-case `#rrggbb`, in [`Role::ALL`] order.
pub const SAMPLE_DARK: [(Role, &str); 13] = [
    (Role::Background, "#0f1424"),
    (Role::Surface, "#161c30"),
    (Role::Raised, "#1e2540"),
    (Role::Text, "#f2ead8"),
    (Role::TextMuted, "#c4bca8"),
    (Role::Line, "#2c3452"),
    (Role::Accent, "#e8c27a"),
    (Role::OnAccent, "#1b1606"),
    (Role::Selection, "#2f3a63"),
    (Role::Focus, "#e8c27a"),
    (Role::Good, "#7cc47f"),
    (Role::Warn, "#e0b34a"),
    (Role::Bad, "#f08a80"),
];

/// The sample's roles in `variant`, for comparing with an app's tokens.
pub fn sample_roles(variant: Variant) -> CoreRoles {
    let table = match variant {
        Variant::Light => &SAMPLE_LIGHT,
        Variant::Dark => &SAMPLE_DARK,
    };
    let mut roles = black_roles();
    for &(role, hex) in table {
        roles.set(role, Rgba::parse_hex(hex).unwrap_or(Rgba::BLACK));
    }
    roles
}

fn black_roles() -> CoreRoles {
    let b = Rgba::BLACK;
    CoreRoles {
        background: b,
        surface: b,
        raised: b,
        text: b,
        text_muted: b,
        line: b,
        accent: b,
        on_accent: b,
        selection: b,
        focus: b,
        good: b,
        warn: b,
        bad: b,
    }
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
        Create,
        Sync,
        Rename,
        HardLink,
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
