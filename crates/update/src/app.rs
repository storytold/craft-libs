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
    /// The bundle id when it isn't `ai.storyteller.<id>` (a community app brings its own).
    pub bundle: Option<&'static str>,
    /// Older bundle ids its Flatpak shipped under (SolidCraft was `ai.storyteller.solidcraft`).
    pub former_bundles: &'static [&'static str],
}

impl App {
    /// An app with no former names, no icon and the usual bundle id.
    pub const fn new(id: &'static str, name: &'static str, tagline: &'static str, repo: &'static str) -> Self {
        Self { id, name, tagline, repo, former_ids: &[], former_names: &[], icon_png: None, bundle: None, former_bundles: &[] }
    }

    /// macOS bundle id, Flatpak id and Linux desktop-file id: [`bundle`](Self::bundle), or
    /// `ai.storyteller.<id>`.
    pub fn bundle_id(&self) -> String {
        self.bundle.map_or_else(|| format!("ai.storyteller.{}", self.id), str::to_string)
    }

    /// Bundle ids this app may be installed under, current first.
    pub fn bundle_ids(&self) -> impl Iterator<Item = String> + '_ {
        std::iter::once(self.bundle_id()).chain(self.former_bundles.iter().map(|b| b.to_string()))
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
        assert_eq!(pdf.bundle_ids().collect::<Vec<_>>(), ["ai.storyteller.pdfcraft"]);
    }

    #[test]
    fn a_community_app_brings_its_own_bundle_id() {
        let sc = App {
            bundle: Some("io.tangencylabs.solidcraft"),
            former_bundles: &["ai.storyteller.solidcraft", "ai.storyteller.solvecraft"],
            ..App::new("solidcraft", "SolidCraft", "3D modeling", "tangency-labs/solidcraft")
        };
        assert_eq!(sc.bundle_id(), "io.tangencylabs.solidcraft");
        assert_eq!(sc.bundle_ids().collect::<Vec<_>>(), ["io.tangencylabs.solidcraft", "ai.storyteller.solidcraft", "ai.storyteller.solvecraft"]);
    }
}
