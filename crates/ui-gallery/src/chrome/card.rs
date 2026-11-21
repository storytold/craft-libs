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
use craft_ui::contract::{
    CommandSet, Commands as _, Gap, MemoProps, MemoView, Outbox, Pad, Props as _, ShowContainer as _, Sizing, StoryLayout, Surface, ThemeScope,
};
use craft_ui::display::{InspectLayers, InspectOverlay, Shade, Text};
use craft_ui::layout_components::{Grid, Stack};
use craft_ui::runtime::env::Env;
use craft_ui::runtime::env::with_env;
use craft_ui::runtime::inspect;
use craft_ui::runtime::layout_memory;
use craft_ui::sections::{HeaderCommand, HeaderStrength, Panel};
use craft_ui::style::SurfaceKind;
use craft_ui::testing::{StoryData, StoryOutput, StoryRegistration, least_stage_width};

/// A story's show function.
type StoryFn = fn(&mut Ui, &mut StoryData) -> StoryOutput;
use egui::{Id, Rect, Ui};

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
#[derive(Clone, Copy, Debug, Default, PartialEq, MemoProps)]
#[non_exhaustive]
pub struct Look {
    /// The mode the story's tokens resolve in.
    #[memo_props(hash = "a fieldless enum of the theme")]
    pub mode: Option<Mode>,
    /// The layout direction.
    #[memo_props(hash = "a fieldless enum of the i18n crate")]
    pub direction: Option<Direction>,
    /// The control size.
    #[memo_props(hash = "a fieldless enum of the theme")]
    pub size: Option<ControlSize>,
    /// The component set.
    pub set: Option<craft_theme::ComponentSet>,
}

/// One story a card shows.
#[non_exhaustive]
pub struct CardStory<'a> {
    /// The index of the story in the catalog: the key of its state.
    pub index: usize,
    /// The story.
    pub story: StoryRegistration,
    /// The story's caption.
    pub caption: &'a str,
}

// The card reads only the story's name and show function; registry values are owned copies.
impl MemoProps for CardStory<'_> {
    fn key(&self, key: &mut craft_ui::contract::MemoKey) {
        self.index.key(key);
        self.caption.key(key);
        self.story.story.name.key(key);
        key.feed_hash(&self.story.story.show);
    }
}

impl<'a> CardStory<'a> {
    /// The story at catalog index `index`, under `caption`.
    pub fn new(index: usize, story: StoryRegistration, caption: &'a str) -> Self {
        CardStory { index, story, caption }
    }
}

/// What one card shows.
#[derive(MemoProps)]
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

/// The column lists of story grids, by count.
const COLUMNS: [ColumnWidth; 4] = [ColumnWidth::fill(1.0); 4];

/// The columns of a form-rows card: the captions, the stories, then the values they show. The stories' column is as
/// wide as its widest story, so each value sits a gap past its control, as the old gallery showed it.
const FORM_ROWS: &[ColumnWidth] = &[ColumnWidth::Content, ColumnWidth::Content, ColumnWidth::fill(1.0)];

/// The states of the stories a card shows, by catalog index.
type States<'a, 's> = &'s mut [(usize, &'a mut CardState)];

/// The state of the story at catalog index `index`.
fn state_of<'s>(states: &'s mut [(usize, &mut CardState)], index: usize) -> &'s mut CardState {
    match states.iter_mut().find(|(at, _)| *at == index) {
        Some((_, state)) => state,
        None => unreachable!("the card holds the state of each of its stories"),
    }
}

/// What a card's view reads: the card and the states of its stories. The card's own run reports through `events`.
#[derive(MemoProps)]
struct CardProps<'a> {
    card: &'a Card<'a>,
    states: Vec<(usize, &'a mut CardState)>,
    events: Outbox<'a, CardEvents>,
}

fn card_view(props: &mut CardProps<'_>, ui: &mut Ui) -> Option<Rect> {
    let CardProps { card, states, events } = props;
    let shown = card.run(ui, states);
    let rect = shown.rect;
    events.send(shown);
    rect
}

