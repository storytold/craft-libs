//! Focus scopes, key use and the programmatic focus target (spec 9.3, 9.4).

use craft_behavior::{CancelRule, DocKey, EditCommand, Input, OwnerClass, PartRef, ScopeKind};
use craft_keymap::{Chord, Mods};
use egui::Id;

/// Where focus goes the first time a scope is pushed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Initial {
    /// Focus stays where it is.
    Keep,
    /// The scope's first stop.
    First,
    /// The scope's first text or number field, else its default stop, else nothing: a dialog's default.
    FirstField,
    /// The stop registered as the default.
    Default,
    /// The stop registered under this name.
    Key(&'static str),
    /// Focus stays; a collection keeps its anchor.
    KeepAnchor,
}

/// A one-shot request for focus on a component's stop (`Props::request_focus`), for what opens without a layer of its
/// own (an inline editor, a view's search field): a layer names its stop with `Initial` instead. The component
/// sends it in the one pass it asks for focus, and the focus service takes it there once the component's stops
/// are registered; the press that opened the component then leaves the focus where the request put it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FocusRequest {
    /// The component's stop takes the focus.
    Take,
    /// The component's stop takes the focus when nothing holds it.
    WhenNone,
}

/// What Enter does in a scope after the focused component declined it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EnterRule {
    /// Nothing: an untaken `Submit` goes to the next scope outward.
    Pass,
    /// The scope has a default action, which takes an untaken `Submit`.
    Submit,
}

/// A focus scope's rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FocusPolicy {
    /// What the scope is.
    pub kind: ScopeKind,
    /// Tab wraps inside the scope and cannot leave it.
    pub trap: bool,
    /// Focus returns to the opener when the scope disappears. In a scope that does not trap, the opener is the stop
    /// focus entered the scope from.
    pub restore: bool,
    /// Where focus goes when the scope first appears.
    pub initial: Initial,
    /// What Enter does after the focused component declined it.
    pub enter: EnterRule,
    /// What Escape does when nothing inside takes it.
    pub cancel: CancelRule,
    /// Ctrl+Tab cycles pages.
    pub pages: bool,
    /// In a `KeepAnchor` layer, Enter on a text opener activates the layer's list; off, the opener keeps Enter and
    /// Tab alone takes the list's lead (a completion list that leaves Enter to the field's own commit).
    pub list_enter: bool,
    /// In a `KeepAnchor` layer, the collection the opener hands its list keys to (Up, Down, Page keys, Home and End,
    /// Enter and Tab as the layer's rules say). Without one the opener keeps its keys.
    pub list: Option<Id>,
    /// In a dialog whose Escape closes it, the Escape keeps the typing in the focused field: the field commits its
    /// draft as on leaving it, and the dialog then closes. Off (the default), one Escape cancels the typing with the
    /// dialog. A window that applies its rows as they change (a live Preferences window) keeps what was typed.
    pub escape_keeps_typing: bool,
}

impl FocusPolicy {
    /// A scope that changes nothing.
    pub const NONE: FocusPolicy = FocusPolicy {
        kind: ScopeKind::Group,
        trap: false,
        restore: false,
        initial: Initial::Keep,
        enter: EnterRule::Pass,
        cancel: CancelRule::Ignore,
        pages: false,
        list_enter: true,
        list: None,
        escape_keeps_typing: false,
    };
    /// A pane: an F6 stop.
    pub const PANE: FocusPolicy = FocusPolicy { kind: ScopeKind::Pane, ..FocusPolicy::NONE };
    /// A dialog: Tab stays inside, Enter submits, Escape closes.
    pub const DIALOG: FocusPolicy = FocusPolicy {
        kind: ScopeKind::Dialog,
        trap: true,
        restore: true,
        initial: Initial::FirstField,
        enter: EnterRule::Submit,
        cancel: CancelRule::Close,
        pages: false,
        list_enter: true,
        list: None,
        escape_keeps_typing: false,
    };
    /// A popup: focus moves in, Escape closes and focus returns to the opener.
    pub const POPUP: FocusPolicy = FocusPolicy {
        kind: ScopeKind::Popup,
        trap: true,
        restore: true,
        initial: Initial::First,
        enter: EnterRule::Pass,
        cancel: CancelRule::Close,
        pages: false,
        list_enter: true,
        list: None,
        escape_keeps_typing: false,
    };
    /// A menu: like a popup.
    pub const MENU: FocusPolicy = FocusPolicy { kind: ScopeKind::Menu, ..FocusPolicy::POPUP };
    /// A layer that only Tab reaches, such as the toasts: it traps nothing and takes no focus on its own, and when it
    /// goes away with focus inside, focus returns to the stop it came from.
    pub const TAB_LAYER: FocusPolicy = FocusPolicy { restore: true, ..FocusPolicy::NONE };

