//! The key table (spec 9.2): the one place that maps a key press to an intent and modifiers to `MoveMods`.
//! Pure functions over craft-keymap's `Key` and `Mods`, where `cmd` is Ctrl on Windows and Linux and Cmd on
//! macOS and `ctrl` is the Control key on macOS only.

use crate::intent::{Granularity, Intent, MoveMods, NavMove};
use craft_host::target::Os;
use craft_keymap::{Key, Mods};

/// What the focused stop is, as far as the table's rows differ.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct KeyRole {
    /// A value control with a reset value and no text part: Delete resets it.
    pub resettable: bool,
    /// A collection: Space is its own intent (`Intent::Space`), which the item step turns into its `SpaceAction`.
    pub space: bool,
    /// A reorderable collection: Alt+Up and Alt+Down move the lead item.
    pub reorderable: bool,
    /// A collection with `CollectionProps.rename`.
    pub rename: bool,
    /// `FocusPolicy.pages`: Ctrl+Tab switches pages.
    pub pages: bool,
    /// The component reads arrow nudges: arrows are not moves. One that deletes (a canvas control) takes only the
    /// plain Delete and Backspace; with a modifier held they are the app's.
    pub nudge: bool,
    /// Typed letters reach the component as `Intent::Text` (type-ahead).
    pub type_ahead: bool,
    /// Typed digits reach the component as `Intent::Digit`; the other typed text stays `Intent::Text`.
    pub digits: bool,
    /// A value control: Alt with an arrow is a fine move.
    pub value: bool,
    /// The stop is a text part: Alt+Left and Alt+Right keep their word-move rows.
    pub text: bool,
    /// The component acts on the arrow moves routed to it: they are not shortcut candidates.
    pub moves: bool,
    /// The component acts on Delete and Backspace (text edits, deleting a selection): they reach it as
    /// `Intent::Delete`. Any other role leaves them to the command scope chain.
    pub deletes: bool,
    /// A collection that leaves its arrow keys to the component around it (`ItemKeys::Owner`): they are no moves
    /// of this role.
    pub owner_arrows: bool,
    /// The item that Ctrl or Cmd with Home goes to: the first one that is not frozen. Zero where nothing is frozen.
    pub home_row: usize,
}

impl KeyRole {
    /// A role with no special rows.
    pub const fn new() -> KeyRole {
        KeyRole {
            resettable: false,
            space: false,
            reorderable: false,
            rename: false,
            pages: false,
            nudge: false,
            type_ahead: false,
            digits: false,
            value: false,
            text: false,
            moves: false,
            deletes: false,
            owner_arrows: false,
            home_row: 0,
        }
    }

    /// Delete resets the control.
    pub const fn resettable(mut self, on: bool) -> Self {
        self.resettable = on;
        self
    }

    /// Space is the component's own intent.
    pub const fn space(mut self, on: bool) -> Self {
        self.space = on;
        self
    }

    /// Alt+Up and Alt+Down move the lead item.
    pub const fn reorderable(mut self, on: bool) -> Self {
        self.reorderable = on;
        self
    }

    /// Rename has its key.
    pub const fn rename(mut self, on: bool) -> Self {
        self.rename = on;
        self
    }

    /// Ctrl+Tab switches pages.
    pub const fn pages(mut self, on: bool) -> Self {
        self.pages = on;
        self
    }

    /// Arrows are nudges, not moves.
    pub const fn nudge(mut self, on: bool) -> Self {
        self.nudge = on;
        self
    }

    /// Typed letters are delivered as text.
    pub const fn type_ahead(mut self, on: bool) -> Self {
        self.type_ahead = on;
        self
    }

    /// Typed digits are delivered as digits.
    pub const fn digits(mut self, on: bool) -> Self {
        self.digits = on;
        self
    }

    /// Alt with an arrow is a fine move.
    pub const fn value(mut self, on: bool) -> Self {
        self.value = on;
        self
    }

    /// The component acts on the moves routed to it.
    pub const fn moves(mut self, on: bool) -> Self {
        self.moves = on;
        self
    }