impl Card<'_> {
    /// Shows the card in the room `ui` offers, with the states of its stories: a memoized view, which replays its last
    /// run until the card, a state of its stories or what it shows around it changes. A replay reports no events.
    pub fn show(&self, ui: &mut Ui, cells: &mut Cells) -> CardEvents {
        let indices: Vec<usize> = self.stories.iter().map(|story| story.index).collect();
        let states = cells.states_of(&indices);
        let mut sent = Vec::new();
        let props = CardProps { card: self, states, events: Outbox::new(&mut sent) };
        let rect = MemoView::new(("gallery.card", &self.key), props, card_view).show(ui).flatten();
        let mut events = sent.pop().unwrap_or_default();
        events.rect = rect;
        events
    }

    fn run(&self, ui: &mut Ui, states: States<'_, '_>) -> CardEvents {
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
            panel = panel.note(note.to_owned());
        }
        let mut commands = Vec::new();
        if self.open {
            commands.push(HeaderCommand::new(IconId::new("open"), CardCommands::Open).size(ControlSize::Sm));
        }
        if self.usable {
            commands.push(HeaderCommand::new(IconId::new("check"), CardCommands::Use).size(ControlSize::Sm));
        }
        let away = states.iter().any(|(_, state)| state.changed() || state.dialog.shown);
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
            self.stories_in(ui, &mut *states);
        });
        events.rect = Some(shown.response.rect);
        events
    }

    /// The card's stories in its layout; a lone story outside form rows without its caption.
    fn stories_in(&self, ui: &mut Ui, states: States<'_, '_>) {
        if let ([story], false) = (self.stories, self.layout == StoryLayout::FormRows) {
            self.body(ui, story, None, states);
            return;
        }
        match self.layout {
            StoryLayout::FormRows => {
                Grid::new(FORM_ROWS).fill_width().show(ui, |ui, _| {
                    for story in self.stories {
                        Text::new(story.caption.to_owned()).show(ui);
                        let body = self.story_body(story, None);
                        body.show(ui, state_of(states, story.index));
                        Text::new(body.value_text(ui).unwrap_or_default()).shade(Shade::Muted).show(ui);
                    }
                });
            }
            StoryLayout::Columns(most) => {
                let gap = Gap::Item.points(ui);
                // The narrowest a column of stories may be is the least story width, as the old gallery's grids kept
                // their cases.
                let least = least_stage_width(&Env::of(ui).tokens);
                let fit = ((ui.available_width() + gap) / (least + gap)).floor() as usize;
                let columns = fit.clamp(1, usize::from(most).clamp(1, COLUMNS.len()));
                Grid::new(&COLUMNS[..columns]).gap(Gap::Item).fill_width().show(ui, |ui, _| {
                    for story in self.stories {
                        self.body(ui, story, Some(story.caption), states);
                    }
                });
            }
            _ => {
                Stack::new().fill_width().show(ui, |ui, _| {
                    for story in self.stories {
                        self.body(ui, story, Some(story.caption), states);
                    }
                });
            }
        }
    }

    /// The body of `story`, with its state, under `caption` when given.
    fn body(&self, ui: &mut Ui, story: &CardStory<'_>, caption: Option<&str>, states: States<'_, '_>) -> BodyShown {
        self.story_body(story, caption).show(ui, state_of(states, story.index))
    }

    /// The body of `story` under `caption` when given.
    fn story_body(&self, story: &CardStory<'_>, caption: Option<&str>) -> StoryBody {
        let key = format!("{}/{}", self.key, story.story.story.name);
        StoryBody {
            look: self.look,
            gated: self.gated,
            caption: caption.map(str::to_owned),
            inspect: self.inspect.as_ref().map(|inspected| (region_key(&inspected.tab, story.index), inspected.layers)),
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
    pub story: StoryRegistration,
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
    /// The story's output when the pass ran it for real: not in a sizing pass or a replay of its view, nor held back
    /// behind its button.
    pub output: Option<StoryOutput>,
}

impl StoryBody {
    /// The body of `story` on `surface`, in the window's look, with ungated wheel takers.
    pub fn new(key: impl Into<String>, story: StoryRegistration, surface: Surface, room: f32, panel_pad: Pad) -> Self {
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
        let mut reported = Vec::new();
        let props = StoryProps {
            show,
            data: &mut state.data,
            host,
            runs,
            region: self.inspect.map(|(key, _)| key),
            value_key: self.value_key(),
            reported: Outbox::new(&mut reported),
        };
        let surface = MemoView::new((host.id, "story"), props, story_view).show(ui);
        // What a real run of the story in this pass reported; `None` when it did not run for real.
        let run = reported.pop();
        // Whether a dialog of the story showed in that run.
        let ran = run.as_ref().and_then(|run| run.dialog);
        // The story's output of that run.
        let real = run.and_then(|run| run.output);
        if !runs {
            ui.ctx().request_repaint();
        }
        if let Some((key, layers)) = self.inspect
            && let Some(recorded) = inspect::recorded(ui.ctx(), key)
        {
            let pointed = inspect::pointed(ui.ctx(), key);
            let overlay = InspectOverlay::new(recorded).layers(layers);
            let overlay = match pointed {
                Some(index) => overlay.pointed(index),
                None => overlay,
            };
            if let Some(rect) = surface {
                overlay.show_at(ui, rect);
            }
        }
        if let Some(shown) = ran {
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
        BodyShown { flow: surface, output: real }
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
    state.data.settle();
    state.start = None;
}

/// What a story runs in inside its card.
#[derive(Clone, Copy, MemoProps)]
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
            stack = stack.theme(ThemeScope::new().mode(mode));
        }
        stack
    }
}

