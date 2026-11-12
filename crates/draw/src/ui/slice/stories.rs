//! Stories of the Slice tools: the tool run live over a document's slices, the slices in each mode and the bar in
//! each mode.

use super::{
    SliceBar, SliceCommand, SliceEvent, SliceItem, SliceMode, SliceOrigin, SliceOverlay, SliceRules, SliceTool, SliceToolSettings, SliceToolWords,
};
use craft_behavior::geometry::Constraint;
use craft_behavior::kurbo::Rect;
use craft_behavior::{CanvasView, EditEvent, PixelSize};
use craft_ui::bars::{Bar, BarKind};
use craft_ui::canvas::{Canvas, RasterSource as _};
use craft_ui::contract::{Entry, Show as _, ShowContainer as _};
use craft_ui::runtime::env::{Env, UnitKind};
use craft_ui::testing::canvas_story::StoryDocument;
use craft_ui::testing::{Drive, EditKind, StoryButton, StoryData, StoryOutput, ToolStory, story, tool_story};
use egui::Ui;

craft_ui::words! {
    /// The status line of the Slice tool story.
    #[non_exhaustive]
    pub(crate) struct SliceStoryWords {
        /// The status line: the selected slice, then the last command or refusal.
        status: template "sliceStory.status" = "{selection}. {note}";
        /// The selected slice.
        selected: template "sliceStory.selected" = "Selected: slice {number}";
        /// No slice selected.
        none: "sliceStory.none" = "No slice selected";
        /// Nothing done so far.
        idle: "sliceStory.idle" = "Draw, move or resize a slice";
        /// The last command.
        command: template "sliceStory.command" = "Command: {command}";
    }
}

/// A document of 240 by 160 pixels at 72 pixels per inch.
const DOCUMENT: PixelSize = PixelSize::new(240, 160);
const PIXELS_PER_INCH: f64 = 72.0;
/// The document the canvas shows under the slices.
const SHOWN: craft_ui::testing::canvas_story::StoryDocument = craft_ui::testing::canvas_story::StoryDocument::new(DOCUMENT.width, DOCUMENT.height);

/// Two user slices, a layer slice and the auto slice between them.
fn slices() -> Vec<SliceItem> {
    vec![
        SliceItem::new(1, Some(1), Rect::new(0.0, 0.0, 120.0, 80.0), SliceOrigin::User),
        SliceItem::new(2, Some(2), Rect::new(120.0, 80.0, 240.0, 160.0), SliceOrigin::Layer),
        SliceItem::new(3, None, Rect::new(120.0, 0.0, 240.0, 80.0), SliceOrigin::Auto),
        SliceItem::new(4, Some(4), Rect::new(24.0, 96.0, 96.0, 144.0), SliceOrigin::User),
    ]
}

fn on_canvas(ui: &mut Ui, configure: impl for<'a> FnOnce(SliceOverlay<'a>) -> SliceOverlay<'a>) -> StoryOutput {
    let items = slices();
    let mut configure = Some(configure);
    let stage = craft_ui::testing::stage(ui);
    let _ = Canvas::new("Document").view(SHOWN.view(stage, ui.pixels_per_point(), 100.0)).show_at(ui, stage, |ui, body| {
        craft_ui::testing::canvas_story::show_document(ui, body.rect, &SHOWN);
        configure.take().map(|configure| configure(SliceOverlay::new("Slices", &items)).show_at(ui, body.rect))
    });
    StoryOutput::default()
}

#[story(SliceOverlay<'static>, covers = [Default])]
fn slice_mode(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    on_canvas(ui, |overlay| overlay)
}

#[story(SliceOverlay<'static>, covers = [Default])]
fn select_mode(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    on_canvas(ui, |overlay| overlay.pick(true).selected_slice(4))
}

#[story(SliceOverlay<'static>, covers = [])]
fn locked_slices(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    on_canvas(ui, |overlay| overlay.pick(true).selected_slice(1).locked(true))
}

fn bar(ui: &mut Ui, data: &mut StoryData, mode: SliceMode, style: Constraint) -> StoryOutput {
    let settings = data.value("settings", SliceToolSettings::new(style));
    let units = [Entry::new(UnitKind::Pixels, "px"), Entry::new(UnitKind::Inches, "in")];
    Bar::new("Options", BarKind::Options)
        .show(ui, |ui, _| {
            SliceBar::new(mode, settings).size_units(UnitKind::Pixels, &units, PIXELS_PER_INCH, DOCUMENT).show(ui);
        })
        .output
        .into()
}

