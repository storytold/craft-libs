//! The key stage (spec 9.1, 9.2, 9.3, 9.7): the input hook strips the keys the runtime owns and records
//! every key event of the pass, the begin hook turns them into the `Intents` of the focused component,
//! and claims, menu-bar keys and AccessKit action requests are routed from the same record.

use crate::runtime::commands::{ClipContent, FirePhase};
use crate::runtime::egui::input::Action;
use crate::runtime::frame::PerViewport;
use craft_behavior::EditCommand;
use craft_keymap::{Chord, Key, KeyInput, Layout, Mods};
use egui::{Context, Id, IdMap, Vec2, ViewportId};
use std::collections::HashMap;

mod claims;
mod input;
mod intents;
mod nodes;
mod route;

#[doc(hidden)]
pub use claims::capture;
#[doc(hidden)]
pub use claims::claim;
#[doc(hidden)]
pub use claims::take_claimed;
#[doc(hidden)]
pub use claims::{claim_alt_tap, take_alt_tap};
pub(crate) use claims::{fire_claimed, hold_escape, take_first_claimed};
/// The types `Intents` reads and returns, for an app that reads the window's intents (`with_window_intents`).
pub use craft_behavior::{AccessRequest, Intent, MoveMods, NavMove, PartRef};
pub use input::held;
#[doc(hidden)]
pub use input::keyed;
pub use input::textless_paste;
pub(crate) use input::{claims_press, count_sent, input_hook, keys_state, note_ime_presses, queue_nav, sent, stirred, take_menu_keys, take_nav};
pub use intents::Intents;
#[doc(hidden)]
pub use intents::intents_of;
#[cfg(feature = "suite")]
pub(crate) use intents::lent_taken;
#[doc(hidden)]
pub use intents::settle;
#[doc(hidden)]
pub use intents::take_submit;
pub(crate) use intents::{
    Lent, context_part, context_request, context_requests, give_back, hand_on_cancel, lend, request_context, route_whole_reset, scope_escape,
    take_cancel, take_every_escape, take_window_cancel,
};
pub use intents::{with_canvas_window_intents, with_window_intents};
pub(crate) use nodes::{redirect_node, register_expanded, register_node};
pub(crate) use route::{
    activated_member, begin, focus_in_modal_dialog, layout_of, owner_at, pasted, pasted_text, queue_edit, routed_to, take_shortcuts,
};
pub use route::{escape_mods, focus_holds};

/// A menu-bar key of an in-window bar (spec 9.7). F10 reaches the bar on every OS; Alt keys only off a Mac.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKey {
    /// F10 or a lone Alt press: focus the menu bar.
    Focus,
    /// Alt and a letter: open the menu with that mnemonic.
    Letter(char),
}

/// Tab and F6 moves the input hook stripped: `true` is forward.
#[derive(Default)]
pub(crate) struct NavKeys {
    pub(crate) tabs: Vec<bool>,
    pub(crate) panes: Vec<bool>,
}

struct Press {
    key: Key,
    mods: Mods,
    /// An auto-repeat of a key that is still held.
    repeat: bool,
    /// The press's place among the key presses the strip left in egui's input, `None` when the strip took it.
    in_egui: Option<usize>,
    /// The key the press's release names (the physical key, else the typed one), which the strip matches releases by.
    release: Option<Key>,
    /// The place in `Batch::texts` of the text the press typed.
    text: Option<usize>,
    /// The Space of the canvas a dialog keeps open (`runtime::sampler`): the canvas's press, which no component's
    /// routing reads, no focused stop holds and no field types.
    canvas: bool,
    /// The press's place among the frame's events ([`Batch::place`]).
    place: usize,
    /// The keys down when it went down, itself not among them: what a nudge reads as held with its arrow.
    held: Vec<Key>,
    /// An input method committed text right after it: the press typed that text, which a text stop takes.
    ime: bool,
}

/// An arrow press as a nudge of content: its unit direction, its modifiers and the keys down when it went down.
#[derive(Clone, Debug)]
pub(crate) struct Nudge {
    pub(crate) direction: Vec2,
    pub(crate) mods: Mods,
    pub(crate) held: Vec<Key>,
}

/// A key press, release or clipboard chord that the shortcut dispatcher may match, with the press that the
/// intents may have consumed.
#[derive(Clone, Copy)]
pub(crate) struct Shortcut {
    pub(crate) input: KeyInput,
    pub(crate) phase: FirePhase,
    /// The index of the press in `Batch::presses`.
    press: Option<usize>,
    /// The press is the canvas's Space ([`Press::canvas`]): it fires its command whatever the focused stop types.
    pub(crate) canvas: bool,
    /// The input time the event arrived at when an earlier frame held it back; `None` for the pass's own input, which
    /// arrived at the pass time.
    pub(crate) at: Option<f64>,
    /// The event's place among the frame's events ([`Batch::place`]), which the editor inputs it comes between share.
    pub(crate) place: usize,
}

