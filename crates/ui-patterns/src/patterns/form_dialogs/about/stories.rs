//! Stories of `AboutDialog`: the About tab (centred and listed), the Contributors tab with a PR-only contributor, and an app tab.

use super::{AboutAppTab, AboutDialog, AboutImage, AboutInfo, AboutLayout, AboutLink, AboutTab, Contributor, Credits, HostedAbout, ModelCredit};
use crate::contract::Show as _;
use crate::dialog::stories::hosted;
use crate::display::Text;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Color32, ColorImage, Ui};
use std::sync::{Arc, LazyLock};

/// A diagonal gradient standing for the app's mark.
fn mark() -> Arc<ColorImage> {
    const SIDE: usize = 32;
    let pixels = (0..SIDE * SIDE).map(|at| Color32::from_rgb((at % SIDE * 255 / SIDE) as u8, (at / SIDE * 255 / SIDE) as u8, 160)).collect();
    Arc::new(ColorImage::new([SIDE, SIDE], pixels))
}

fn info(community: bool, with_mark: bool) -> AboutInfo {
    let mut info = AboutInfo::new("PhotoCraft", "An open-source, native image editor written in Rust.", "0.42.1 (a1b2c3d, 2026-10-30)")
        .links([AboutLink::new("Website", "https://example.org"), AboutLink::new("Source", "https://example.org/source")])
        .built_with("egui · wgpu");
    if community {
        info = info.community(AboutLink::new("Join us on Discord", "https://discord.example/join"));
    }
    if with_mark {
        info = info.icon(mark());
    }
    info
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
    let mark = *data.value("mark", true);
    hosted(ui, data, (tab, community, mark), || HostedAbout::new(info(community, mark), CREDITS.clone()).tab(tab))
}

/// The About tab in the listed layout: name, version, blurb and links one to a line, and Close.
#[story(AboutDialog<'static>, covers = [], drive = Drive::Never)]
fn about_dialog_listed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let community = *data.value("community_link", true);
    hosted(ui, data, community, || HostedAbout::new(info(community, false), CREDITS.clone()).layout(AboutLayout::Listed))
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
        HostedAbout::new(info(community, false), CREDITS.clone()).app_tab(languages).tab(AboutTab::App(0))
    })
}

/// The story's app mark: a rounded square with a page corner folded down.
const MARK: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48"><rect x="4" y="4" width="40" height="40" rx="10" fill="#3b6fd8"/><path d="M16 12h12l8 8v16H16z" fill="#ffffff"/><path d="M28 12v8h8z" fill="#c9d8f7"/></svg>"##;

/// The story's suite logo: a dot and a bar, dark ink for a light theme.
const SUITE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="5" cy="8" r="4" fill="#202124"/><rect x="10" y="4" width="4" height="8" rx="1" fill="#202124"/></svg>"##;

/// The suite logo in light ink, for a dark theme.
const SUITE_DARK: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="5" cy="8" r="4" fill="#f1f3f4"/><rect x="10" y="4" width="4" height="8" rx="1" fill="#f1f3f4"/></svg>"##;

#[story(AboutDialog<'static>, covers = [], drive = Drive::Never, title = "About with a mark and suite logo")]
fn about_dialog_with_mark(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    hosted(ui, data, AboutTab::About, || {
        let marked = info(true, false).mark(AboutImage::svg("PhotoCraft", MARK)).part_of(AboutImage::svg("Craft Suite", SUITE).dark(SUITE_DARK));
        HostedAbout::new(marked, CREDITS.clone())
    })
}

#[story(AboutDialog<'static>, covers = [], drive = Drive::Never, title = "About with icon links")]
fn about_dialog_icon_links(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    hosted(ui, data, AboutTab::About, || {
        let links = [
            AboutLink::new("Website", "https://example.org").icon(craft_behavior::IconId::new("globe")),
            AboutLink::new("Source", "https://example.org/source").icon(craft_behavior::IconId::new("file")),
        ];
        HostedAbout::new(info(false, false).links(links), CREDITS.clone())
    })
}

#[story(AboutDialog<'static>, covers = [], drive = Drive::Never)]
fn about_dialog_contributors(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tab = *data.choice("tab", AboutTab::Contributors, &TABS);
    let community = *data.value("community_button", true);
    hosted(ui, data, (tab, community), || HostedAbout::new(info(community, false), CREDITS.clone()).tab(tab))
}