/// What a story's run reported: whether a dialog of it showed and the story's output, both `None` in a sizing run.
struct StoryRun {
    dialog: Option<bool>,
    output: Option<StoryOutput>,
}

/// What a story's view shows: the story's surface with the story under its caption, in the card's direction and control
/// size, as the inspected region `region` when given, and its readout under it. The surface is in the view, so a story
/// that fills takes its content's height there, as the surface sizes from its content.
#[derive(MemoProps)]
struct StoryProps<'a> {
    #[memo_props(hash = "a fn pointer names the story's code")]
    show: StoryFn,
    data: &'a mut StoryData,
    host: Host<'a>,
    /// Whether the story runs in this pass: the surface shows empty otherwise.
    runs: bool,
    region: Option<Id>,
    /// Where the story's value text is handed forward (`layout_memory::emit_text`).
    value_key: Id,
    /// What the run reported: a replay sends nothing.
    reported: Outbox<'a, StoryRun>,
}

fn story_view(props: &mut StoryProps<'_>, ui: &mut Ui) -> Rect {
    let StoryProps { show, data, host, runs, region, value_key, reported } = props;
    let (host, show, region, value_key) = (*host, *show, *region, *value_key);
    host.stack()
        .show(ui, |ui, cx| {
            if !*runs {
                return;
            }
            let shell = cx.shell();
            let before = shell.as_ref().map_or(0, |shell| shell.dialogs_shown());
            if let Some(caption) = host.caption {
                Text::new(caption.to_owned()).shade(Shade::Muted).show(ui);
            }
            let look = host.look;
            let output = with_env(
                ui,
                |env| {
                    env.direction = look.direction;
                    env.size = look.size;
                    env.wheel = host.gated.then_some(craft_ui::runtime::env::WheelScope::Gated);
                    if let Some(set) = look.set {
                        env.values.set(set);
                    }
                },
                |ui| match region {
                    Some(key) => inspect::record(ui, key, |ui| show(ui, data)).0,
                    None => show(ui, data),
                },
            );
            // The run may have changed a value: settling moves the revision the view keys on, so it runs once more and
            // replays.
            data.settle();
            if let Some(readout) = &output.readout {
                Text::new(readout.clone()).shade(Shade::Muted).show(ui);
            }
            layout_memory::emit_text(ui.ctx(), value_key, output.value_text.as_deref());
            let real = !ui.is_sizing_pass();
            let dialog = real.then(|| shell.is_some_and(|shell| shell.dialogs_shown() > before));
            reported.send(StoryRun { dialog, output: real.then_some(output) });
        })
        .output
        .rect
}
