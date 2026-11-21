//! `Keytips`: the Alt and F10 key hints of a ribbon. Badges over its tabs and controls name the letters that pick a
//! tab or run a command, so the whole ribbon is reachable from the keyboard without moving focus off the document.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use std::sync::Arc;

use craft_behavior::{Insets, Metrics, Sense};
use craft_keymap::mnemonic::assign_keytips;
use craft_keymap::{Chord, CommandRef, Key, Mods};
use craft_ui::contract::{Common, Component, Cx, Focus as _, MeasureCx, MenuKey, NoValue, Outcome, PaintUi, Remember as _, Role, Styled as _};
use craft_ui::primitives::TextRun;
use craft_ui::runtime::commands::{self, FireSource};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::overlay::{self, OpenFilter};
use craft_ui::runtime::{focus, inspect};
use craft_ui::style::{ComponentStyle, StateSet, Stateful, StyleCx, SurfaceLook, TextLook, TypeRole, UiTokens};
use egui::{Context, Id, Pos2, Rect, Response, Ui, Vec2};

/// Escape ends the keytips while they show.
const ESCAPE: Chord = Chord::plain(Key::Escape);

/// The look of the badges.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct KeytipsStyle {
    /// The room around a badge's letters; its top is also a badge's inset from its control's corner.
    #[style(metrics)]
    metrics: Metrics,
    /// A badge: the accent fill, with the ring that parts it from the control under it. Not the component's root
    /// surface, which would fill the whole area the keytips show over.
    badge: SurfaceLook,
    /// The badge letters.
    #[style(text)]
    text: TextLook,
}

/// What the keys are on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    /// No keytips: the ribbon works as without them.
    #[default]
    Off,
    /// Badges over the tabs and the digit badges.
    Tabs,
    /// Badges over the controls of the shown tab.
    Commands,
    /// Partway through a host sequence, which shows no badges.
    Sequence,
}

/// The keytips between passes: held interaction state, changed only in the pass that receives a key.
#[derive(Clone, Debug, Default)]
struct State {
    phase: Phase,
    /// The badge letters typed so far in this phase.
    typed: String,
}

impl State {
    fn active(&self) -> bool {
        self.phase != Phase::Off
    }
}

/// What completing a badge or a sequence does.
#[derive(Clone, Debug)]
enum Act {
    /// Picks the tab of this badge, as the app gave it.
    Tab(String),
    /// Fires the command.
    Fire(CommandRef),
}

/// Where a badge shows.
#[derive(Clone, Debug)]
enum Place {
    /// On the element that stands for the access node of this id (a tab).
    Tab(Id),
    /// On the element bound to this command.
    Command(CommandRef),
    /// Nowhere: a host sequence.
    Hidden,
}

/// A badge or sequence the keys can complete in a phase.
#[derive(Clone, Debug)]
struct Entry {
    /// The letters, upper case.
    badge: String,
    act: Act,
    place: Place,
}

/// What a key the keytips claimed means.
#[derive(Clone, Copy, Debug)]
enum Typed {
    /// A badge or sequence letter.
    Letter(char),
    /// Alt with a tab badge's first letter.
    Tab(char),
    /// Escape.
    Escape,
}

/// What a pass's keys did, reported by that pass only.
#[derive(Clone, Debug, Default)]
struct Reported {
    tab: Option<String>,
    fired: Option<CommandRef>,
}