#[story(SliceBar<'static>, covers = [Default], drive = Drive::Never)]
fn slice_bar_slice_mode(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    bar(ui, data, SliceMode::Slice, Constraint::Free)
}

#[story(SliceBar<'static>, covers = [], drive = Drive::Never)]
fn slice_bar_fixed_size(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    bar(ui, data, SliceMode::Slice, Constraint::Size(64.0, 64.0))
}

#[story(SliceBar<'static>, covers = [], drive = Drive::Never)]
fn slice_bar_select_mode(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    bar(ui, data, SliceMode::Select, Constraint::Free)
}

/// The slices of the live story's document: a demo layout of user, auto and layer slices on the story document.
fn sample_slices() -> Vec<SliceItem> {
    let page = StoryDocument::STANDARD.size();
    let (right, bottom) = (f64::from(page.width), f64::from(page.height));
    let item = |number, origin, rect| SliceItem::new(number, (origin != SliceOrigin::Auto).then_some(number as u32), rect, origin);
    vec![
        item(1, SliceOrigin::Auto, Rect::new(0.0, 0.0, right, 60.0)),
        item(2, SliceOrigin::Auto, Rect::new(0.0, 60.0, 80.0, bottom)),
        item(3, SliceOrigin::User, Rect::new(80.0, 60.0, 300.0, 200.0)),
        item(4, SliceOrigin::Auto, Rect::new(300.0, 60.0, right, 120.0)),
        item(5, SliceOrigin::Layer, Rect::new(360.0, 120.0, 560.0, 300.0)),
        item(6, SliceOrigin::Auto, Rect::new(560.0, 120.0, right, 300.0)),
        item(7, SliceOrigin::Auto, Rect::new(80.0, 200.0, 360.0, bottom)),
        item(8, SliceOrigin::Auto, Rect::new(360.0, 300.0, right, bottom)),
    ]
}

/// The slices and the selection the Slice tool story keeps as its document.
#[derive(Clone, Debug, PartialEq)]
struct Slices {
    items: Vec<SliceItem>,
    selected: Option<usize>,
    hide_auto: bool,
}

impl Slices {
    fn next_number(&self) -> usize {
        self.items.iter().map(|item| item.number).max().unwrap_or(0) + 1
    }

    /// Sets a slice's rectangle; a moved auto or layer slice becomes a user slice.
    fn place(&mut self, number: usize, rect: Rect) {
        if let Some(item) = self.items.iter_mut().find(|item| item.number == number) {
            item.rect = rect;
            item.origin = SliceOrigin::User;
        }
    }

    /// Answers one event as the app does, and returns the status note it leaves.
    fn apply(&mut self, event: SliceEvent, words: &SliceStoryWords, tool_words: &SliceToolWords) -> Option<String> {
        match event {
            SliceEvent::Draw(event) => {
                let rect = done(&event)?;
                let number = self.next_number();
                self.items.push(SliceItem::new(number, Some(number as u32), rect, SliceOrigin::User));
                self.selected = Some(number);
            }
            SliceEvent::Move { number, event, .. } | SliceEvent::Resize { number, event, .. } => self.place(number, done(&event)?),
            SliceEvent::Select { number, .. } => self.selected = number,
            SliceEvent::Command(command) => {
                match command {
                    SliceCommand::Delete => {
                        let selected = self.selected.take();
                        self.items.retain(|item| item.origin != SliceOrigin::User || Some(item.number) != selected);
                    }
                    SliceCommand::Promote => {
                        if let Some(item) = self.items.iter_mut().find(|item| Some(item.number) == self.selected) {
                            item.origin = SliceOrigin::User;
                        }
                    }
                    _ => {}
                }
                return Some(craft_i18n::fmt(words.command.as_str(), &[("command", command_name(command))]));
            }
            SliceEvent::Refused(_) => return Some(tool_words.locked.as_str().to_owned()),
        }
        None
    }
}

/// The value an edit ends on, once it commits.
fn done(event: &EditEvent<Rect>) -> Option<Rect> {
    match event {
        EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => Some(*after),
        _ => None,
    }
}

/// The kind of the edit a slice event carries.
fn edit_kind(event: &SliceEvent) -> Option<EditKind> {
    match event {
        SliceEvent::Draw(event) | SliceEvent::Move { event, .. } | SliceEvent::Resize { event, .. } => EditKind::of(event),
        _ => None,
    }
}

