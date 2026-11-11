//! One card: stories live on their surface under a header with the component's name and, while a story differs from
//! its start, Reset; the Open command of an Overview card opens the component's tab. A card of several stories lays
//! them out as the component's catalog entry says, each under its caption and over its readout. A story runs in the
//! shell of the view the card is in, so its dialogs open over that view, as an app's do. [`StoryBody`] is the story
//! part alone, which the story renderer (`story-render`) shows too.

use super::inspect::{Inspected, region_key};
use crate::viewer::{CardState, Cells, DialogState};
use craft_behavior::{ColumnWidth, IconId};
use craft_i18n::Direction;
use craft_theme::{ControlSize, Mode};
use craft_ui::buttons::Button;
use craft_ui::contract::Show as _;
use craft_ui::contract::{CommandSet, Commands as _, Gap, Pad, Props as _, ShowContainer as _, Sizing, StoryLayout, Surface, ThemeScope};
use craft_ui::display::{InspectLayers, InspectOverlay, Shade, Text};
use craft_ui::layout_components::{Grid, Stack};
use craft_ui::runtime::env::with_env;
use craft_ui::runtime::inspect::{self, Recorded};
use craft_ui::runtime::layout_memory;
use craft_ui::sections::{HeaderCommand, HeaderStrength, Panel};
use craft_ui::style::SurfaceKind;
use craft_ui::testing::{StoryData, StoryOutput, StoryRegistration};

/// A story's show function.
type StoryFn = fn(&mut Ui, &mut StoryData) -> StoryOutput;
use egui::{Id, Rect, Ui};
use std::sync::Arc;

/// The header commands of a card.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "galleryCard", scope = "gallery.card")]
#[non_exhaustive]
pub enum CardCommands {
    /// Open the component's tab.
    #[command(label = "Open", icon = "open")]
    Open,
    /// Give the story its start values.
    #[command(label = "Reset", icon = "undo")]
    Reset,
    /// Use the card's value, in a Variants view.
    #[command(label = "Use", icon = "check")]
    Use,
}

/// The appearance values a card sets for its story beyond the window's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Look {
    /// The mode the story's tokens resolve in.
    pub mode: Option<Mode>,
    /// The layout direction.
    pub direction: Option<Direction>,
    /// The control size.
    pub size: Option<ControlSize>,
}

/// One story a card shows.
#[non_exhaustive]
pub struct CardStory<'a> {
    /// The index of the story in the catalog: the key of its state.
    pub index: usize,
    /// The story.
    pub story: &'static StoryRegistration,
    /// The story's caption.
    pub caption: &'a str,
}

impl<'a> CardStory<'a> {
    /// The story at catalog index `index`, under `caption`.
    pub fn new(index: usize, story: &'static StoryRegistration, caption: &'a str) -> Self {
        CardStory { index, story, caption }
    }
}

/// What one card shows.
#[non_exhaustive]
pub struct Card<'a> {
    /// The id scope of the card: unique among the cards of a pass.
    pub key: String,
    /// The component's display name.
    pub title: &'a str,
    /// A note after the title.
    pub note: Option<&'a str>,
    /// The stories, in the order they show. A card of more than one shows each under its caption.
    pub stories: &'a [CardStory<'a>],
    /// How the stories are laid out together.
    pub layout: StoryLayout,
    /// The surface the stories sit on.
    pub surface: Surface,
    /// The most a story that fills takes, at its content height below that: the visible height of the view the cards
    /// scroll in, as the old gallery's tiles gave it.
    pub room: f32,
    /// Whether the header has Open.
    pub open: bool,
    /// Whether the header has Use.
    pub usable: bool,
    /// Whether the card has the selection ring.
    pub selected: bool,
    /// The values the card sets for its stories.
    pub look: Look,
    /// Whether the stories' wheel takers take the wheel only while focused or armed: a card among others in a
    /// scrolling overview.
    pub gated: bool,
    /// How the card's stories are inspected, while the inspector is on and the card is the selected one.
    pub inspect: Option<Inspected>,
}

/// What a card reported in a pass.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct CardEvents {
    /// A press went down in the card.
    pub pressed: bool,
    /// Open was run.
    pub opened: bool,
    /// Reset was run.
    pub reset: bool,
    /// Use was run.
    pub used: bool,
    /// The card's rect: where the story renderer crops it.
    pub rect: Option<Rect>,
}

