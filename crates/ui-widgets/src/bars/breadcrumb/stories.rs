//! Stories of `BreadcrumbBar`.

use super::BreadcrumbBar;
use crate::contract::{Entry, Show as _};
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::{Key, Modifiers, Ui};

fn crumbs() -> Vec<Entry<u8>> {
    ["photos", "2026", "trip", "day-one", "selects"].into_iter().zip(0u8..).map(|(name, at)| Entry::new(at, name)).collect()
}

#[story(BreadcrumbBar<'static, u8>, covers = [Default], drive = Drive::Never)]
fn breadcrumb_bar(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    BreadcrumbBar::new("Path", &crumbs()).show(ui).into()
}

/// A width the five crumbs do not fit in.
const NARROW: f32 = 160.0;

fn open_hidden(fixture: &mut Fixture) {
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::ALT);
}

#[story(BreadcrumbBar<'static, u8>, covers = [Open], drive = Drive::Custom(open_hidden))]
fn breadcrumb_bar_hidden_crumbs(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ui.set_max_width(NARROW);
    crate::testing::memory::scoped(ui, |ui| BreadcrumbBar::new("Path", &crumbs()).show(ui)).into()
}
