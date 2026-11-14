//! Stories of the Artboard tool, run live over three boards on the pasteboard, and of the boards, the bar and the
//! Properties section.

use super::{
    ArtboardBackground, ArtboardBar, ArtboardEvent, ArtboardItem, ArtboardOverlay, ArtboardPreset, ArtboardProperties, ArtboardRules, ArtboardTool,
};
use crate::ui::add_beside::{AddBesideHandles, AddSide};
use craft_behavior::kurbo::{Point, Rect, Size, Vec2};
use craft_behavior::{CanvasView, EditEvent};
use craft_theme::Srgba;
use craft_ui::bars::{Bar, BarKind};
use craft_ui::canvas::{Canvas, PageFrame, Pasteboard};
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::runtime::env::Env;
use craft_ui::style::DataColor;
use craft_ui::testing::canvas_story::StoryPicture;
use craft_ui::testing::{Drive, EditKind, StoryData, StoryOutput, ToolStory, story, tool_story};
use egui::{Color32, Ui};

craft_ui::words! {
    /// The board names and the status line of the Artboard tool story.
    #[non_exhaustive]
    pub(crate) struct ArtboardStoryWords {
        /// A board the story adds: `{number}` from 1.
        board: template "artboardStory.board" = "Artboard {number}";
        /// The status line: the selected boards, then the last result.
        status: template "artboardStory.status" = "{selection}. {note}";
        /// The selected boards' names, joined.
        selected: template "artboardStory.selected" = "Selected: {names}";
        /// No board selected.
        none: "artboardStory.none" = "No artboard selected";
        /// Nothing done so far.
        idle: "artboardStory.idle" = "Draw, move or resize a board";
        /// A board was added.
        added: template "artboardStory.added" = "Added {name}";
        /// Artboard Options were asked for.
        opened: template "artboardStory.opened" = "Options of {name}";
        /// Illustrator: fit a board to the object clicked.
        fit: template "artboardStory.fit" = "Fit to the object at {x}, {y}";
        /// Illustrator: Esc left the tool.
        left: "artboardStory.left" = "Left the Artboard tool";
        /// The bar story's board size.
        size: template "artboardStory.size" = "Size {width} × {height}";
    }
}

/// A board's size and the gap the "+" buttons leave (story sample data, in document pixels).
const BOARD: Size = Size::new(180.0, 240.0);
const GAP: f64 = 60.0;

fn presets() -> Vec<ArtboardPreset> {
    vec![ArtboardPreset::new("Letter", Size::new(612.0, 792.0)), ArtboardPreset::new("Phone", Size::new(390.0, 844.0))]
}

fn boards() -> Vec<ArtboardItem> {
    vec![ArtboardItem::new("Artboard 1", Rect::new(0.0, 0.0, 180.0, 240.0)), ArtboardItem::new("Artboard 2", Rect::new(240.0, 0.0, 420.0, 240.0))]
}

/// Two boards on the pasteboard, the first selected.
#[story(ArtboardOverlay<'static>, covers = [Default])]
fn two_artboards(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let shown = Boards { items: boards(), selected: vec![0] };
    let (items, selected, pages) = (&shown.items, &shown.selected, shown.pages());
    let stage = craft_ui::testing::stage(ui);
    let bounds = items.iter().fold(items[0].rect, |bounds, item| bounds.union(item.rect));
    let view = craft_ui::testing::canvas_story::story_view(stage, ui.pixels_per_point(), bounds.center(), 100.0);
    let shown = Canvas::new("Document").view(view).show_at(ui, stage, |ui, body| {
        craft_ui::testing::canvas_story::show_document(ui, body.rect, &craft_ui::testing::canvas_story::StoryDocument::STANDARD);
        let named = Pasteboard::new("Pasteboard", &pages).show_at(ui, body.rect).name_clicked.is_some();
        let _ = ArtboardOverlay::new("Artboards", items, selected).show_at(ui, body.rect);
        named
    });
    // A board's name is a button: its click is a click of the story.
    StoryOutput { clicked: shown.inner.unwrap_or_default(), ..StoryOutput::default() }
}

/// The bar for one board, whose size the story keeps: a preset, Make Portrait or Landscape, or W and H resize it.
#[story(ArtboardBar<'static>, covers = [Default], drive = Drive::Never)]
fn bar_for_a_landscape_board(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let env = Env::of(ui);
    let (words, units) = (env.words::<ArtboardStoryWords>(), env.units());
    let size = data.value("size", Size::new(240.0, 180.0));
    let board = ArtboardItem::new("Artboard 1", Rect::from_origin_size(Point::ORIGIN, *size));
    let presets = presets();
    let mut members = None;
    let bar = Bar::new("Options", BarKind::Options).show(ui, |ui, _| members = Some(ArtboardBar::new(Some(&board), &presets).show(ui)));
    let mut output = StoryOutput::from(bar.output);
    if let Some(members) = members {
        output.edits = members.size.iter().filter_map(EditKind::of).collect();
        if let Some(landed) = members.size.landed() {
            *size = *landed;
        }
    }
    let number = |value: f64| units.number(value, 0..=0);
    output.readout(craft_i18n::fmt(words.size.as_str(), &[("width", number(size.width).as_str()), ("height", number(size.height).as_str())]))
}