/// Alt and F10 keytips with one- and two-letter badges ([`assign_keytips`]), placed at the rect of each element of
/// the given inspected-region records ([`Keytips::places`]) that has a tab's id or is bound to a control's command,
/// from this pass's records.
///
/// - A lone Alt tap or F10 shows the tab badges and the digit badges, and hides them again; Alt with a letter picks
///   the tab of that badge at once (the chord is claimed, so any other Alt+letter stays the app's shortcut). On macOS
///   only F10 opens them, since Option types characters there. Escape ends them, and so does a press anywhere: the
///   press does what it does.
/// - Typing a tab's badge reports the tab ([`KeytipsOutput::tab`]) and shows the badges of the controls the app
///   passes for it. Typing a control's or a digit's badge fires its command with `FireSource::Keytip` and ends the
///   keytips. After the first letter of a two-letter badge only the badges it can complete show.
/// - A letter a badge or a sequence can still need is claimed while the keytips show, so it never types into the
///   focused document, and the letters of one frame apply in the order they arrived; a letter no badge has is not
///   taken.
/// - Off while a text field or a shortcut recorder has focus (`focus::takes_letters`), under a modal layer, while the
///   window is in the background, and when the app passes `false` to [`Keytips::new`] because its window shows an in-window menu bar,
///   which takes Alt and F10 itself.
///
/// The badges paint in the layer of the `Ui` the keytips show in, over its whole clip rect, so the keytips show after
/// the ribbon, in a `Ui` that covers it and any other recorded region.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[must_use = "show the keytips with `show`"]
pub struct Keytips<'a> {
    #[common]
    common: Common,
    /// The ribbon's tabs, each with its name, its badge letters and the id of its access node (a ribbon reports them in
    /// `RibbonOutput::tab_ids`). A tab's badge shows on the element of the records that stands for that node
    /// (`Element::node`), and [`KeytipsOutput::tab`] reports the badge.
    #[prop]
    tabs: &'a [(Label, &'a str, Id)],
    /// The controls of the shown tab that get a badge, in the order they are shown: their badges come from their
    /// labels and fixed letters ([`assign_keytips`]).
    #[prop]
    controls: &'a [KeytipControl<'a>],
    /// Fixed letter sequences typed after Alt, each with the command it fires. They show no badge, start only on a
    /// letter they lead with, and wait for their next letter until they are complete or Escape ends them.
    #[prop]
    sequences: &'a [(&'a str, CommandRef)],
    /// The commands the digit badges fire, in order (a quick access toolbar's): the first nine take one digit, the
    /// rest a leading zero and digits of one width, so no digit badge is the start of another.
    #[prop]
    digits: &'a [CommandRef],
    /// Letters no assigned control badge may use: the letters the app's Alt+letter shortcuts take.
    #[prop]
    reserved: &'a [char],
    /// The inspected-region records whose elements the badges are placed at: the ribbon's, and any other region the
    /// app records (a quick access toolbar in the title bar).
    #[prop]
    places: &'a [Arc<inspect::Recorded>],
    /// The state this pass ends with.
    state: State,
    /// Each badge of this pass with the rect of the control it sits on, in this component's layer.
    badges: Vec<(Rect, String)>,
    reported: Reported,
}

impl<'a> Keytips<'a> {
    /// Keytips that react to the keys when `enabled`. An app that shows an in-window menu bar passes `false`: that
    /// bar takes Alt and F10.
    pub fn new(enabled: bool) -> Self {
        let keytips = Keytips {
            common: Common::new(),
            tabs: &[],
            controls: &[],
            sequences: &[],
            digits: &[],
            reserved: &[],
            places: &[],
            state: State::default(),
            badges: Vec::new(),
            reported: Reported::default(),
        };
        craft_ui::contract::Props::enabled(keytips, enabled)
    }

    /// Shows the badges and reads the keys. Show it after the regions it places badges on, in a `Ui` that covers
    /// them.
    pub fn show_over(self, ui: &mut Ui) -> KeytipsOutput {
        let area = ui.clip_rect().intersect(ui.ctx().content_rect());
        craft_ui::contract::Show::show_at(self, ui, area)
    }

    /// The badges of the controls, in the order of `controls`; an empty badge shows nothing.
    fn control_badges(&self, cx: &Cx<'_, Self>) -> Vec<String> {
        let labels: Vec<(&str, Option<&str>)> = self.controls.iter().map(|control| (control.label, control.letters)).collect();
        assign_keytips(&labels, self.reserved, &cx.env.folder())
    }