/// What the intent routing did with one press.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Taken {
    /// Nothing: the press is a shortcut candidate.
    No,
    /// The press became an intent, a focus move or an activation.
    Yes,
    /// With nothing focused, the window's document canvas took the press (a nudge it declares, `KeyUse::nudge_mods`,
    /// or a page move): it is the canvas's alone, as while the canvas has the focus, and no app shortcut runs with it.
    Canvas,
    /// A collection under the pointer took the key.
    Hover,
}

/// The key events the owner leaves to the app this pass, with the facts the matching needs.
#[derive(Default)]
pub(crate) struct Shortcuts {
    pub(crate) keys: Vec<Shortcut>,
    /// A text field owns the keys: plain keys wait for it.
    pub(crate) typing: bool,
    /// The text component that owns the keys leaves command chords and function keys to the app (`KeyUse::command_keys`).
    pub(crate) command_keys: bool,
    /// A dialog owns the keys: commands that do not fire in dialogs wait.
    pub(crate) dialog: bool,
    /// The layout that reads the presses.
    pub(crate) layout: Layout,
    /// Each change of Cmd, Ctrl or Alt in the pass, with the index into `keys` (the kept list) of the first entry after it.
    pub(crate) mods: Vec<(usize, Mods)>,
    /// The pass's input had `WindowFocused(false)`.
    pub(crate) focus_lost: bool,
    /// The content of a paste no focused component lists: the Paste command the keys fire carries it.
    pub(crate) pasted: Option<ClipContent>,
}

/// What the input hook recorded for the next pass's begin hook.
#[derive(Default)]
struct Batch {
    presses: Vec<Press>,
    /// How many key presses the strip left in egui's input.
    egui_presses: usize,
    shortcuts: Vec<Shortcut>,
    /// The content of the paste the focused component takes.
    paste: Option<ClipContent>,
    /// The content of a paste no focused component lists, for the command its chord fires (`Fired::pasted`).
    app_paste: Option<ClipContent>,
    /// Every text event of the pass, in order.
    texts: Vec<Typed>,
    /// How many text events the strip left in egui's input.
    egui_texts: usize,
    /// Each clipboard command of the pass, with its place among the frame's events.
    clips: Vec<(EditCommand, usize)>,
    access: Vec<Action>,
    escapes: Vec<EscapePress>,
    claimed: Vec<Chord>,
    nav: NavKeys,
    menu: Vec<MenuKey>,
    /// A lone Alt tap ended in this pass and a tool claimed it.
    alt_tap: bool,
    /// Each change of Cmd, Ctrl or Alt in the pass, with the index into `shortcuts` of the first entry after it.
    mods: Vec<(usize, Mods)>,
    /// The pass's input had `WindowFocused(false)`.
    focus_lost: bool,
    /// The latest key press of the pass, which a following text event was typed by; its release or a clipboard event
    /// ends it.
    last_press: Option<LastPress>,
    /// What this pass's typed text taught: the physical key, the modifiers and the text.
    learned: Vec<(Key, Mods, String)>,
    /// The bare Tab presses the input hook took from egui: each toggles the window's panels or moves focus, which
    /// `take_nav` decides once this pass's root env is installed.
    tabs: Vec<TabPress>,
    /// A lone Alt in a field that holds drafts was refused: the input hook calls `frame::refuse`.
    refused: bool,
    /// The place of the event the strip reads now among the events the IME engine left in the frame's input: the
    /// engine places its edits by the same count (`ime::take_placed`), so an editor part reads the two in the order
    /// they arrived.
    place: usize,
}

/// One text event of the pass.
struct Typed {
    text: String,
    /// A claim took the press that typed it: the text is the claim's too, so it reaches no field.
    claimed: bool,
    /// The text's place among the text events the strip left in egui's input, `None` when the strip took it.
    in_egui: Option<usize>,
    /// The text's place among the frame's events ([`Batch::place`]).
    place: usize,
}

/// A key press a following text event may belong to.
struct LastPress {
    input: KeyInput,
    /// Its place in `Batch::presses`, when the strip recorded it there.
    pressed: Option<usize>,
    /// A claim took it: its text is the claim's too, so it reaches no field.
    claimed: bool,
}

/// An Escape press the strip took as a scope escape.
struct EscapePress {
    mods: Mods,
    /// An auto-repeat of a key that is still held.
    repeat: bool,
}

/// A bare Tab or Shift+Tab press.
struct TabPress {
    input: KeyInput,
    /// An auto-repeat of a key that is still held.
    repeat: bool,
    /// When and where among the frame's events it arrived, as a [`Shortcut`] has them.
    at: Option<f64>,
    place: usize,
}

#[derive(Clone)]
struct Item {
    intent: Intent,
    /// A physical left or right arrow: under right-to-left text it reads as the opposite logical move.
    horizontal: bool,
    /// The press or text it came from; `None` for an intent of no one event.
    source: Option<Source>,
}

/// The press or text an intent came from.
#[derive(Clone, Copy, Debug)]
struct Source {
    /// Its place among the frame's events ([`Batch::place`]).
    place: usize,
    /// Shift was held at the press; never for a text.
    shift: bool,
}

