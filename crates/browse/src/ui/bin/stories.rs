//! Stories of the media bin: the List view with nested bins, the Icon view with bins first, badges and an In and
//! Out bar, and the Freeform view with placed and unplaced cards.
//!
//! The pictures come from a `FakeThumbSource`; nothing decodes, spawns a thread or polls on a timer.

use craft_behavior::{ColumnKey, ColumnWidth, IconId, Selection};
use craft_theme::Srgba;
use craft_theme::variant::Tone;
use craft_ui::contract::{Bind, ColumnSpec};
use craft_ui::primitives::textures::{FakeThumbSource, ThumbnailOptions, Thumbnails};
use craft_ui::sections::FooterAction;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use craft_ui::tile_grid::{BadgeKind, BadgeSlot, TileBadge, TileStatus};
use egui::{Ui, Vec2};

use super::{BinEntry, BinItem, BinModel, BinState, BinView, MediaBin};
use crate::ui::freebin::FreeformCard;

/// Where the story places its cards, in points.
const NEXT: f32 = 160.0;
const DOWN: f32 = 48.0;

const DURATION: ColumnKey = ColumnKey::new("duration");

/// One entry of the story's project.
struct Node {
    key: u32,
    name: &'static str,
    parent: Option<u32>,
    bin: bool,
    icon: &'static str,
    duration: &'static str,
}

/// The story's project: two bins, a nested one, and items.
struct Project(Vec<Node>);

impl Project {
    fn sample() -> Self {
        let bin = |key, name, parent| Node { key, name, parent, bin: true, icon: "folder", duration: "" };
        let item = |key, name, parent, icon, duration| Node { key, name, parent, bin: false, icon, duration };
        Project(vec![
            bin(1, "Footage", None),
            bin(2, "Archive", None),
            bin(3, "Interviews", Some(1)),
            item(10, "Beach.mov", Some(1), "image", "00:12:04"),
            item(11, "Clip.mov", Some(1), "image", "00:03:10"),
            item(12, "Narration.wav", Some(1), "file", "00:45:00"),
            item(13, "Wide shot.mov", Some(1), "image", "00:01:45"),
            item(14, "Drone.mov", Some(1), "image", "00:08:20"),
            item(15, "Host.wav", Some(3), "file", "00:22:10"),
            item(20, "Old beach.mov", Some(2), "image", "00:09:30"),
            item(30, "Title.psd", None, "image", ""),
            item(31, "Rough cut", None, "play", "00:04:00"),
        ])
    }

    fn node(&self, key: u32) -> Option<&Node> {
        self.0.iter().find(|node| node.key == key)
    }
}

impl BinModel<u32> for Project {
    fn children(&self, parent: Option<&u32>, out: &mut Vec<BinEntry<u32>>) {
        for node in self.0.iter().filter(|node| node.parent == parent.copied()) {
            out.push(if node.bin { BinEntry::Bin(node.key) } else { BinEntry::Item(node.key) });
        }
    }

    fn item(&self, key: &u32, item: &mut BinItem) {
        let Some(node) = self.node(*key) else { return };
        item.name = node.name.to_owned();
        item.icon = Some(IconId::new(node.icon));
        item.duration = (!node.duration.is_empty()).then(|| node.duration.to_owned());
        match node.key {
            10 => {
                item.range = Some((0.2, 0.7));
                item.badges.push(TileBadge::new(BadgeSlot::EndTop, BadgeKind::Glyph(IconId::new("link")), Tone::Accent, Some("Linked".into())));
            }
            11 => item.label_color = Some(Srgba::rgb(0.82, 0.42, 0.23)),
            13 => item.status = TileStatus::Offline(None),
            14 => item.status = TileStatus::Failed,
            _ => {}
        }
    }

    fn cell(&self, key: &u32, column: &ColumnKey) -> String {
        if *column == DURATION { self.node(*key).map(|node| node.duration.to_owned()).unwrap_or_default() } else { String::new() }
    }

    fn is_bin(&self, key: &u32) -> bool {
        self.node(*key).is_some_and(|node| node.bin)
    }

    fn parent(&self, key: &u32) -> Option<u32> {
        self.node(*key).and_then(|node| node.parent)
    }

    fn count(&self, bin: &u32) -> usize {
        self.0.iter().filter(|node| node.parent == Some(*bin)).map(|node| if node.bin { self.count(&node.key) } else { 1 }).sum()
    }
}

fn columns() -> Vec<ColumnSpec> {
    vec![
        ColumnSpec::new(ColumnKey::new("name"), "Name", ColumnWidth::fill(2.0)).sortable().resizable(),
        ColumnSpec::new(DURATION, "Duration", ColumnWidth::Controls(4.0)).sortable().resizable(),
    ]
}

/// The bin in `view`, keeping its state and selection in the story's data.
fn show_bin(ui: &mut Ui, data: &mut StoryData, view: BinView, cards: &[FreeformCard<u32>], selected: &[u32]) -> StoryOutput {
    let key = match view {
        BinView::List => "list",
        BinView::Icon => "icon",
        BinView::Freeform => "freeform",
    };
    let mut state = data
        .value(key, {
            let mut state = BinState::<u32>::default();
            state.settings.view = view;
            state.expanded.set(&1, true);
            state
        })
        .clone();
    let mut selection = Selection::new(selected.to_vec(), selected.last().copied(), selected.last().copied());
    selection = data.value("selection", selection).clone();
    let project = Project::sample();
    let thumbs = Thumbnails::<u32>::in_context(ui.ctx(), "craft_browse::bin::stories", ThumbnailOptions::new());
    let source = FakeThumbSource::<u32>::new().aspect(16, 9);
    let out = MediaBin::new("Project", Bind::bound(&mut state), &project)
        .selection(Bind::bound(&mut selection))
        .columns(columns())
        .thumbnails(&thumbs, &source)
        .hover_scrub(true)
        .freeform(cards)
        .footer_actions([FooterAction::new(IconId::new("plus"), "New Item")])
        .show(ui);
    *data.value(key, BinState::<u32>::default()) = state;
    *data.value("selection", Selection::<u32>::default()) = selection;
    out.into()
}

#[story(MediaBin<'static, u32, dyn BinModel<u32>>, covers = [], drive = Drive::Never)]
fn media_bin_list(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_bin(ui, data, BinView::List, &[], &[11])
}

#[story(MediaBin<'static, u32, dyn BinModel<u32>>, covers = [], drive = Drive::Never)]
fn media_bin_icon(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_bin(ui, data, BinView::Icon, &[], &[])
}

#[story(MediaBin<'static, u32, dyn BinModel<u32>>, covers = [], drive = Drive::Never)]
fn media_bin_freeform(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    // Two of the root's four entries are placed; the others flow.
    let cards = [FreeformCard { key: 30, pos: Vec2::ZERO, stack: None }, FreeformCard { key: 31, pos: Vec2::X * NEXT + Vec2::Y * DOWN, stack: None }];
    show_bin(ui, data, BinView::Freeform, &cards, &[30])
}