#[story(ArtboardProperties<'static>, covers = [Default], drive = Drive::Never)]
fn properties_with_a_custom_background(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut board = ArtboardItem::new("Artboard 1", Rect::new(0.0, 0.0, 390.0, 844.0));
    board.background = ArtboardBackground::Custom(Srgba::rgb(0.9, 0.4, 0.2));
    let presets = presets();
    ArtboardProperties::new(&board, &presets).show(ui);
    StoryOutput::default()
}

/// The boards and the selection the Artboard tool story keeps as its document.
#[derive(Clone, Debug, PartialEq)]
struct Boards {
    items: Vec<ArtboardItem>,
    selected: Vec<usize>,
}

impl Boards {
    /// Three boards side by side, the first selected.
    fn new(words: &ArtboardStoryWords) -> Self {
        let items = (1..=3)
            .map(|number| {
                let x = (number - 1) as f64 * (BOARD.width + GAP);
                ArtboardItem::new(board_name(words, number), Rect::new(x, 0.0, x + BOARD.width, BOARD.height))
            })
            .collect();
        Boards { items, selected: vec![0] }
    }

    fn pages(&self) -> Vec<PageFrame> {
        let page = |(index, item): (usize, &ArtboardItem)| {
            let page = PageFrame::new(item.rect.abs()).name(item.name.clone()).active(self.selected.contains(&index));
            match item.background {
                ArtboardBackground::White => page.fill(DataColor(Color32::WHITE)), // design: allow[raw-colour/const](the artboard background is document data)
                ArtboardBackground::Black => page.fill(DataColor(Color32::BLACK)), // design: allow[raw-colour/const](the artboard background is document data)
                ArtboardBackground::Custom(color) => page.fill(DataColor(Color32::from(color))),
                _ => page,
            }
        };
        self.items.iter().enumerate().map(page).collect()
    }

    /// The board the "+" buttons surround: the active one, unless a drag runs.
    fn active(&self) -> Option<Rect> {
        self.selected.first().and_then(|&index| self.items.get(index)).map(|item| item.rect.abs())
    }

    /// Adds a board named after its place, at `rect`, and selects it; returns its name.
    fn push(&mut self, rect: Rect, words: &ArtboardStoryWords) -> String {
        let name = board_name(words, self.items.len() + 1);
        self.items.push(ArtboardItem::new(name.clone(), rect));
        self.selected = vec![self.items.len() - 1];
        name
    }

    /// Answers one tool event as the app does, and returns the status note it leaves.
    fn apply(&mut self, event: ArtboardEvent, words: &ArtboardStoryWords) -> Option<String> {
        let added = |name: String| craft_i18n::fmt(words.added.as_str(), &[("name", name.as_str())]);
        match event {
            ArtboardEvent::Draw(event) => return done(&event).map(|rect| added(self.push(rect.abs(), words))),
            ArtboardEvent::Move { indices, event, copy, .. } => {
                let by = done(&event)?;
                let mut note = None;
                for index in indices {
                    let Some(from) = self.items.get(index).map(|item| item.rect) else { continue };
                    if copy {
                        note = Some(added(self.push(from + by, words)));
                    } else {
                        self.items[index].rect = from + by;
                    }
                }
                return note;
            }
            ArtboardEvent::Resize { index, event, .. } => {
                if let (Some(rect), Some(item)) = (done(&event), self.items.get_mut(index)) {
                    item.rect = rect.abs();
                }
            }
            ArtboardEvent::AddBeside { index, side, .. } => {
                let from = self.items.get(index)?.rect.abs();
                let by = match side {
                    AddSide::Top => Vec2::new(0.0, -(from.height() + GAP)),
                    AddSide::Right => Vec2::new(from.width() + GAP, 0.0),
                    AddSide::Bottom => Vec2::new(0.0, from.height() + GAP),
                    _ => Vec2::new(-(from.width() + GAP), 0.0),
                };
                return Some(added(self.push(from + by, words)));
            }
            ArtboardEvent::Select { indices, .. } => self.selected = indices,
            ArtboardEvent::Delete { mut indices, .. } => {
                indices.sort_unstable_by(|a, b| b.cmp(a));
                for index in indices {
                    if index < self.items.len() {
                        self.items.remove(index);
                    }
                }
                self.selected.clear();
            }
            ArtboardEvent::OpenOptions { index, .. } => {
                let name = self.items.get(index)?.name.as_str();
                return Some(craft_i18n::fmt(words.opened.as_str(), &[("name", name)]));
            }
            ArtboardEvent::FitObject { at, .. } => {
                let (x, y) = (format!("{:.0}", at.x), format!("{:.0}", at.y));
                return Some(craft_i18n::fmt(words.fit.as_str(), &[("x", x.as_str()), ("y", y.as_str())]));
            }
            ArtboardEvent::Leave => {
                self.selected.clear();
                return Some(words.left.as_str().to_owned());
            }
        }
        None
    }
}