    /// Whether Tab wraps inside the scope.
    pub const fn trap(mut self, on: bool) -> Self {
        self.trap = on;
        self
    }

    /// Whether focus returns to the opener.
    pub const fn restore(mut self, on: bool) -> Self {
        self.restore = on;
        self
    }

    /// Where focus goes first.
    pub const fn initial(mut self, initial: Initial) -> Self {
        self.initial = initial;
        self
    }

    /// What Escape does.
    pub const fn cancel(mut self, cancel: CancelRule) -> Self {
        self.cancel = cancel;
        self
    }

    /// Whether Enter on a text opener activates the layer's list.
    pub const fn list_enter(mut self, on: bool) -> Self {
        self.list_enter = on;
        self
    }

    /// The collection in the layer that takes the opener's list keys.
    pub const fn list(mut self, list: Id) -> Self {
        self.list = Some(list);
        self
    }

    /// Whether Escape in this dialog keeps the focused field's typing ([`FocusPolicy::escape_keeps_typing`]).
    pub const fn escape_keeps_typing(mut self, on: bool) -> Self {
        self.escape_keeps_typing = on;
        self
    }

    /// Whether Ctrl+Tab cycles pages.
    pub const fn pages(mut self, on: bool) -> Self {
        self.pages = on;
        self
    }
}

impl Default for FocusPolicy {
    fn default() -> Self {
        FocusPolicy::NONE
    }
}

/// What `runtime::focus::request` focuses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FocusTarget {
    /// A component.
    Id(Id),
    /// A focusable part of a component.
    Part(Id, PartRef),
    /// The component registered under a document key.
    Doc(DocKey),
}

/// The keys a focused component keeps from the key stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeepKeys {
    /// None.
    None,
    /// Tab.
    Tab,
    /// Left and right arrows.
    InlineArrows,
    /// All four arrows.
    BlockArrows,
}

impl KeepKeys {
    /// Whether a focused component with these keys keeps the arrow move `mv` for itself, so a roving group around it
    /// does not take it.
    pub(crate) fn keeps_arrow(self, mv: craft_behavior::NavMove) -> bool {
        use craft_behavior::NavMove;
        match self {
            KeepKeys::BlockArrows => matches!(mv, NavMove::Up | NavMove::Down | NavMove::Back | NavMove::Forward),
            KeepKeys::InlineArrows => matches!(mv, NavMove::Back | NavMove::Forward),
            KeepKeys::None | KeepKeys::Tab => false,
        }
    }
}

/// The keys that activate a component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ActivateKeys {
    /// Neither.
    Neither,
    /// Enter.
    Enter,
    /// Space.
    Space,
    /// Both.
    Both,
}

impl ActivateKeys {
    /// Whether Enter activates.
    pub const fn enter(self) -> bool {
        matches!(self, ActivateKeys::Enter | ActivateKeys::Both)
    }

    /// Whether Space activates.
    pub const fn space(self) -> bool {
        matches!(self, ActivateKeys::Space | ActivateKeys::Both)
    }
}

/// The edit commands a component takes as `Intent::Edit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditSet(u8);

impl EditSet {
    /// No command.
    pub const NONE: EditSet = EditSet(0);
    /// Every command.
    pub const ALL: EditSet = EditSet(0b111_1111);

    const fn bit(command: EditCommand) -> u8 {
        1 << match command {
            EditCommand::Cut => 0,
            EditCommand::Copy => 1,
            EditCommand::Paste => 2,
            EditCommand::SelectAll => 3,
            EditCommand::Undo => 4,
            EditCommand::Delete => 6,
            _ => 5,
        }
    }

