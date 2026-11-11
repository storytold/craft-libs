//! Stories of `TabStrip`.

use super::{TabKind, TabStrip};
use crate::contract::{ReorderRule, ShowCollection as _};
use crate::tabs::Tab;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::{DisplayOrder, ItemSource};
use egui::{Key, Modifiers, Ui};
use std::borrow::Cow;

const TITLES: [&str; 12] = [
    "Untitled-1",
    "Photo.psd",
    "Poster final.psd",
    "Logo.ai",
    "Sketch",
    "Notes",
    "Brochure.indd",
    "Cover.psd",
    "Pattern",
    "Banner.psd",
    "Draft",
    "Export",
];

/// The width the overflow story's room is held to.
const OVERFLOW_ROOM: f32 = 240.0;

/// The tabs still open, keyed by their place in the title list and named by it.
struct Documents<'t> {
    open: &'t [usize],
    titles: &'static [&'static str],
}

impl DisplayOrder<usize> for Documents<'_> {
    fn len(&self) -> usize {
        self.open.len()
    }

    fn key(&self, at: usize) -> usize {
        self.open[at]
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        self.open.iter().position(|open| open == key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.titles.get(self.open[at]).map(|title| Cow::Borrowed(*title))
    }
}

impl ItemSource<usize> for Documents<'_> {}

/// A strip of `count` tabs; a closed tab leaves the story's list.
fn strip(ui: &mut Ui, data: &mut StoryData, strip: TabStrip<'_, usize>, titles: &'static [&'static str], count: usize, kind: TabKind) -> StoryOutput {
    let open = data.value("open", (0..count).collect::<Vec<usize>>());
    let out = strip.kind(kind).show_items(ui, &Documents { open, titles }, &mut |key, _| Tab::new(titles[*key]).closable(true));
    open.retain(|key| !out.closed.contains(key));
    out.into()
}

#[story(TabStrip<'static, usize>, covers = [Default, Many], drive = Drive::Never)]
fn tab_strip_many(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    strip(ui, data, TabStrip::new("Documents").shown(3).reorder(ReorderRule::new()), &TITLES, TITLES.len(), TabKind::Document)
}

fn open_overflow(fixture: &mut Fixture) {
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::ALT);
}

#[story(TabStrip<'static, usize>, covers = [Open], drive = Drive::Custom(open_overflow))]
fn tab_strip_overflow(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    crate::testing::memory::scoped(ui, |ui| {
        ui.set_max_width(OVERFLOW_ROOM);
        strip(ui, data, TabStrip::new("Documents").shown(0), &TITLES, TITLES.len(), TabKind::Document)
    })
}

#[story(TabStrip<'static, usize>, covers = [One], drive = Drive::Never)]
fn tab_strip_one(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    strip(ui, data, TabStrip::new("Documents").shown(0), &TITLES, 1, TabKind::Document)
}

#[story(TabStrip<'static, usize>, covers = [Empty], drive = Drive::Never)]
fn tab_strip_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    strip(ui, data, TabStrip::new("Documents"), &TITLES, 0, TabKind::Document)
}

#[story(TabStrip<'static, usize>, covers = [], drive = Drive::Never)]
fn tab_strip_panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    strip(ui, data, TabStrip::new("Panels").shown(1), &TITLES, 4, TabKind::Panel)
}
