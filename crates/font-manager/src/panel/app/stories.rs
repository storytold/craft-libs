//! Stories of `FontsPanelView` over the story panel: a small index, two installed faces and a fake font store.

use std::sync::{Arc, Mutex, PoisonError};

use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::{FontsPanel, FontsPanelView, Pending, Tab};
use crate::panel::fixtures::story_panel;

/// The panel a story keeps between passes. Equal when it is the same panel.
#[derive(Clone)]
struct Held(Arc<Mutex<FontsPanel>>);

impl PartialEq for Held {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Shows the story panel on `tab` after `setup` changed it, on the first pass only.
fn show(ui: &mut Ui, data: &mut StoryData, tab: Tab, setup: fn(&mut FontsPanel)) -> StoryOutput {
    let slot = data.value("panel", None::<Held>);
    if slot.is_none() {
        let mut panel = story_panel();
        panel.set_tab(tab);
        setup(&mut panel);
        *slot = Some(Held(Arc::new(Mutex::new(panel))));
    }
    let Some(held) = slot.clone() else { return StoryOutput::default() };
    let mut panel = held.0.lock().unwrap_or_else(PoisonError::into_inner);
    FontsPanelView::new(&mut panel).show(ui).into()
}

#[story(FontsPanelView<'static>, covers = [Default], drive = Drive::Never)]
fn fonts_panel_installed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, Tab::Installed, |_| {})
}

#[story(FontsPanelView<'static>, covers = [], drive = Drive::Never)]
fn fonts_panel_catalogue(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, Tab::Catalogue, |panel| {
        let _ = panel.set_downloads_allowed(true);
    })
}

#[story(FontsPanelView<'static>, covers = [], drive = Drive::Never)]
fn fonts_panel_downloads_off(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, Tab::Catalogue, |panel| {
        let _ = panel.set_downloads_allowed(false);
    })
}

#[story(FontsPanelView<'static>, covers = [Empty], drive = Drive::Never)]
fn fonts_panel_no_results(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, Tab::Catalogue, |panel| panel.query.text = "no such family".to_owned())
}

#[story(FontsPanelView<'static>, covers = [], drive = Drive::Never)]
fn fonts_panel_consent(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, Tab::Catalogue, |panel| {
        let _ = panel.set_downloads_allowed(false);
        panel.asking = Some(Pending::Install(0));
    })
}
