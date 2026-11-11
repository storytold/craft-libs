//! The key stage (spec 9.1, 9.2, 9.3, 9.7): the input hook strips the keys the runtime owns and records
//! every key event of the pass, the begin hook turns them into the `Intents` of the focused component,
//! and claims, menu-bar keys and AccessKit action requests are routed from the same record.

use crate::runtime::commands::FirePhase;
use crate::runtime::egui::input::Action;
use crate::runtime::frame::PerViewport;
use craft_behavior::{EditCommand, Intent, PartRef};
use craft_keymap::{Chord, Key, KeyInput, Layout, Mods};
use egui::{Context, Id, Vec2, ViewportId};
use std::collections::HashMap;
use std::sync::Arc;

mod claims;
mod input;
mod intents;
mod nodes;
mod route;

#[doc(hidden)]
pub use claims::capture;
#[doc(hidden)]
pub use claims::claim;
pub(crate) use claims::hold_escape;
#[doc(hidden)]
pub use claims::take_claimed;
#[doc(hidden)]
pub use input::down;
pub(crate) use input::{input_hook, take_menu_keys, take_nav};
pub use intents::Intents;
#[doc(hidden)]
pub use intents::intents_of;
#[doc(hidden)]
pub use intents::settle;
#[doc(hidden)]
pub use intents::take_submit;
#[doc(hidden)]
pub use intents::with_window_intents;
pub(crate) use intents::{context_request, request_context, route_whole_reset, scope_escape, take_cancel, take_window_cancel};
pub(crate) use nodes::{redirect_node, register_expanded, register_node};
pub(crate) use route::{activated_member, begin, layout_of, pasted_text, queue_activate, queue_edit, take_shortcuts};

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
}

/// A key press, release or clipboard chord that the shortcut dispatcher may match, with the press that the
/// intents may have consumed.
#[derive(Clone, Copy)]
pub(crate) struct Shortcut {
    pub(crate) input: KeyInput,
    pub(crate) phase: FirePhase,
    /// The press typed text in this pass.
    typed: bool,
    /// The index of the press in `Batch::presses`.
    press: Option<usize>,
    /// The focused widget does not act on the arrow move the press became: a command scoped to a container around
    /// the focus takes the press, an app command does not.
    pub(crate) moved: bool,
}

/// What the intent routing did with one press.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Taken {
    /// Nothing: the press is a shortcut candidate.
    No,
    /// The press became an arrow move routed to a focused stop that does not act on moves.
    Moved,
    /// The press became an intent, a focus move or an activation.
    Yes,
}

/// The key events the owner leaves to the app this pass, with the facts the matching needs.
#[derive(Default)]
pub(crate) struct Shortcuts {
    pub(crate) keys: Vec<Shortcut>,
    /// A text field owns the keys: plain keys wait for it.
    pub(crate) typing: bool,
    /// A dialog owns the keys: commands that do not fire in dialogs wait.
    pub(crate) dialog: bool,
    /// The layout that reads the presses.
    pub(crate) layout: Layout,
}

/// What the input hook recorded for the next pass's begin hook.
#[derive(Default)]
struct Batch {
    presses: Vec<Press>,
    shortcuts: Vec<Shortcut>,
    /// The text of the paste the focused component takes.
    paste: Option<String>,
    texts: Vec<String>,
    clips: Vec<EditCommand>,
    access: Vec<Action>,
    escapes: usize,
    claimed: Vec<Chord>,
    nav: NavKeys,
    menu: Vec<MenuKey>,
    /// Whether the last Enter event of the pass was a press (`true`) or a release, `None` when there was none.
    enter_down: Option<bool>,
    /// The latest key press of the pass, which a following text event was typed by.
    last_press: Option<KeyInput>,
    /// What this pass's typed text taught: the physical key, the modifiers and the text.
    learned: Vec<(Key, Mods, String)>,
    /// The bare Tab presses the input hook took from egui: each toggles the window's panels or moves focus, which
    /// `take_nav` decides once this pass's root env is installed.
    tabs: Vec<TabPress>,
}

/// A bare Tab or Shift+Tab press.
struct TabPress {
    input: KeyInput,
    /// An auto-repeat of a key that is still held.
    repeat: bool,
}

#[derive(Clone)]
struct Item {
    intent: Intent,
    /// A physical left or right arrow: under right-to-left text it reads as the opposite logical move.
    horizontal: bool,
}

#[derive(Clone, Default)]
struct Routed {
    items: Vec<Item>,
    nudge: Option<(Vec2, Mods)>,
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
    other_key: bool,
    /// The right Alt key is down: text typed with it (AltGr) teaches the layout nothing.
    right: bool,
}

#[derive(Default)]
struct Stage {
    batch: Batch,
    /// Controls a label click activates, queued by `queue_activate` for the next pass.
    activate: Vec<Id>,
    routed: HashMap<Option<Id>, Routed>,
    claimed: Vec<(Id, Chord)>,
    claims: Vec<(Id, Chord)>,
    /// Chords a container holds for itself while focus is anywhere in the viewport (a window mode), from the
    /// pass that registered them.
    captures: Vec<(Id, Chord)>,
    menu: Vec<MenuKey>,
    layout: Layout,
    nodes: HashMap<u64, Node>,
    last_nodes: HashMap<u64, Node>,
    activated: HashMap<Id, Id>,
    alt: AltTrack,
    /// The keys down now, as the input hook read them before it withheld any press.
    down: Vec<Key>,
    /// The keys whose press the strip took and whose release it is still to drop.
    consumed: Vec<Key>,
    shortcuts: Shortcuts,
    /// The paste text the focused component reads through its clipboard handle.
    pasted: Option<(Id, Arc<str>)>,
    /// Edit commands fired from the queue for the focused component.
    queued_edits: Vec<EditCommand>,
    /// The components that asked for a context menu in this pass, with the rect it opens at.
    contexts: Vec<(Id, egui::Rect)>,
    /// Components with an open edit run this pass: Escape cancels the run before it leaves a row's child.
    open_edits: Vec<Id>,
    last_open_edits: Vec<Id>,
    /// Enter is held down: its last event was a press.
    enter_down: bool,
    /// A modal layer was open at the previous pass.
    modal_open: bool,
    /// The held Enter began before the open modal layer: its repeats reach nobody until a fresh press.
    enter_stale: bool,
}

fn with<R>(ctx: &Context, viewport: ViewportId, f: impl FnOnce(&mut Stage) -> R) -> R {
    PerViewport::<Stage>::with(ctx, viewport, f)
}

fn current<R>(ctx: &Context, f: impl FnOnce(&mut Stage) -> R) -> R {
    with(ctx, crate::runtime::egui::pass::current_viewport(ctx), f)
}