    /// Delete and Backspace are the component's own.
    pub const fn deletes(mut self, on: bool) -> Self {
        self.deletes = on;
        self
    }

    /// The arrow keys are the enclosing component's, not this role's moves.
    pub const fn owner_arrows(mut self, on: bool) -> Self {
        self.owner_arrows = on;
        self
    }

    /// Ctrl or Cmd with Home goes to item `row`.
    pub const fn home_row(mut self, row: usize) -> Self {
        self.home_row = row;
        self
    }

    /// The stop is a text part.
    pub const fn text(mut self, on: bool) -> Self {
        self.text = on;
        self
    }
}

/// What a drop of a dragged payload does with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DropOperation {
    /// Move the payload.
    Move,
    /// Copy the payload (Alt, Option on macOS).
    Copy,
    /// Link to the payload (Ctrl+Shift, Cmd+Option on macOS).
    Link,
}

/// The drop operation the held modifiers pick.
pub fn drop_operation(os: Os, mods: Mods) -> DropOperation {
    let link = if os == Os::Mac { mods.cmd && mods.alt } else { mods.cmd && mods.shift };
    if link {
        DropOperation::Link
    } else if mods.alt {
        DropOperation::Copy
    } else {
        DropOperation::Move
    }
}

/// Typed `text` split for a role that takes digits: the ASCII digits it holds, in order, and the text without them.
pub fn split_digits(text: &str) -> (Vec<u8>, String) {
    let digits = text.chars().filter_map(|letter| letter.to_digit(10)).map(|digit| digit as u8).collect();
    (digits, text.chars().filter(|letter| !letter.is_ascii_digit()).collect())
}

fn is_arrow(key: Key) -> bool {
    matches!(key, Key::Up | Key::Down | Key::Left | Key::Right)
}

/// The intent a key press has for a stop of `role`, if any. `Enter` and `Space` as activation are not rows:
/// they follow the focused component's `KeyUse`.
pub fn intent(os: Os, key: Key, mods: Mods, role: KeyRole) -> Option<Intent> {
    let mac = os == Os::Mac;
    let plain = mods.is_none();
    let only = |m: Mods| mods == m;
    let shifted = |m: Mods| mods == Mods { shift: true, ..m };
    // Arrows are nudges for the content, with every modifier: Alt+arrow is no Open or Close.
    if role.nudge && is_arrow(key) {
        return None;
    }
    if !role.reorderable
        && let Some(fine) = fine_move(key, mods, role)
    {
        return Some(fine);
    }
    match key {
        Key::Delete | Key::Backspace if plain && role.resettable => return Some(Intent::Reset),
        Key::Space if plain && role.space => return Some(Intent::Space),
        Key::Menu if plain && !mac => return Some(Intent::ContextMenu),
        Key::F10 if only(Mods::SHIFT) => return Some(Intent::ContextMenu),
        Key::F2 if plain && role.rename && !mac => return Some(Intent::Rename),
        Key::Enter if plain && role.rename && mac => return Some(Intent::Rename),
        Key::Down if mac && role.rename && only(Mods::CMD) => return Some(Intent::Activate),
        Key::Enter if only(Mods::CMD) => return Some(Intent::SubmitForce),
        Key::Up if only(Mods::ALT) && role.reorderable => return Some(Intent::move_item(false)),
        Key::Down if only(Mods::ALT) && role.reorderable => return Some(Intent::move_item(true)),
        Key::Down if only(Mods::ALT) => return Some(Intent::Open),
        Key::Up if only(Mods::ALT) => return Some(Intent::Close),
        Key::PageUp if !mac && only(Mods::CMD) => return Some(Intent::switch_page(false)),
        Key::PageDown if !mac && only(Mods::CMD) => return Some(Intent::switch_page(true)),
        Key::OpenBracket | Key::OpenCurlyBracket if mac && shifted(Mods::CMD) => return Some(Intent::switch_page(false)),
        Key::CloseBracket | Key::CloseCurlyBracket if mac && shifted(Mods::CMD) => return Some(Intent::switch_page(true)),
        Key::Tab if role.pages => {
            let ctrl = if mac { Mods::CTRL } else { Mods::CMD };
            if only(ctrl) {
                return Some(Intent::switch_page(true));
            }
            if shifted(ctrl) {
                return Some(Intent::switch_page(false));
            }
        }
        _ => {}
    }
    if role.owner_arrows && is_arrow(key) {
        return None;
    }
    movement(mac, key, mods, role)
}

