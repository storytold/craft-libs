//! The model behind a shortcut editor: an edit session with undo, a chord recorder, conflict
//! queries and search.
//!
//! A shortcut dialog drives it like this:
//!
//! ```
//! use craft_keymap::editor::holders;
//! use craft_keymap::{AssignError, Bindable, Binding, Chord, EditSession, Keymap, Platform, Policy, Recorded, Recorder, Scope, Scopes};
//!
//! let catalog = vec![
//!     Bindable::new("file.save", "Save", Scope::app()).with_default(Binding::parse("Cmd+S").unwrap()),
//!     Bindable::new("edit.copy", "Copy", Scope::app()).with_default(Binding::parse("Cmd+C").unwrap()),
//! ];
//! let mut keymap = Keymap::new(catalog, Scopes::new(), Platform::Mac);
//! let now = 12.5;
//!
//! let mut session = EditSession::begin(&keymap);
//! let mut recorder = Recorder::new();
//! let save = "file.save".into();
//! // The user presses Cmd+C in the shortcut field of Save.
//! if let Recorded::Single(trigger) = recorder.press(Chord::parse("Cmd+C").unwrap(), now) {
//!     let binding = Binding::new(trigger);
//!     assert_eq!(holders(&keymap, &save, &binding)[0].command, "edit.copy".into());
//!     if let Err(AssignError::Taken(_)) = session.assign(&mut keymap, &save, binding.clone(), Policy::Refuse) {
//!         // The user chose to take the key from Copy.
//!         session.assign(&mut keymap, &save, binding, Policy::Steal).ok();
//!     }
//! }
//! assert!(keymap.bindings(&"edit.copy".into()).is_empty());
//! // OK: store `keymap.overrides()` (`Profiles::store` for sets). Cancel:
//! session.cancel(&mut keymap);
//! assert_eq!(keymap.bindings(&"edit.copy".into()), [Binding::parse("Cmd+C").unwrap()]);
//! ```

use std::collections::VecDeque;

use crate::chord::{Chord, Key, Platform, Trigger};
use crate::command::CommandRef;
use crate::keymap::{AssignError, Assigned, Bindable, Binding, Holder, Keymap, Overrides, Policy};

/// Undo steps an [`EditSession`] keeps; older ones are dropped.
pub const MAX_UNDO: usize = 500;
/// Most bytes of undo snapshots an [`EditSession`] keeps (keys and texts, plus a fixed estimate
/// per entry and per text for their allocations); older steps are dropped, though the latest one
/// always stays.
pub const MAX_UNDO_BYTES: usize = 64 << 20;

/// Seconds within which a second press of the same chord counts as a double press.
pub const DOUBLE_PRESS: f64 = 0.6;

/// One opening of a shortcut editor: the overrides it started from, and undo and redo as
/// snapshots of [`Overrides`].
#[derive(Clone, Debug)]
pub struct EditSession {
    start: Overrides,
    /// Snapshots with their [`size`].
    undo: VecDeque<(Overrides, usize)>,
    undo_bytes: usize,
    redo: Vec<Overrides>,
}

/// Heap bytes a snapshot entry costs beyond its strings' bytes: the key's `String`, the list and
/// the map node, estimated.
const ENTRY_OVERHEAD: usize = size_of::<String>() + 64;
/// Bytes a text costs beyond its own: its `String` and allocator rounding, estimated.
const TEXT_OVERHEAD: usize = size_of::<String>() + 16;

/// The estimated bytes of a snapshot, so many short texts count as more than their length.
fn size(o: &Overrides) -> usize {
    o.iter().map(|(k, texts)| ENTRY_OVERHEAD + k.len() + texts.iter().map(|t| TEXT_OVERHEAD + t.len()).sum::<usize>()).sum()
}

impl EditSession {
    /// Starts a session on `keymap`'s current overrides; [`cancel`](Self::cancel) returns to them.
    pub fn begin(keymap: &Keymap) -> EditSession {
        EditSession { start: keymap.overrides().clone(), undo: VecDeque::new(), undo_bytes: 0, redo: Vec::new() }
    }

    /// Runs one edit. When it changes the overrides it becomes an undo step and clears redo.
    pub fn edit<R>(&mut self, keymap: &mut Keymap, f: impl FnOnce(&mut Keymap) -> R) -> R {
        let before = keymap.overrides().clone();
        let out = f(keymap);
        if *keymap.overrides() != before {
            self.push_undo(before);
            self.redo.clear();
        }
        out
    }

    /// Adds a binding as one undo step. With [`Policy::Refuse`] a chord other commands hold gives
    /// [`AssignError::Taken`] and changes nothing, so the dialog can ask whether to steal or keep;
    /// the answer is a second call with [`Policy::Steal`] or [`Policy::Keep`].
    pub fn assign(&mut self, keymap: &mut Keymap, command: &CommandRef, binding: Binding, policy: Policy) -> Result<Assigned, AssignError> {
        self.edit(keymap, |km| km.assign(command, binding, policy))
    }