    /// Every badge and sequence the keys can complete in `phase`.
    fn entries(&self, phase: Phase, controls: &[String]) -> Vec<Entry> {
        let fire = |badge: String, command: &CommandRef, place: Place| Entry { badge, act: Act::Fire(command.clone()), place };
        let sequences = move || self.sequences.iter().map(move |(keys, command)| fire(keys.to_ascii_uppercase(), command, Place::Hidden));
        match phase {
            Phase::Off => Vec::new(),
            Phase::Tabs => {
                let tabs = self.tabs.iter().map(|(_, badge, id)| Entry {
                    badge: badge.to_ascii_uppercase(),
                    act: Act::Tab((*badge).to_owned()),
                    place: Place::Tab(*id),
                });
                let count = self.digits.len();
                let digits =
                    self.digits.iter().enumerate().map(|(index, command)| fire(digit_badge(index, count), command, Place::Command(command.clone())));
                tabs.chain(digits).chain(sequences()).filter(|entry| !entry.badge.is_empty()).collect()
            }
            Phase::Commands => self
                .controls
                .iter()
                .zip(controls)
                .filter(|(_, badge)| !badge.is_empty())
                .map(|(control, badge)| fire(badge.clone(), &control.command, Place::Command(control.command.clone())))
                .collect(),
            Phase::Sequence => sequences().collect(),
        }
    }

    /// The letters the entries `state` leads to can still need, with their chords. All of them are claimed, not only
    /// the next one: one frame can carry a whole badge, and the claim made by the pass before is the only thing that
    /// keeps its letters from typing into the document.
    fn needed(&self, state: &State, controls: &[String]) -> Vec<(char, Chord)> {
        let mut keys: Vec<(char, Chord)> = Vec::new();
        for entry in self.entries(state.phase, controls) {
            let Some(rest) = entry.badge.strip_prefix(state.typed.as_str()) else { continue };
            for letter in rest.chars() {
                if keys.iter().any(|(known, _)| *known == letter) {
                    continue;
                }
                if let Some(key) = Key::from_char(letter) {
                    keys.push((letter, Chord::plain(key)));
                }
            }
        }
        keys
    }

    /// The Alt+letter chord of each tab's first badge letter, with the letter: they pick a tab at once. None on
    /// macOS, where Option types characters.
    fn tab_chords(&self, cx: &Cx<'_, Self>) -> Vec<(Chord, char)> {
        if cx.env.shell().os.is_mac() {
            return Vec::new();
        }
        self.tabs
            .iter()
            .filter_map(|(_, badge, _)| {
                let letter = badge.chars().next()?.to_ascii_uppercase();
                Some((Chord::new(Mods::ALT, Key::from_char(letter)?), letter))
            })
            .collect()
    }

    /// The letter `letter` typed in `state`. A complete badge or sequence acts and ends its phase; the start of one
    /// waits for the next letter; a letter that leads nowhere changes nothing.
    fn press(&mut self, state: &mut State, letter: char, controls: &[String], cx: &Cx<'_, Self>) {
        let typed = format!("{}{letter}", state.typed);
        let entries = self.entries(state.phase, controls);
        if let Some(entry) = entries.iter().find(|entry| entry.badge == typed) {
            match &entry.act {
                Act::Tab(badge) => {
                    self.reported.tab = Some(badge.clone());
                    *state = State { phase: Phase::Commands, typed: String::new() };
                }
                Act::Fire(command) => {
                    if commands::fire(&cx.response.ctx, command.clone(), FireSource::Keytip) {
                        self.reported.fired = Some(command.clone());
                    }
                    *state = State::default();
                }
            }
            return;
        }
        let leading: Vec<&Entry> = entries.iter().filter(|entry| entry.badge.starts_with(&typed)).collect();
        if leading.is_empty() {
            return;
        }
        // Only sequences go on from here: their hint stands in for the badges.
        if leading.iter().all(|entry| matches!(entry.place, Place::Hidden)) {
            state.phase = Phase::Sequence;
        }
        state.typed = typed;
    }

    /// The badges `state` shows, each at the rect of its element in the layer `layer`. An element takes one badge.
    fn placed(&self, state: &State, controls: &[String], cx: &Cx<'_, Self>) -> Vec<(Rect, String)> {
        let ctx = &cx.response.ctx;
        let mut taken: Vec<(usize, usize)> = Vec::new();
        let mut placed = Vec::new();
        for entry in self.entries(state.phase, controls) {
            if !entry.badge.starts_with(&state.typed) {
                continue;
            }
            let found = match &entry.place {
                Place::Hidden => None,
                Place::Tab(id) => self.find(&taken, |element| element.node() == *id),
                Place::Command(command) => self.find(&taken, |element| element.command.as_ref() == Some(command)),
            };
            let Some((region, index)) = found else { continue };
            taken.push((region, index));
            if let Some(rect) = self.places[region].rect_in(ctx, index, cx.response.layer_id) {
                placed.push((rect, entry.badge));
            }
        }
        placed
    }