/// The narrowest a column of stories may be, as the old gallery's story grids kept their cases.
const STORY_MIN_WIDTH: f32 = 240.0;

/// The column lists of story grids, by count.
static COLUMNS: [ColumnWidth; 4] = [ColumnWidth::fill(1.0); 4];

/// The columns of a form-rows card: the captions, the stories, then the values they show.
const FORM_ROWS: &[ColumnWidth] = &[ColumnWidth::Content, ColumnWidth::fill(1.0), ColumnWidth::Content];

impl Card<'_> {
    /// Shows the card in the room `ui` offers, with the states of its stories.
    pub fn show(&self, ui: &mut Ui, cells: &mut Cells) -> CardEvents {
        let mut events = CardEvents::default();
        // A story on the card's own surface sits inside the card's padding; any other surface runs edge to edge under the
        // header band.
        let pad = if self.surface == Surface::Panel { Pad::Regular } else { Pad::None };
        // Its id comes from its key, not from its place: a change of the column count regroups the cards, and each
        // keeps its state and its stories' ids.
        let mut panel = Panel::new(self.title.to_owned())
            .id(craft_ui::testing::memory::global("gallery.card", &self.key))
            .sizing(Sizing::Fill)
            .command_scope(CardCommands::SCOPE)
            .selected(self.selected)
            .pickable()
            .framed()
            .band(SurfaceKind::Background)
            .strength(HeaderStrength::Heading)
            .pad(pad);
        if let Some(note) = self.note {
            panel = panel.subtitle(format!("· {note}"));
        }
        let mut commands = Vec::new();
        if self.open {
            commands.push(HeaderCommand::new(IconId::new("open"), CardCommands::Open).size(ControlSize::Sm));
        }
        if self.usable {
            commands.push(HeaderCommand::new(IconId::new("check"), CardCommands::Use).size(ControlSize::Sm));
        }
        let away = self.stories.iter().any(|story| cells.get(&story.index).is_some_and(|state| state.changed() || state.dialog.shown));
        if away {
            commands.push(HeaderCommand::new(IconId::new("undo"), CardCommands::Reset).size(ControlSize::Sm));
        }
        let shown = panel.commands(commands).show(ui, |ui, cx| {
            while let Some((command, _)) = cx.take_command::<CardCommands>() {
                match command {
                    CardCommands::Open => events.opened = true,
                    CardCommands::Reset => events.reset = true,
                    CardCommands::Use => events.used = true,
                }
            }
            events.pressed |= cx.surface_pressed();
            self.stories_in(ui, cells);
        });
        events.rect = Some(shown.response.rect);
        events
    }

    /// The card's stories in its layout; a lone story outside form rows without its caption.
    fn stories_in(&self, ui: &mut Ui, cells: &mut Cells) {
        if let ([story], false) = (self.stories, self.layout == StoryLayout::FormRows) {
            self.body(ui, story, None, cells);
            return;
        }
        match self.layout {
            StoryLayout::FormRows => {
                Grid::new(FORM_ROWS).fill_width().show(ui, |ui, _| {
                    for story in self.stories {
                        Text::new(story.caption.to_owned()).show(ui);
                        let body = self.story_body(story, None);
                        body.show(ui, cells.entry(story.index).or_default());
                        Text::new(body.value_text(ui).unwrap_or_default()).shade(Shade::Muted).show(ui);
                    }
                });
            }
            StoryLayout::Columns(most) => {
                let gap = Gap::Item.points(ui);
                let fit = ((ui.available_width() + gap) / (STORY_MIN_WIDTH + gap)).floor() as usize;
                let columns = fit.clamp(1, usize::from(most).clamp(1, COLUMNS.len()));
                Grid::new(&COLUMNS[..columns]).gap(Gap::Item).fill_width().show(ui, |ui, _| {
                    for story in self.stories {
                        self.body(ui, story, Some(story.caption), cells);
                    }
                });
            }
            _ => {
                Stack::new().fill_width().show(ui, |ui, _| {
                    for story in self.stories {
                        self.body(ui, story, Some(story.caption), cells);
                    }
                });
            }
        }
    }

    /// The body of `story`, with its state, under `caption` when given.
    fn body(&self, ui: &mut Ui, story: &CardStory<'_>, caption: Option<&str>, cells: &mut Cells) -> BodyShown {
        self.story_body(story, caption).show(ui, cells.entry(story.index).or_default())
    }

    /// The body of `story` under `caption` when given.
    fn story_body(&self, story: &CardStory<'_>, caption: Option<&str>) -> StoryBody {
        let key = format!("{}/{}", self.key, story.story.story.name);
        StoryBody {
            look: self.look,
            gated: self.gated,
            caption: caption.map(str::to_owned),
            inspect: self.inspect.map(|inspected| (region_key(inspected.tab, story.index), inspected.layers)),
            ..StoryBody::new(key, story.story, self.surface, self.room, Pad::None)
        }
    }
}