    /// The set plus `command`.
    pub const fn with(self, command: EditCommand) -> EditSet {
        EditSet(self.0 | Self::bit(command))
    }

    /// Whether the set has `command`.
    pub const fn contains(self, command: EditCommand) -> bool {
        self.0 & Self::bit(command) != 0
    }
}

/// What a bare End means for a focused component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EndKey {
    /// The last item of the component's content (a list's last row, a text field's line end).
    LastItem,
    /// A sheet's End mode: End arms the next arrow, which jumps to the edge of the data region. Ctrl/Cmd+End goes to
    /// the last used cell.
    EndMode,
}

/// A component's declared keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct KeyUse {
    /// The keys kept from the key stage.
    pub keep: KeepKeys,
    /// The keys that activate.
    pub activate: ActivateKeys,
    /// Chords kept from the shortcut dispatcher, and from Alt+numpad code entry, while focused, and for the window's
    /// document canvas also while nothing has the focus. An app's own egui widget (`focus::host_canvas`) reads their
    /// presses from egui's input; a framework component reads them as claimed chords.
    pub claims: &'static [Chord],
    /// Edit commands delivered as `Intent::Edit`.
    pub edits: EditSet,
    /// Edit commands in `edits` that do nothing now (Cut and Copy with no selection): their keys and menu picks still
    /// reach the component, so they never fall through to the commands around it, and the edit menu shows them
    /// disabled.
    pub inactive: EditSet,
    /// The component's key class.
    pub owner: OwnerClass,
    /// The component's arrows step a value: Alt with an arrow is a fine move. Set per instance by a component that
    /// keeps Alt+Down for something else when it has a dropdown.
    pub value: bool,
    /// The component acts on the arrow moves routed to it (`Intent::Move`: it takes them with `take_move`), so a
    /// shortcut never shares them. A control that does not (a button) leaves them to the commands around the focus.
    pub moves: bool,
    /// The component is a document canvas: a window that registers one this pass shows a document, and a bare Tab
    /// toggles that window's panels with nothing focused or the canvas focused (`window.togglePanels`). The page keys'
    /// document role comes from the canvas's view instead (`ContainerProps::scroll_owner`), for its body's stops too.
    pub chrome_key: bool,
    /// The arrows are nudges for the component's content (a canvas tool or control), not moves: they arrive as
    /// `Intents::take_nudge`.
    pub nudge: bool,
    /// The modifiers a document canvas (`chrome_key`) acts on with the arrows: while nothing in the window has the
    /// focus, an arrow pressed with one of them is the canvas's nudge alone, and an arrow with any other is no nudge and
    /// stays the app's shortcut (Alt+Left of a view history). A canvas lists what its tool and body act on.
    pub nudge_mods: NudgeMods,
    /// The component acts on Delete and Backspace (`Intent::Delete`: text edits, deleting a selection or a canvas
    /// control's object). A component that does not leaves them to the commands around the focus; a canvas tool claims
    /// them instead (`CanvasTool::claims`).
    pub deletes: bool,
    /// The component acts on Enter itself (a text field that saves or searches on it, a multi-line field that makes it a
    /// line break): it takes the `Submit` intent, so the enclosing dialog's default button does not receive it. A
    /// component that does not leaves Enter to that button.
    pub submits: bool,
    /// What a bare End means for the component.
    pub end: EndKey,
    /// The component is an arena member that holds its arena's edit session (a crop frame, a transform box): while
    /// none of its parts has focus it takes the Enter, Escape and arrows routed to its arena root (the canvas) and,
    /// with nothing in the window focused, the window's, before the arena's tool. What it leaves goes on.
    pub session: bool,
    /// Typed letters reach the component as `Intent::Text` (`take_text`): a closed select that opens its list on the
    /// first entry they begin.
    pub type_ahead: bool,
    /// The component is a text field whose draft a press on a part declared `holds_drafts`, or on a popup layer above
    /// it, leaves unapplied; a lone Alt in the field only alerts and a window deactivation does not commit the draft.
    pub holds_drafts: bool,
    /// While the component types, chords with the platform's command modifier and function keys still run the app's
    /// commands whatever the app's typing rule, as under `TypingRule::CommandKeysOnly`; the text editing chords stay
    /// the component's. For a canvas tool's text editor whose app runs its commands while typing.
    pub command_keys: bool,
}