    /// The first element of the records, in their order, that `matches` and has no badge yet: its region and index.
    fn find(&self, taken: &[(usize, usize)], matches: impl Fn(&inspect::Element) -> bool) -> Option<(usize, usize)> {
        self.places.iter().enumerate().find_map(|(region, recorded)| {
            recorded
                .elements
                .iter()
                .enumerate()
                .find(|(index, element)| element.rect.is_positive() && !taken.contains(&(region, *index)) && matches(element))
                .map(|(index, _)| (region, index))
        })
    }
}

/// The digit badge of the command at `index` of `count`: one digit for the first nine, then a leading zero and the
/// rest's number, all of one width (12 commands: `1`..`9`, `01`, `02`, `03`).
fn digit_badge(index: usize, count: usize) -> String {
    if index < 9 {
        (index + 1).to_string()
    } else {
        let width = count.saturating_sub(9).to_string().len();
        format!("0{:0width$}", index - 8)
    }
}

/// Whether the keytips stay off: a text field or a shortcut recorder has focus (the letters are typed; a document's
/// editor is neither, see `focus::takes_letters`), a modal layer is open, or the window is in the background.
fn suppressed(ctx: &Context) -> bool {
    let typing = focus::focused_rec(ctx).is_some_and(|rec| focus::takes_letters(&rec.stop.keys));
    typing || overlay::any_open(ctx, OpenFilter::Modal) || !ctx.input(|input| input.focused)
}

/// Where a badge with letters `text` across goes on the control at `control`: inside its top start corner when the
/// control is more than twice the badge's height, else centred on that corner, so it hangs into the gap beside a
/// compact control instead of covering its glyph. The badge is the letters in `padding`.
fn badge_rect(control: Rect, text: Vec2, padding: Insets, rtl: bool) -> Rect {
    let height = text.y + padding.top + padding.bottom;
    let size = Vec2::new((text.x + padding.start + padding.end).max(height), height);
    let corner = if rtl { control.right_top() } else { control.left_top() };
    if size.y * 2.0 > control.height() {
        return Rect::from_center_size(corner, size);
    }
    let inset = padding.top;
    let x = if rtl { corner.x - inset - size.x } else { corner.x + inset };
    Rect::from_min_size(Pos2::new(x, corner.y + inset), size)
}