/// A story on its surface, as a card's body shows it, and what showed.
#[non_exhaustive]
pub struct StoryBody {
    /// The id scope of the story: unique among the stories of a pass.
    pub key: String,
    /// The story.
    pub story: &'static StoryRegistration,
    /// The surface the story sits on.
    pub surface: Surface,
    /// The most a story that fills takes.
    pub room: f32,
    /// The values the body sets for its story.
    pub look: Look,
    /// Whether the story's wheel takers take the wheel only while focused or armed.
    pub gated: bool,
    /// The margin of a story on the panel surface: none in a card, whose padding is that margin.
    pub panel_pad: Pad,
    /// The caption over the story, on the story's surface, in the old gallery's caption style.
    pub caption: Option<String>,
    /// The inspected region the story is shown as, and the drawings shown over it.
    pub inspect: Option<(Id, InspectLayers)>,
}

/// What a story body showed.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct BodyShown {
    /// Where the story shows in flow: its surface; `None` while the button that opens the story's dialog shows.
    pub flow: Option<Rect>,
}

impl StoryBody {
    /// The body of `story` on `surface`, in the window's look, with ungated wheel takers.
    pub fn new(key: impl Into<String>, story: &'static StoryRegistration, surface: Surface, room: f32, panel_pad: Pad) -> Self {
        StoryBody { key: key.into(), story, surface, room, look: Look::default(), gated: false, panel_pad, caption: None, inspect: None }
    }

    /// The story on its surface, in the shell around the body: its dialogs open over the shell's view.
    ///
    /// A story that opens a dialog, a modal or a window starts closed behind a button, and nothing of what it opens
    /// paints before the button is pressed. The shell counts what shows as a dialog (`ShellCx::dialogs_shown`), and the
    /// body reads the count before and after its story. A story that has not painted yet runs only in a pass that can
    /// still be discarded, and when the count changes what the card shows, the card discards the pass, so nothing of
    /// that pass paints. The button turns the story's `open` value on and runs it, which opens the real thing (an OS
    /// window where the component uses one). Once nothing shows any more, the exit fade included, the card shows the
    /// button again.
    pub fn show(&self, ui: &mut Ui, state: &mut CardState) -> BodyShown {
        let host = Host {
            caption: self.caption.as_deref(),
            id: craft_ui::testing::memory::global("gallery.card.story", (&self.key, state.resets)),
            room: self.room,
            look: self.look,
            surface: self.surface,
            gated: self.gated,
            panel_pad: self.panel_pad,
        };
        let show = self.story.story.show;
        if state.dialog.hidden {
            state.note_start();
            if Button::new("Show dialog").show(ui).clicks == 0 {
                return BodyShown::default();
            }
            ui.ctx().request_repaint();
            state.show_dialog();
        }
        // A story that has not painted yet runs only in a pass that can still be discarded: what the shell's count finds
        // open would paint otherwise. The next frame's first pass runs it.
        let runs = state.dialog.painted || craft_ui::runtime::frame::discardable(ui.ctx());
        // Whether the story ran for real in this pass (not in a measuring run, nor in place of a kept measurement), and
        // whether a dialog of it showed then.
        let ran = std::cell::Cell::new(None);
        let value_key = self.value_key();
        let data = &mut state.data;
        // The record of the story's real run, when it is inspected.
        let mut recorded = None;
        let surface = host.stack().show(ui, |ui, cx| {
            if !runs {
                return;
            }
            let shell = cx.shell();
            let before = shell.as_ref().map_or(0, |shell| shell.dialogs_shown());
            let (output, record) = host.content(ui, show, data, self.inspect.map(|(key, _)| key));
            recorded = record.or(recorded.take());
            if let Some(readout) = &output.readout {
                Text::new(readout.clone()).shade(Shade::Muted).show(ui);
            }
            if !ui.is_sizing_pass() {
                ran.set(Some(shell.is_some_and(|shell| shell.dialogs_shown() > before)));
            }
            layout_memory::emit_text(ui.ctx(), value_key, output.value_text.as_deref());
        });
        if !runs {
            ui.ctx().request_repaint();
        }
        if let (Some((key, layers)), Some(recorded)) = (self.inspect, recorded) {
            let pointed = inspect::pointed(ui.ctx(), key);
            InspectOverlay::new(recorded).layers(layers).pointed(pointed).show_at(ui, surface.output.rect);
        }
        if let Some(shown) = ran.get() {
            let dialog = &mut state.dialog;
            if shown && !dialog.painted {
                close_at_start(state);
                craft_ui::runtime::frame::discard_pass(ui.ctx(), "gallery: a card's story showed a dialog before it painted");
            } else if dialog.known && !dialog.hidden && dialog.shown && !shown {
                *dialog = DialogState { known: true, hidden: true, shown: false, painted: true };
                craft_ui::runtime::frame::discard_pass(ui.ctx(), "gallery: a card's dialog closed");
            } else {
                dialog.shown |= dialog.known && shown;
                dialog.painted = true;
            }
            // The start is the values after the story's first real run: a pass that took a kept measurement, ran
            // only the measuring run or held the story back has not made them yet.
            state.note_start();
        }
        BodyShown { flow: Some(surface.output.rect) }
    }

