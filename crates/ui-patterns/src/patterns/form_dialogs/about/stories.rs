//! Stories of `AboutDialog`: the About tab (centred and listed), the Contributors tab with a PR-only contributor, and an app tab.

use super::{AboutAppTab, AboutDialog, AboutInfo, AboutLayout, AboutLink, AboutTab, Contributor, Credits, HostedAbout, ModelCredit};
use crate::contract::Show as _;
use crate::dialog::stories::hosted;
use crate::display::Text;
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
    hosted(ui, data, (tab, community), || HostedAbout::new(info(community), CREDITS.clone()).tab(tab))
}

/// The About tab in the listed layout: name, version, blurb and links one to a line, and Close.
#[story(AboutDialog<'static>, covers = [], drive = Drive::Never)]
fn about_dialog_listed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let community = *data.value("community_link", true);
    hosted(ui, data, community, || HostedAbout::new(info(community), CREDITS.clone()).layout(AboutLayout::Listed))
}

/// The dialog with one app tab after Models, open on it.
#[story(AboutDialog<'static>, covers = [], drive = Drive::Never)]
fn about_dialog_app_tab(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let community = *data.value("community_button", true);
    hosted(ui, data, community, || {
        let languages = AboutAppTab::new("languageData", "Languages", |ui: &mut Ui| {
            Text::new("English (built in)").show(ui);
            Text::new("Français 1.2 · 98% translated").show(ui);
            Text::new("Deutsch 1.1 · 91% translated").show(ui);
        });
        HostedAbout::new(info(community), CREDITS.clone()).app_tab(languages).tab(AboutTab::App(0))
    })
}

#[story(AboutDialog<'static>, covers = [], drive = Drive::Never)]
fn about_dialog_contributors(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tab = *data.choice("tab", AboutTab::Contributors, &TABS);
    let community = *data.value("community_button", true);
    hosted(ui, data, (tab, community), || HostedAbout::new(info(community), CREDITS.clone()).tab(tab))
}