/// Alt with an arrow on a value role: a fine move, the same on every OS. A text role keeps Left and Right for word
/// moves, so a text value takes the fine step on Up and Down only. Shift extends, which a value reads as the small step.
fn fine_move(key: Key, mods: Mods, role: KeyRole) -> Option<Intent> {
    if !role.value || role.nudge || !mods.alt || mods.cmd || mods.ctrl {
        return None;
    }
    let mv = match key {
        Key::Up => NavMove::Up,
        Key::Down => NavMove::Down,
        Key::Left if !role.text => NavMove::Back,
        Key::Right if !role.text => NavMove::Forward,
        _ => return None,
    };
    Some(Intent::Move(mv, MoveMods::new(mods.shift, false, Granularity::Unit).fine(true)))
}

/// The move rows: arrows, Home, End and the page keys, and the deletes.
fn movement(mac: bool, key: Key, mods: Mods, role: KeyRole) -> Option<Intent> {
    let by_word = if mac { mods.alt } else { mods.cmd };
    let lead = mods.cmd;
    let moved = |mv: NavMove, by: Granularity| Some(Intent::Move(mv, MoveMods::new(mods.shift, lead, by)));
    let word_or_unit = if by_word { Granularity::Word } else { Granularity::Unit };
    // The Control key on macOS belongs to the OS (Ctrl+arrow switches spaces).
    let mac_control = mac && mods.ctrl;
    let plain_alt_free = !mods.alt || (mac && by_word);
    match key {
        Key::Left | Key::Right if mac && mods.cmd && !mods.alt => {
            moved(if key == Key::Left { NavMove::Start } else { NavMove::End }, Granularity::Line)
        }
        Key::Left if plain_alt_free && !mac_control => moved(NavMove::Back, word_or_unit),
        Key::Right if plain_alt_free && !mac_control => moved(NavMove::Forward, word_or_unit),
        Key::Up | Key::Down if mac && mods.cmd && !mods.alt => {
            moved(if key == Key::Up { NavMove::Start } else { NavMove::End }, Granularity::Document)
        }
        Key::Up if !mods.alt => moved(NavMove::Up, Granularity::Unit),
        Key::Down if !mods.alt => moved(NavMove::Down, Granularity::Unit),
        Key::Home | Key::End if !mac || !mods.cmd => {
            let by = if mods.cmd { Granularity::Document } else { Granularity::Line };
            moved(if key == Key::Home { NavMove::Start } else { NavMove::End }, by)
        }
        Key::PageUp if !mods.alt && !mods.cmd && !mods.ctrl => moved(NavMove::PageUp, Granularity::Unit),
        Key::PageDown if !mods.alt && !mods.cmd && !mods.ctrl => moved(NavMove::PageDown, Granularity::Unit),
        // A canvas control takes the plain keys; with a modifier they are the app's (Photoshop fills on Alt+Backspace
        // and Cmd+Backspace and opens Fill on Shift+Backspace). A canvas tool takes no row: it claims the deletes it
        // takes in its state (`CanvasTool::claims` in craft-ui).
        Key::Delete | Key::Backspace if role.deletes && role.nudge && !role.text => {
            mods.is_none().then(|| Intent::delete(key == Key::Delete, Granularity::Unit))
        }
        Key::Delete | Key::Backspace if role.deletes && (!mods.alt || mac) => {
            let by = if by_word { Granularity::Word } else { Granularity::Unit };
            Some(Intent::Delete { forward: key == Key::Delete, by, shift: mods.shift })
        }
        _ => None,
    }
}