/// Which Tab stops a dialog's Tab order visits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TabReach {
    /// The window's rule (`RootEnv::tab_all_controls`).
    #[default]
    Root,
    /// Only text stops, on every OS, wrapping inside the dialog's trap; with no text stop Tab does nothing.
    Fields,
}

impl KeyUse {
    /// A multi-line text field: Up and Down move the caret.
    pub const TEXT_MULTILINE: KeyUse = KeyUse::of(Input::Text).with_keep(KeepKeys::BlockArrows);
    /// A code editor: Tab inserts.
    pub const CODE: KeyUse = KeyUse::of(Input::Editor).with_keep(KeepKeys::Tab);
    /// A document canvas: all arrows, no activation, and the key that marks its window as showing a document.
    pub const CANVAS: KeyUse = KeyUse {
        keep: KeepKeys::BlockArrows,
        activate: ActivateKeys::Neither,
        claims: &[],
        edits: EditSet::NONE,
        inactive: EditSet::NONE,
        owner: OwnerClass::Widget,
        value: false,
        moves: false,
        chrome_key: true,
        nudge: true,
        nudge_mods: NudgeMods::STEPS,
        deletes: false,
        submits: false,
        end: EndKey::LastItem,
        session: false,
        type_ahead: false,
        holds_drafts: false,
        command_keys: false,
    };
    /// A divider between panes: it takes the arrows as moves of its value, but it is no document and no canvas tool, so
    /// it marks no window as showing a document and takes no nudges.
    pub const SPLITTER: KeyUse = KeyUse { chrome_key: false, nudge: false, ..KeyUse::CANVAS.with_moves() };
    /// A shortcut recorder: it takes the arrows of a canvas, but it is no document, so it marks no window and a Tab in
    /// it never toggles panels.
    pub const RECORDER: KeyUse = KeyUse { owner: OwnerClass::Recorder, chrome_key: false, nudge: false, ..KeyUse::CANVAS };

    /// The default key use of a component that takes `input`. The only place egui key filters start from.
    pub const fn of(input: Input) -> KeyUse {
        let (keep, activate, owner) = match input {
            Input::Pointer => (KeepKeys::None, ActivateKeys::Both, OwnerClass::Widget),
            Input::Text => (KeepKeys::InlineArrows, ActivateKeys::Neither, OwnerClass::Text),
            Input::Editor => (KeepKeys::BlockArrows, ActivateKeys::Neither, OwnerClass::Text),
            _ => (KeepKeys::None, ActivateKeys::Neither, OwnerClass::Widget),
        };
        // Text parts list every edit command (11.3); editors every one but Delete, whose run (a menu row) stays the
        // app's command while an editor has the focus.
        let edits = match input {
            Input::Text => EditSet::ALL,
            Input::Editor => EditSet(EditSet::ALL.0 & !EditSet::bit(EditCommand::Delete)),
            _ => EditSet::NONE,
        };
        let owner_text = matches!(owner, OwnerClass::Text);
        KeyUse {
            keep,
            activate,
            claims: &[],
            edits,
            inactive: EditSet::NONE,
            owner,
            value: false,
            moves: false,
            chrome_key: false,
            nudge: false,
            nudge_mods: NudgeMods::NONE,
            deletes: owner_text,
            submits: false,
            end: EndKey::LastItem,
            session: false,
            type_ahead: false,
            holds_drafts: false,
            command_keys: false,
        }
    }

    /// The same use declaring that a press that would apply the focused draft leaves it unapplied (`holds_drafts`).
    pub const fn with_holds_drafts(mut self) -> Self {
        self.holds_drafts = true;
        self
    }

    /// The same use letting command chords and function keys run the app's commands while it types (`command_keys`).
    pub const fn with_command_keys(mut self) -> Self {
        self.command_keys = true;
        self
    }

    /// The same use taking typed letters as `Intent::Text`.
    pub const fn with_type_ahead(mut self) -> Self {
        self.type_ahead = true;
        self
    }