fn board_name(words: &ArtboardStoryWords, number: usize) -> String {
    craft_i18n::fmt(words.board.as_str(), &[("number", number.to_string().as_str())])
}

/// The value an edit ends on, once it commits a change.
fn done<T: Copy>(event: &EditEvent<T>) -> Option<T> {
    match event {
        EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => Some(*after),
        _ => None,
    }
}

/// The kind of the edit a board event carries.
fn edit_kind(event: &ArtboardEvent) -> Option<EditKind> {
    match event {
        ArtboardEvent::Draw(event) | ArtboardEvent::Resize { event, .. } => EditKind::of(event),
        ArtboardEvent::Move { event, .. } => EditKind::of(event),
        _ => None,
    }
}

/// The kept Artboard tool and the status note of its last result.
#[derive(Default)]
struct ArtboardKept {
    tool: ArtboardTool,
    note: Option<String>,
}

/// The Artboard tool live over three boards on the pasteboard, the third locked by a story toggle. Photoshop's rules
/// show the "+" buttons round the active board; Illustrator's select several boards, copy with Alt, open Options on a
/// double click and leave with Esc. A drag on the pasteboard draws a board, a board's name selects it.
#[story(ArtboardOverlay<'static>, covers = [], drive = Drive::Never)]
fn artboard_tool(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<ArtboardStoryWords>();
    let rules =
        *data.choice("rules", ArtboardRules::PHOTOSHOP, &[("Photoshop", ArtboardRules::PHOTOSHOP), ("Illustrator", ArtboardRules::ILLUSTRATOR)]);
    let locked = *data.value("locked board", true);
    let boards = data.value("document", Boards::new(&words));
    if let Some(third) = boards.items.get_mut(2) {
        third.locked = locked;
    }
    let first = Boards::new(&words);
    let bounds = first.items.iter().map(|item| item.rect).reduce(|a, b| a.union(b)).unwrap_or_default();
    // The pasteboard covers all but the boards: the picture is only the document's extent.
    let picture = StoryPicture::new(bounds.width() as u32, bounds.height() as u32, |_: Point| Color32::TRANSPARENT);
    let body = |ui: &mut Ui, at: egui::Rect, _: &CanvasView, _: &mut ArtboardKept, boards: &Boards| {
        let pages = boards.pages();
        let named = Pasteboard::new("Pasteboard", &pages).show_at(ui, at).name_clicked;
        let overlay = ArtboardOverlay::new("Artboards", &boards.items, &boards.selected).rules(rules).show_at(ui, at);
        let plus = boards.active().filter(|_| rules.plus_buttons && !overlay.dragging);
        let added = plus.and_then(|board| {
            let shown = AddBesideHandles::new("Add artboard", board).show_at(ui, at);
            shown.added.map(|side| (side, shown.copy))
        });
        (named, overlay.events, added)
    };
    let story = ToolStory::new("Document", &picture).view(bounds.center(), 75.0);
    tool_story(ui, data, story, ArtboardKept::default, first, body, |run| {
        let (boards, kept) = (run.document, run.tool);
        let (named, mut events, added) = run.shown.unwrap_or_default();
        kept.tool.items(&boards.items).selected(&boards.selected).rules(rules);
        if let Some((side, copy)) = added {
            kept.tool.add_beside(side, copy);
        }
        let _ = run.host.run(&mut kept.tool);
        events.extend(kept.tool.take_events());
        let edits: Vec<EditKind> = events.iter().filter_map(edit_kind).collect();
        for event in events {
            if let Some(note) = boards.apply(event, &words) {
                kept.note = Some(note);
            }
        }
        if let Some(index) = named {
            boards.selected = vec![index];
        }
        let names: Vec<&str> = boards.selected.iter().filter_map(|&index| boards.items.get(index)).map(|item| item.name.as_str()).collect();
        let selection = if names.is_empty() {
            words.none.as_str().to_owned()
        } else {
            craft_i18n::fmt(words.selected.as_str(), &[("names", names.join(", ").as_str())])
        };
        let note = kept.note.clone().unwrap_or_else(|| words.idle.as_str().to_owned());
        let mut output =
            StoryOutput::default().readout(craft_i18n::fmt(words.status.as_str(), &[("selection", selection.as_str()), ("note", note.as_str())]));
        // A board's name and the "+" buttons are buttons: their clicks are the story's.
        output.clicked = named.is_some() || added.is_some();
        output.edits = edits;
        output
    })
}