/// The kept Slice tool and the status note of its last command or refusal.
#[derive(Default)]
struct SliceKept {
    tool: SliceTool,
    note: Option<String>,
}

/// The slice commands by their menu names; the first four are story buttons.
const COMMANDS: [(&str, SliceCommand); 5] = [
    ("Promote", SliceCommand::Promote),
    ("Delete", SliceCommand::Delete),
    ("Slice Options…", SliceCommand::Options),
    ("Divide…", SliceCommand::Divide),
    ("Slices From Guides", SliceCommand::FromGuides),
];
/// The story button that turns Hide Auto Slices on and off.
const HIDE_AUTO: &str = "Hide Auto Slices";

fn command_name(command: SliceCommand) -> &'static str {
    COMMANDS.iter().find(|(_, listed)| *listed == command).map_or("", |(name, _)| *name)
}

/// The Slice and Slice Select tools live over a document's user, auto and layer slices. Slice mode draws a slice;
/// Select mode moves and resizes a user slice (an auto or layer slice that moves becomes a user slice). Promote,
/// Delete and Hide Auto Slices act; Slice Options and Divide only report. The lock refuses every edit.
#[story(SliceOverlay<'static>, covers = [], drive = Drive::Never)]
fn slice_tool(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let env = Env::of(ui);
    let (words, tool_words) = (env.words::<SliceStoryWords>(), env.words::<SliceToolWords>());
    let mode = *data.choice("mode", SliceMode::Slice, &[("Slice", SliceMode::Slice), ("Select", SliceMode::Select)]);
    let rules = *data.choice("rules", SliceRules::PHOTOSHOP, &[("Photoshop", SliceRules::PHOTOSHOP), ("Illustrator", SliceRules::ILLUSTRATOR)]);
    let locked = *data.value("locked", false);
    let settings = SliceToolSettings::new(Constraint::Free);
    let buttons: Vec<StoryButton> = COMMANDS[..4].iter().map(|(label, _)| *label).chain([HIDE_AUTO]).map(StoryButton::new).collect();
    let body = |ui: &mut Ui, at: egui::Rect, _: &CanvasView, _: &mut SliceKept, slices: &Slices| {
        let overlay = SliceOverlay::new("Slices", &slices.items);
        let overlay = match slices.selected {
            Some(number) => overlay.selected_slice(number),
            None => overlay,
        };
        let overlay = overlay.pick(mode == SliceMode::Select).hide_auto(slices.hide_auto).locked(locked).rules(rules);
        overlay.show_at(ui, at).events
    };
    let first = Slices { items: sample_slices(), selected: None, hide_auto: false };
    let picture = StoryDocument::STANDARD;
    tool_story(ui, data, ToolStory::new("Document", &picture).buttons(&buttons), SliceKept::default, first, body, |run| {
        let (slices, kept) = (run.document, run.tool);
        let mut tool_settings = settings;
        tool_settings.hide_auto = slices.hide_auto;
        kept.tool.mode(mode).items(&slices.items).selected(slices.selected).locked(locked).rules(rules).settings(tool_settings);
        let _ = run.host.run(&mut kept.tool);
        let mut events = run.shown.unwrap_or_default();
        events.extend(kept.tool.take_events());
        let mut output = StoryOutput::default();
        output.edits = events.iter().filter_map(edit_kind).collect();
        output.clicked = run.pressed.is_some();
        if run.pressed == Some(HIDE_AUTO) {
            slices.hide_auto = !slices.hide_auto;
        } else if let Some((_, command)) = run.pressed.and_then(|label| COMMANDS.iter().find(|(name, _)| *name == label)) {
            events.push(SliceEvent::Command(*command));
        }
        for event in events {
            if let Some(note) = slices.apply(event, &words, &tool_words) {
                kept.note = Some(note);
            }
        }
        let selection = match slices.selected {
            Some(number) => craft_i18n::fmt(words.selected.as_str(), &[("number", number.to_string().as_str())]),
            None => words.none.as_str().to_owned(),
        };
        let note = kept.note.clone().unwrap_or_else(|| words.idle.as_str().to_owned());
        output.readout(craft_i18n::fmt(words.status.as_str(), &[("selection", selection.as_str()), ("note", note.as_str())]))
    })
}
