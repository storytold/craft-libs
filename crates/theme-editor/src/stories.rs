//! Stories of the theme panels, over the first built-in theme.

use super::{AppLayerEditor, Pick, Problems, ThemeSubject, ThemeWorkspace, TokenBrowser, TokenDetail};
use craft_theme::report::ContrastReport;
use craft_theme::{AppSizes, Appearance, Contrast, Explained, Mode, Size, TextSize, builtins};
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;
use std::sync::LazyLock;

fn appearance() -> Appearance {
    Appearance { mode: Mode::Light, contrast: Contrast::Standard, size: Size::Compact, text_size: TextSize::Small, reduced_motion: false }
}

/// The explanation and contrast report of the first built-in theme; a built-in theme always explains.
fn explained() -> (&'static Explained, &'static ContrastReport) {
    static WORKSPACE: LazyLock<ThemeWorkspace> = LazyLock::new(|| {
        let mut workspace = ThemeWorkspace::new();
        workspace.refresh(&ThemeSubject::new(&builtins()[0], appearance()));
        workspace
    });
    let explained = WORKSPACE.explained().expect("a built-in theme explains");
    (explained, WORKSPACE.report().expect("a built-in theme has a contrast report"))
}

#[story(TokenBrowser<'static>, covers = [])]
fn token_browser(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    TokenBrowser::new(explained().0, Bind::bound(data.value("pick", None::<Pick>))).show(ui);
    StoryOutput::default()
}

// A read-only view of the pick: nothing in it takes input.
#[story(TokenDetail<'static>, covers = [], drive = Drive::Never)]
fn token_detail(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    TokenDetail::new(explained().0).show(ui);
    StoryOutput::default()
}

// A click on a pair reports its token; the story keeps the pick, as the host does.
#[story(Problems<'static>, covers = [], drive = Drive::Activate)]
fn problems(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (explained, report) = explained();
    if let Some(pick) = Problems::new(report, explained, &ThemeSubject::new(&builtins()[0], appearance())).show(ui).picked {
        *data.value("pick", None::<Pick>) = Some(pick);
    }
    StoryOutput::default()
}

/// The size the host has picked in the browser: its override editor shows only for a picked size.
fn picked_size() -> &'static Pick {
    static PICK: LazyLock<Pick> = LazyLock::new(|| {
        let first = explained().0.sizes.first().expect("a built-in theme has sizes");
        Pick::Size(first.path.clone())
    });
    &PICK
}

// The first size is picked, so the override editor and its controls show.
#[story(AppLayerEditor<'static>, covers = [], drive = Drive::Activate)]
fn app_layer_editor(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    AppLayerEditor::new(Bind::bound(data.value("sizes", AppSizes::default())), explained().0).pick(picked_size()).show(ui);
    StoryOutput::default()
}