impl Component for Keytips<'_> {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const INPUT: craft_behavior::Input = craft_behavior::Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = KeytipsStyle;
    type Output = KeytipsOutput;

    fn style(cx: &StyleCx<'_>, _: KeytipsStyleKey) -> KeytipsStyle {
        let colors = &cx.tokens.color;
        let pad = cx.tokens.space.pad.chip;
        let badge = SurfaceLook::new(Stateful::new(cx.color(colors.accent.solid)))
            .radius(UiTokens::corner(cx.tokens.shape.radius.sm))
            .border(Stateful::new(cx.color(colors.overlay.halo)), cx.tokens.stroke.thin);
        KeytipsStyle {
            metrics: Metrics::new().padding(Insets::new(pad, pad, pad, pad)),
            badge,
            text: TextLook::new(TypeRole::Caption, Stateful::new(cx.color(colors.text.on_accent))),
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    /// The keys of this pass. Only the pass that receives the frame's input sees its menu keys and claimed chords, so
    /// a second pass of the frame keeps the state the first one wrote and reports nothing new.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let ctx = cx.response.ctx.clone();
        if !cx.resolved.enabled || suppressed(&ctx) {
            *cx.memory::<State>() = State::default();
            return;
        }
        // Alt+letter is a mnemonic of this bar only for the tab chords it claims: any other stays the app's shortcut.
        cx.register_menu_bar_by_claim();
        let before = cx.memory::<State>().clone();
        let mut state = before.clone();
        let controls = self.control_badges(cx);
        for key in cx.take_menu_keys() {
            if matches!(key, MenuKey::Focus) {
                state = State { phase: if state.phase == Phase::Tabs { Phase::Off } else { Phase::Tabs }, typed: String::new() };
            }
        }
        // The chords claimed for this frame are those the last pass claimed; they apply in the order they arrived.
        let tab_chords = self.tab_chords(cx);
        let mut keys: Vec<(Chord, Typed)> = tab_chords.iter().map(|(chord, letter)| (*chord, Typed::Tab(*letter))).collect();
        if before.active() {
            keys.extend(self.needed(&before, &controls).into_iter().map(|(letter, chord)| (chord, Typed::Letter(letter))));
            keys.push((ESCAPE, Typed::Escape));
        }
        let chords: Vec<Chord> = keys.iter().map(|(chord, _)| *chord).collect();
        while let Some(chord) = cx.take_first_claimed(&chords) {
            match keys.iter().find(|(known, _)| *known == chord).map(|(_, typed)| *typed) {
                Some(Typed::Letter(letter)) => self.press(&mut state, letter, &controls, cx),
                Some(Typed::Escape) => state = State::default(),
                Some(Typed::Tab(letter)) => {
                    // Alt with a tab's letter picks the tab at once; a badge that is no tab's leaves the keytips as they are.
                    let mut tried = State { phase: Phase::Tabs, typed: String::new() };
                    self.press(&mut tried, letter, &controls, cx);
                    if tried.phase != Phase::Tabs || !tried.typed.is_empty() {
                        state = tried;
                    }
                }
                None => {}
            }
        }
        // A press ends the keytips, as a click anywhere does in a ribbon app; the press itself does what it does. The
        // input of a pass is its own, so a second pass of the frame sees no press.
        if state.active() && ctx.input(|input| input.pointer.any_pressed()) {
            state = State::default();
        }
        for (chord, _) in &tab_chords {
            cx.claim(*chord);
        }
        if state.active() {
            for (_, chord) in self.needed(&state, &controls) {
                cx.claim(chord);
            }
            cx.claim(ESCAPE);
        }
        self.badges = if state.phase == Phase::Sequence { Vec::new() } else { self.placed(&state, &controls, cx) };
        *cx.memory::<State>() = state.clone();
        self.state = state;
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let draw = ui.draw();
        let style = cx.style;
        let rtl = cx.env.direction().is_rtl();
        let color = cx.resolve(&style.text.color);
        let at_rest = StateSet::default();
        for (control, badge) in &self.badges {
            let run = TextRun::new(badge.clone()).role(style.text.role).color(color);
            let layout = draw.layout(&run, f32::INFINITY);
            let rect = badge_rect(*control, layout.size(), style.metrics.padding, rtl);
            draw.surface_look(rect, &style.badge, &at_rest);
            draw.galley(rect.center() - layout.size() / 2.0, &layout);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> KeytipsOutput {
        let pending = (self.state.active() && !self.state.typed.is_empty()).then(|| self.state.typed.clone());
        KeytipsOutput { tab: self.reported.tab, fired: self.reported.fired, pending, active: self.state.active(), response: cx.response.clone() }
    }
}

/// A control that gets a keytip.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct KeytipControl<'a> {
    /// The control's command: its badge shows on the element of the records bound to it.
    pub command: CommandRef,
    /// The text the badge letters come from.
    pub label: &'a str,
    /// The letters the app fixes for the badge, when it does.
    pub letters: Option<&'a str>,
}

impl<'a> KeytipControl<'a> {
    /// A control firing `command`, its badge letters taken from `label` unless `letters` fixes them.
    pub const fn new(command: CommandRef, label: &'a str, letters: Option<&'a str>) -> Self {
        KeytipControl { command, label, letters }
    }
}

/// What the keytips report for a pass. `tab` and `fired` come from the pass that received the key only.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct KeytipsOutput {
    /// The tab the keys chose, by its badge as the app gave it.
    pub tab: Option<String>,
    /// The command the keys fired; it went through the command queue with `FireSource::Keytip`.
    pub fired: Option<CommandRef>,
    /// The badge or sequence letters typed so far, while they wait for the next one.
    pub pending: Option<String>,
    /// Whether the keytips are showing.
    pub active: bool,
    /// The component's response, which every component output carries.
    response: Response,
}

impl Outcome for KeytipsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}
