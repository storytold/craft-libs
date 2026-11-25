//! Stories of `TabStrip`.

use super::{StripEnd, TabKind, TabStrip};
use crate::contract::{CollectionEvent, Entry, ReorderRule, ShowCollection as _};
use crate::runtime::commands::{CommandRef, EditCommands};
use crate::tabs::{PanelTabLook, Tab, TabRole};
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::{DisplayOrder, IconId, ItemSource, SelectionChange};
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
    let out = strip.kind(kind).show_items(ui, &Documents { open, titles }, &mut |key, _| Tab::new(titles[*key]));
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

/// Panel tabs in the pill look (`PanelTabLook::Pill`, a dock's `DockConfig::tab_look`): the shown tab on a filled
/// rounded pill in the middle of the strip, the others bare until the pointer rests on one.
#[story(TabStrip<'static, usize>, covers = [], drive = Drive::Never)]
fn tab_strip_panel_pill(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    strip(ui, data, TabStrip::new("Panels").shown(1).panel_look(PanelTabLook::Pill), &TITLES, 4, TabKind::Panel)
}

/// Panel tabs in the block look (`PanelTabLook::Block`, a dock's `DockConfig::tab_look`): the shown tab a square
/// `surface.control` block as tall as the strip, the others bare until the pointer rests on one.
#[story(TabStrip<'static, usize>, covers = [], drive = Drive::Never)]
fn tab_strip_panel_block(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    strip(ui, data, TabStrip::new("Panels").shown(1).panel_look(PanelTabLook::Block), &TITLES, 4, TabKind::Panel)
}

/// The width the narrow panel story's room is held to: the shown tab whole, the others at their first letters.
const NARROW_ROOM: f32 = 260.0;

#[story(TabStrip<'static, usize>, covers = [], drive = Drive::Never)]
fn tab_strip_panel_narrow(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    crate::testing::memory::scoped(ui, |ui| {
        ui.set_max_width(NARROW_ROOM);
        strip(ui, data, TabStrip::new("Panels").shown(1), &TITLES, TITLES.len(), TabKind::Panel)
    })
}

/// The ribbon's tab names; the last is the contextual tab.
const RIBBON: [&str; 6] = ["File", "Home", "Insert", "Layout", "Review", "Table Design"];

/// The ribbon tabs without the contextual one, and with it.
const PLAIN: [usize; 5] = [0, 1, 2, 3, 4];
const WITH_TABLE: [usize; 6] = [0, 1, 2, 3, 4, 5];

/// The role of a ribbon story's tab: the last one is contextual.
fn ribbon_role(key: &usize) -> TabRole {
    if *key == RIBBON.len() - 1 { TabRole::Contextual } else { TabRole::Standard }
}

/// A ribbon strip of the tabs `open`, showing Home until a tab is picked.
fn ribbon(ui: &mut Ui, data: &mut StoryData, strip: TabStrip<'_, usize>, open: &[usize]) -> StoryOutput {
    let shown = data.value("shown", 1_usize);
    let source = Documents { open, titles: &RIBBON };
    let out = strip
        .kind(TabKind::Ribbon)
        .shown(*shown)
        .roles(ribbon_role)
        .show_items(ui, &source, &mut |key, _| Tab::new(RIBBON[*key]).role(ribbon_role(key)));
    for event in &out.response.events {
        if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
            && let Some(key) = keys.last()
        {
            *shown = *key;
        }
    }
    out.into()
}

/// The rows of the Comments item's menu.
fn comment_rows() -> Vec<Entry<CommandRef>> {
    vec![Entry::new(EditCommands::Cut.into(), "New comment"), Entry::new(EditCommands::Paste.into(), "Show comments")]
}

/// A command item and a menu item at the strip's end.
fn end_items(rows: &[Entry<CommandRef>]) -> [StripEnd<'_>; 2] {
    [StripEnd::command("Share", IconId::new("link").into(), EditCommands::Copy), StripEnd::menu("Comments", IconId::new("list").into(), rows)]
}

/// The width the narrow ribbon story's room is held to: every tab whole, the end items icon only.
const RIBBON_NARROW_ROOM: f32 = 340.0;

/// The frames the leave story lists its contextual tab for before the source drops it.
const TABLE_FRAMES: u64 = 30;

#[story(TabStrip<'static, usize>, covers = [], drive = Drive::Never)]
fn tab_strip_ribbon(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ribbon(ui, data, TabStrip::new("Ribbon"), &WITH_TABLE)
}

#[story(TabStrip<'static, usize>, covers = [], drive = Drive::Never)]
fn tab_strip_ribbon_end_items(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = comment_rows();
    let items = end_items(&rows);
    ribbon(ui, data, TabStrip::new("Ribbon").trailing(&items), &PLAIN)
}

#[story(TabStrip<'static, usize>, covers = [], drive = Drive::Never)]
fn tab_strip_ribbon_end_items_narrow(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = comment_rows();
    let items = end_items(&rows);
    crate::testing::memory::scoped(ui, |ui| {
        ui.set_max_width(RIBBON_NARROW_ROOM);
        ribbon(ui, data, TabStrip::new("Ribbon").trailing(&items), &PLAIN)
    })
}

/// End items as framed buttons: two outlined, the menu one with its chevron, and the last filled with the accent.
#[story(TabStrip<'static, usize>, covers = [], drive = Drive::Never)]
fn tab_strip_ribbon_end_buttons(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = comment_rows();
    let items = [
        StripEnd::command("Comments", IconId::new("list").into(), EditCommands::Paste).variant(craft_theme::Variant::SECONDARY),
        StripEnd::menu("Editing", IconId::new("list").into(), &rows).variant(craft_theme::Variant::SECONDARY),
        StripEnd::command("Share", IconId::new("link").into(), EditCommands::Copy).variant(craft_theme::Variant::PRIMARY),
    ];
    ribbon(ui, data, TabStrip::new("Ribbon").trailing(&items), &PLAIN)
}

/// The contextual tab is listed for the first frames and then dropped by the source: it fades out at its slot. The
/// story reads the frame it is in rather than counting its runs, so a measuring run or a second pass of one frame
/// lists the same tabs as the frame's other runs.
#[story(TabStrip<'static, usize>, covers = [], drive = Drive::Never)]
fn tab_strip_ribbon_contextual_leaves(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let frame = ui.ctx().cumulative_frame_nr();
    let first = *data.value("first_frame", frame);
    let open: &[usize] = if frame.saturating_sub(first) < TABLE_FRAMES { &WITH_TABLE } else { &PLAIN };
    ribbon(ui, data, TabStrip::new("Ribbon"), open)
}