    /// The same use claiming `claims`.
    pub const fn with_claims(mut self, claims: &'static [Chord]) -> Self {
        self.claims = claims;
        self
    }

    /// The same use listing `edits`.
    pub const fn with_edits(mut self, edits: EditSet) -> Self {
        self.edits = edits;
        self
    }

    /// The same use marking `inactive` of its edit commands as doing nothing now (`KeyUse::inactive`).
    pub const fn with_inactive(mut self, inactive: EditSet) -> Self {
        self.inactive = inactive;
        self
    }

    /// The same use with `activate` as the keys that activate: a value control that arrow and Page keys step takes none.
    pub const fn with_activate(mut self, activate: ActivateKeys) -> Self {
        self.activate = activate;
        self
    }

    /// The same use with `value` as whether its arrows step a value.
    pub const fn with_value(mut self, value: bool) -> Self {
        self.value = value;
        self
    }

    /// The same use declaring that the component acts on the moves routed to it.
    pub const fn with_moves(mut self) -> Self {
        self.moves = true;
        self
    }

    /// The same use declaring that the component acts on Delete and Backspace.
    pub const fn with_deletes(mut self) -> Self {
        self.deletes = true;
        self
    }

    /// The same use declaring that the component holds its arena's edit session (`session`).
    pub const fn with_session(mut self) -> Self {
        self.session = true;
        self
    }

    /// The same use declaring that the component acts on Enter and takes `Intent::Submit`.
    pub const fn with_submits(mut self) -> Self {
        self.submits = true;
        self
    }

    /// The same use with `end` as the meaning of a bare End.
    pub const fn with_end(mut self, end: EndKey) -> Self {
        self.end = end;
        self
    }

    /// The same use with `keep` as the keys it keeps from the key stage.
    pub const fn with_keep(mut self, keep: KeepKeys) -> Self {
        self.keep = keep;
        self
    }

    /// The same use with `chrome_key` as whether the component marks its window as showing a document: a canvas
    /// control that is no document sets it to `false`.
    pub const fn with_chrome_key(mut self, chrome_key: bool) -> Self {
        self.chrome_key = chrome_key;
        self
    }

    /// The same use with `nudge_mods` as the modifiers a document canvas acts on with the arrows.
    pub const fn with_nudge_mods(mut self, nudge_mods: NudgeMods) -> Self {
        self.nudge_mods = nudge_mods;
        self
    }

    /// Whether the component is a document canvas's editor part: the document (`chrome_key`) holding its editor's
    /// text keys. It types letters as a text field does, but it is the document the window's menu keys act over, not
    /// a field inside the chrome.
    pub const fn is_document_editor(&self) -> bool {
        self.chrome_key && matches!(self.owner, OwnerClass::Text)
    }

    /// Whether the component claims `chord`.
    pub fn claims_chord(&self, chord: &Chord) -> bool {
        self.claims.contains(chord)
    }
}

/// A set of modifier combinations held with the arrows (`KeyUse::nudge_mods`): each is matched exactly, so Shift alone
/// does not admit Shift with Alt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NudgeMods(u16);

impl NudgeMods {
    /// No arrow.
    pub const NONE: NudgeMods = NudgeMods(0);
    /// The plain arrows and the arrows with Shift: the step and the large step every nudge reads.
    pub const STEPS: NudgeMods = NudgeMods::NONE.with(Mods::NONE).with(Mods::SHIFT);

    /// The same set with `mods` in it.
    #[must_use]
    pub const fn with(self, mods: Mods) -> Self {
        NudgeMods(self.0 | Self::bit(mods))
    }

    /// Every combination of either set.
    #[must_use]
    pub const fn union(self, other: NudgeMods) -> Self {
        NudgeMods(self.0 | other.0)
    }

    /// Whether `mods` is in the set.
    pub const fn contains(self, mods: Mods) -> bool {
        self.0 & Self::bit(mods) != 0
    }

    const fn bit(mods: Mods) -> u16 {
        let at = (mods.cmd as u16) | ((mods.ctrl as u16) << 1) | ((mods.alt as u16) << 2) | ((mods.shift as u16) << 3);
        1 << at
    }
}