    /// The shown value as the story's component words it ([`StoryOutput::value_text`]), as this pass's run of the
    /// story found it, or the kept measurement of a body around the story that this pass took in place of running it.
    pub fn value_text(&self, ui: &Ui) -> Option<String> {
        layout_memory::text(ui.ctx(), self.value_key()).map(|text| text.to_string())
    }

    fn value_key(&self) -> Id {
        craft_ui::testing::memory::global("gallery.card.value", &self.key)
    }
}

/// Puts a story whose dialog shows at its start behind its button, with the dialog closed. The card's start is the
/// story with its dialog closed, so a dialog closed by its own button is back at the start and Reset hides.
fn close_at_start(state: &mut CardState) {
    state.dialog = DialogState { known: true, hidden: true, shown: false, painted: false };
    if let Some(open) = state.data.present::<bool>("open") {
        *open = false;
    }
    state.start = None;
}

/// What a story runs in inside its card.
#[derive(Clone, Copy)]
struct Host<'a> {
    caption: Option<&'a str>,
    id: Id,
    /// The most a story that fills takes.
    room: f32,
    look: Look,
    surface: Surface,
    gated: bool,
    panel_pad: Pad,
}

impl Host<'_> {
    /// The story's surface over the card's width, with the card's mode.
    fn stack(&self) -> Stack<'static> {
        let kind = match self.surface {
            Surface::Panel => SurfaceKind::Panel,
            Surface::Canvas => SurfaceKind::Canvas,
            Surface::Chrome => SurfaceKind::Chrome,
            // A story that paints its own surface sits on the background, as on the old gallery's cards.
            _ => SurfaceKind::Background,
        };
        // On the panel surface the margin is the body's `panel_pad` (in a card, the card's padding is the margin); another
        // surface keeps a margin of its own inside it.
        let pad = if kind == SurfaceKind::Panel { self.panel_pad } else { Pad::Regular };
        let mut stack = Stack::new().id(self.id).fill_width().room_height(self.room).surface(kind).pad(pad);
        if let Some(mode) = self.look.mode {
            stack = stack.theme(ThemeScope::mode(mode));
        }
        stack
    }

    /// The caption, when the story has one.
    fn caption_in(&self, ui: &mut Ui) {
        if let Some(caption) = self.caption {
            Text::new(caption.to_owned()).shade(Shade::Muted).show(ui);
        }
    }

    /// The story under its caption, in the card's direction and control size; shown as the inspected region `region`
    /// when given, with the region's record of a real run.
    fn content(&self, ui: &mut Ui, show: StoryFn, data: &mut StoryData, region: Option<Id>) -> (StoryOutput, Option<Arc<Recorded>>) {
        self.caption_in(ui);
        let (look, gated) = (self.look, self.gated);
        with_env(
            ui,
            |env| {
                env.direction = look.direction;
                env.size = look.size;
                env.wheel = gated.then_some(craft_ui::runtime::env::WheelScope::Gated);
            },
            |ui| match region {
                Some(key) => inspect::record(ui, key, |ui| show(ui, data)),
                None => (show(ui, data), None),
            },
        )
    }
}
