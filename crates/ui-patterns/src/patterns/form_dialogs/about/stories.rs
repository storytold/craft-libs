//! Stories of `AboutDialog`: the About tab and the Contributors tab with a PR-only contributor.

use super::{AboutDialog, AboutInfo, AboutLink, AboutTab, Contributor, Credits, ModelCredit};
use crate::contract::Bind;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;
use std::sync::LazyLock;

fn info(community: bool) -> AboutInfo {
    let info = AboutInfo::new("PhotoCraft", "An open-source, native image editor written in Rust.", "0.42.1 (a1b2c3d, 2026-10-30)")
        .links([AboutLink::new("Website", "https://example.org"), AboutLink::new("Source", "https://example.org/source")])
        .built_with("egui · wgpu");
    if community { info.community(AboutLink::new("Join us on Discord", "https://discord.example/join")) } else { info }
}

const TABS: [(&str, AboutTab); 3] = [("About", AboutTab::About), ("Contributors", AboutTab::Contributors), ("Models", AboutTab::Models)];

static CREDITS: LazyLock<Credits> = LazyLock::new(|| {
    Credits::new(
        vec![
            Contributor::new("alice").counts(1, 12).lines(1_200, 200).active("2025-01-02T10:00:00Z", "2025-06-01T10:00:00Z"),
            Contributor::new("bob")
                .display_name("Robert")
                .counts(3, 40)
                .lines(9_000, 4_000)
                .binary(4, 1)
                .active("2025-03-01T10:00:00Z", "2026-01-01T10:00:00Z"),
            Contributor::new("carol").counts(2, 0),
        ],
        vec![ModelCredit::new("Acme", "Big", "1").work(60, 5_000, 100), ModelCredit::new("Acme", "Small", "2").work(1, 3, 0)],
        1_000,
    )
});

#[story(AboutDialog<'static>, covers = [], drive = Drive::Never)]
fn about_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tab = *data.choice("tab", AboutTab::About, &TABS);
    let community = *data.value("community_button", true);
    let info = info(community);
    AboutDialog::new(&info, &CREDITS).tab(tab).opened(Bind::bound(data.value("open", true))).show(ui).into()
}

#[story(AboutDialog<'static>, covers = [], drive = Drive::Never)]
fn about_dialog_contributors(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tab = *data.choice("tab", AboutTab::Contributors, &TABS);
    let community = *data.value("community_button", true);
    let info = info(community);
    AboutDialog::new(&info, &CREDITS).tab(tab).opened(Bind::bound(data.value("open", true))).show(ui).into()
}