#[derive(Clone, Default)]
struct Routed {
    items: Vec<Item>,
    nudge: Option<Nudge>,
    /// The chords a recorder stop's presses spell this pass.
    chords: Vec<Chord>,
    set_value: Option<(Option<PartRef>, f64)>,
    /// A `Reset` the label step routed: it resets the whole binding, whatever part holds focus.
    whole_reset: bool,
}

#[derive(Clone, Copy)]
struct Node {
    owner: Id,
    part: Option<PartRef>,
    focusable: bool,
    /// The node's expanded state this pass, `None` when it has none.
    expanded: Option<bool>,
}

#[derive(Default)]
struct AltTrack {
    down: bool,
    /// The modifiers of the last `Modifiers` event, `None` before the first.
    last: Option<Mods>,
    other_key: bool,
    /// The right Alt key is down: text typed with it (AltGr) teaches the layout nothing.
    right: bool,
}

#[derive(Default)]
pub(crate) struct Stage {
    batch: Batch,
    routed: HashMap<Option<Id>, Routed>,
    claimed: Vec<(Id, Chord)>,
    claims: Vec<(Id, Chord)>,
    /// The tool hosts that claim the next lone Alt tap, from the pass that registered them.
    alt_claims: Vec<Id>,
    /// The claimer of the lone Alt tap that ended in this pass.
    alt_tapped: Option<Id>,
    /// Collections that accept Delete under the pointer in the next pass.
    hover_deletes: Vec<Id>,
    /// The state-dependent claims the running pass holds: those registered in the pass before.
    last_claims: Vec<(Id, Chord)>,
    /// The claimed chords fired commands stand for (`fire_claimed`), for their owners in the next pass.
    fired_claims: Vec<(Id, Chord)>,
    /// Chords a container holds for itself while focus is anywhere in the viewport (a window mode), from the
    /// pass that registered them.
    captures: Vec<(Id, Chord)>,
    menu: Vec<MenuKey>,
    layout: Layout,
    nodes: HashMap<u64, Node>,
    last_nodes: HashMap<u64, Node>,
    activated: IdMap<Id>,
    alt: AltTrack,
    /// The keys down now, as the input hook read them before it withheld any press.
    down: Vec<Key>,
    /// The input hook read a key press, typed text or a clipboard event in this pass, withheld ones included.
    keyed: bool,
    /// A key went down or up (no repeat) in this pass, withheld ones included: the pointer's stir.
    stirred: bool,
    /// The input hook read a paste with no text (the clipboard held none) in this pass.
    textless_paste: bool,
    /// How many inputs of the viewport held an event sent to the focus (`count_sent`).
    sent: u64,
    /// The key presses of this input that an input method's commit follows (`note_ime_presses`).
    ime_presses: Vec<egui::Event>,
    /// The keys whose press the strip took and whose release it is still to drop.
    consumed: Vec<Key>,
    shortcuts: Shortcuts,
    /// The paste the focused component reads through its clipboard handle.
    pasted: Option<(Id, ClipContent)>,
    /// Edit commands fired from the queue for the focused component.
    queued_edits: Vec<EditCommand>,
    /// A component read its intents in this pass, so the pass's UI is running: an edit fired from now on waits for the
    /// next pass.
    intents_read: bool,
    /// The components that asked for a context menu in this pass, with the rect it opens at.
    contexts: Vec<(Id, egui::Rect, Option<PartRef>)>,
    /// Components with an open edit run this pass: Escape cancels the run before it leaves a row's child.
    open_edits: Vec<Id>,
    last_open_edits: Vec<Id>,
    /// A modal layer was open at the previous pass.
    modal_open: bool,
    /// The keys that were down when a modal layer opened or closed: their repeats reach neither layer until a fresh
    /// press or the release.
    held_over: Vec<Key>,
    /// The modifiers of the last Escape of the pass that became a `Cancel`.
    escape_mods: Option<Mods>,
    /// The Escapes that windows opened from this one handed on (`intents::hand_on_cancel`), by their modifiers: the next
    /// pass's `Cancel`s.
    handed_cancels: Vec<Mods>,
    /// How many lent keys session members have taken (`lend`), counted over the viewport's life.
    lent_taken: u64,
    /// The pane moves that menu, palette or automation fires of `NavCommand` asked for: `true` is forward.
    fired_panes: Vec<bool>,
    /// A menu, palette or automation fire of `NavCommand::FocusMenuBar` asked for the menu bar.
    fired_menu: bool,
}

fn with<R>(ctx: &Context, viewport: ViewportId, f: impl FnOnce(&mut Stage) -> R) -> R {
    PerViewport::<Stage>::with(ctx, viewport, f)
}

fn current<R>(ctx: &Context, f: impl FnOnce(&mut Stage) -> R) -> R {
    with(ctx, crate::runtime::egui::pass::current_viewport(ctx), f)
}

pub(crate) fn register_hover_delete(ctx: &Context, id: Id) {
    crate::runtime::replay::journal(ctx, || id, |ctx, &id| register_hover_delete(ctx, id));
    current(ctx, |stage| {
        if !stage.hover_deletes.contains(&id) {
            stage.hover_deletes.push(id);
        }
    });
}
