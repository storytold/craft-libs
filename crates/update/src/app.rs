//! Which app an installer or an update works on.

/// A Craft App, as its releases and installers know it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct App {
    /// Machine name: repo, binary, package and asset stem (`photocraft`).
    pub id: &'static str,
    /// Display name (`PhotoCraft`).
    pub name: &'static str,
    /// What it's for, in a few words (English). Written into the menu entries installers add.
    pub tagline: &'static str,
    /// GitHub `owner/repo`. Releases are only ever downloaded from this repository.
    pub repo: &'static str,
    /// Older names its release assets may carry (PdfCraft shipped as PrintCraft until 0.2.1).
    pub former_ids: &'static [&'static str],
    /// Older display names its installers registered (PdfCraft's MSI said PrintCraft until 0.2.1).
    pub former_names: &'static [&'static str],
    /// Its 128 px PNG icon, for the menu entry written beside an AppImage. `None`: no icon.
    pub icon_png: Option<&'static [u8]>,
    /// Its accent colour (sRGB), for the update prompt's buttons and highlights: the app's brand
    /// colour, as ArtCraft Launcher shows it. `None`: the default blue.
    pub accent: Option<[u8; 3]>,
    /// Who makes it, shown as "by <developer>" in the update prompt. [`DEVELOPER`] unless the app
    /// says otherwise.
    pub developer: &'static str,
}

/// The developer of every app in the storytold organisation.
pub const DEVELOPER: &str = "ArtCraft";

impl App {
    /// An app with no former names and no icon.
    pub const fn new(id: &'static str, name: &'static str, tagline: &'static str, repo: &'static str) -> Self {
        Self { id, name, tagline, repo, former_ids: &[], former_names: &[], icon_png: None, accent: None, developer: DEVELOPER }
    }

    /// `ai.storyteller.<id>`: macOS bundle id, Flatpak id and Linux desktop-file id.
    pub fn bundle_id(&self) -> String {
        format!("ai.storyteller.{}", self.id)
    }

    pub fn releases_url(&self) -> String {
        format!("https://github.com/{}/releases", self.repo)
    }

    pub fn repo_url(&self) -> String {
        format!("https://github.com/{}", self.repo)
    }

    /// The app's page on getartcraft.com.
    pub fn website(&self) -> String {
        format!("https://getartcraft.com/apps/{}", self.id)
    }

    /// Asset stems this app's releases may use, current first.
    pub fn stems(&self) -> impl Iterator<Item = &'static str> + '_ {
        std::iter::once(self.id).chain(self.former_ids.iter().copied())
    }

    /// Display names this app's installers may have registered, current first.
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        std::iter::once(self.name).chain(self.former_names.iter().copied())
    }

    /// Looks up an app in the Craft App Suite catalog by its id.
    pub fn by_id(id: &str) -> Option<&'static App> {
        crate::suite::by_id(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn former_names_follow_the_current_one() {
        let pdf =
            App { former_ids: &["printcraft"], former_names: &["PrintCraft"], ..App::new("pdfcraft", "PdfCraft", "PDFs", "storytold/pdfcraft") };
        assert_eq!(pdf.stems().collect::<Vec<_>>(), ["pdfcraft", "printcraft"]);
        assert_eq!(pdf.names().collect::<Vec<_>>(), ["PdfCraft", "PrintCraft"]);
        assert_eq!(pdf.bundle_id(), "ai.storyteller.pdfcraft");
        assert_eq!(pdf.releases_url(), "https://github.com/storytold/pdfcraft/releases");
    }
}