    /// Replaces one binding of a command with another as one undo step, as when the user records
    /// over a key in the list. Conflicts follow `policy` as in [`EditSession::assign`].
    pub fn replace(
        &mut self,
        keymap: &mut Keymap,
        command: &CommandRef,
        old: &Binding,
        new: Binding,
        policy: Policy,
    ) -> Result<Assigned, AssignError> {
        let list: Vec<Binding> = keymap.bindings(command).into_iter().map(|b| if b == *old { new.clone() } else { b }).collect();
        self.edit(keymap, |km| km.set_bindings(command, list, policy))
    }

    /// Removes one binding of a command as one undo step.
    pub fn remove(&mut self, keymap: &mut Keymap, command: &CommandRef, binding: &Binding) -> bool {
        self.edit(keymap, |km| km.remove(command, binding))
    }

    /// Leaves a command without bindings as one undo step.
    pub fn clear(&mut self, keymap: &mut Keymap, command: &CommandRef) -> bool {
        self.edit(keymap, |km| km.clear(command))
    }

    /// Gives a command its default bindings back as one undo step.
    pub fn reset(&mut self, keymap: &mut Keymap, command: &CommandRef) -> bool {
        self.edit(keymap, |km| km.reset(command))
    }

    /// Gives every command its defaults as one undo step.
    pub fn reset_all(&mut self, keymap: &mut Keymap) {
        self.edit(keymap, Keymap::reset_all);
    }

    fn push_undo(&mut self, snapshot: Overrides) {
        let bytes = size(&snapshot);
        self.undo.push_back((snapshot, bytes));
        self.undo_bytes += bytes;
        while self.undo.len() > 1 && (self.undo.len() > MAX_UNDO || self.undo_bytes > MAX_UNDO_BYTES) {
            if let Some((_, b)) = self.undo.pop_front() {
                self.undo_bytes -= b;
            }
        }
    }

    /// Reverts the last edit; false when there is none.
    pub fn undo(&mut self, keymap: &mut Keymap) -> bool {
        let Some((previous, bytes)) = self.undo.pop_back() else { return false };
        self.undo_bytes -= bytes;
        self.redo.push(keymap.overrides().clone());
        keymap.set_overrides(previous);
        true
    }

    /// Applies the last undone edit again; false when there is none.
    pub fn redo(&mut self, keymap: &mut Keymap) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        self.push_undo(keymap.overrides().clone());
        keymap.set_overrides(next);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Restores the overrides the session started with and forgets its undo history.
    pub fn cancel(&mut self, keymap: &mut Keymap) {
        keymap.set_overrides(self.start.clone());
        self.undo.clear();
        self.undo_bytes = 0;
        self.redo.clear();
    }

    /// Whether `keymap`'s overrides differ from those the session started with.
    pub fn is_modified(&self, keymap: &Keymap) -> bool {
        *keymap.overrides() != self.start
    }
}

/// The other commands whose bindings clash with `binding` given to `command`: what
/// [`Policy::Steal`] would take and [`Policy::Keep`] would leave clashing. Tools of the command's
/// group and twins sharing a default key are not listed. A dialog shows these while the user
/// records, before anything is assigned.
pub fn holders(keymap: &Keymap, command: &CommandRef, binding: &Binding) -> Vec<Holder> {
    keymap.clashing(command, std::slice::from_ref(binding))
}

/// What a press means to the recorder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recorded {
    Single(Trigger),
    /// The chord just recorded, pressed again within [`DOUBLE_PRESS`]: it replaces that single.
    Double(Trigger),
    /// Escape without modifiers: stop recording.
    Cancel,
}

/// Turns the presses of a shortcut field into triggers.
#[derive(Clone, Debug, Default)]
pub struct Recorder {
    last: Option<(Chord, f64)>,
}

impl Recorder {
    pub fn new() -> Recorder {
        Recorder::default()
    }

    /// Records a press at `now_secs` (any monotonic clock, such as egui's `input.time`).
    pub fn press(&mut self, chord: Chord, now_secs: f64) -> Recorded {
        if chord.key == Key::Escape && chord.mods.is_none() {
            self.last = None;
            return Recorded::Cancel;
        }
        // After a double the next press starts over, so a third press is a single again.
        if let Some((last, at)) = self.last.take()
            && last == chord
            && now_secs >= at
            && now_secs - at < DOUBLE_PRESS
        {
            return Recorded::Double(Trigger::double(chord));
        }
        self.last = Some((chord, now_secs));
        Recorded::Single(Trigger::single(chord))
    }

    /// Forgets the last press, so the next one cannot become a double.
    pub fn reset(&mut self) {
        self.last = None;
    }
}

/// The bindables matching `query`, in catalog order: a case-insensitive substring of the label,
/// an alias, a menu path segment, the command id, or a binding's canonical or `platform` display
/// text. An empty query matches everything.
pub fn search<'a>(keymap: &'a Keymap, query: &str, platform: Platform) -> Vec<&'a Bindable> {
    let q = query.trim().to_lowercase();
    keymap
        .catalog()
        .iter()
        .filter(|b| {
            if q.is_empty() {
                return true;
            }
            let hit = |s: &str| s.to_lowercase().contains(&q);
            hit(&b.label)
                || b.aliases.iter().any(|a| hit(a))
                || b.paths.iter().flatten().any(|s| hit(s))
                || hit(&b.command.id)
                || keymap.bindings(&b.command).iter().any(|binding| hit(&binding.trigger.to_string()) || hit(&binding.trigger.display(platform)))
        })
        .collect()
}
