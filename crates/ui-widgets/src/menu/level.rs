//! MenuLevel: the rows of one menu level, their mnemonics, the active row and the open submenu.

use super::popup::MenuStep;
use crate::contract::{Component, Cx, Remember as _};
use crate::runtime::env::{AppValues, EnvValue};
use craft_keymap::mnemonic;
use egui::{Id, Pos2, Rect, pos2};
use std::sync::{Arc, Mutex, PoisonError};

/// Whether the pointer at `now`, having come from `from`, heads for `submenu`: it lies inside the triangle from
/// `from` to the submenu's edge nearer to it.
#[doc(hidden)]
pub fn heads_toward(from: Pos2, now: Pos2, submenu: Rect) -> bool {
    let near = if from.x <= submenu.center().x { submenu.min.x } else { submenu.max.x };
    let (a, b) = (pos2(near, submenu.min.y), pos2(near, submenu.max.y));
    let side = |p: Pos2, q: Pos2, r: Pos2| (p.x - r.x) * (q.y - r.y) - (q.x - r.x) * (p.y - r.y);
    let (d1, d2, d3) = (side(now, from, a), side(now, a, b), side(now, b, from));
    !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
}

/// A row as it registered with its level in one pass.
pub(super) struct Row {
    pub(super) id: Id,
    pub(super) text: String,
    pub(super) enabled: bool,
}

/// A row's mnemonic as the level assigned it when the user last saw the rows: the letter and its character index
/// in the row's text.
struct Assigned {
    id: Id,
    mnemonic: Option<(char, usize)>,
    enabled: bool,
}

/// The one open submenu of a level.
struct OpenSubmenu {
    row: Id,
    rect: Option<Rect>,
}

#[derive(Default)]
struct LevelState {
    /// The rows of the pass `pass`, in the order they registered.
    now: Vec<Row>,
    pass: u64,
    /// The letters of the rows as the user last saw them, for a typed letter that arrives before this pass's rows.
    shown: Vec<Assigned>,
    lettered: bool,
    keyboard: bool,
    /// A value menu: every row is a radio item.
    values: bool,
    /// The row a typed letter runs in this pass.
    run: Option<Id>,
    close: bool,
    /// The Left or Right key a row of a top-level level left unhandled.
    step: Option<MenuStep>,
    submenu: bool,
    open_submenu: Option<OpenSubmenu>,
    active: Option<Id>,
    pointer: Option<Pos2>,
    previous_pointer: Option<Pos2>,
    /// The padding of the popup around the rows, as the menu that set the level has it.
    pad: f32,
}

/// The rows of one menu level: each registers its label, and the level gives every row its mnemonic from the
/// registrations of this pass. It also holds the level's one open submenu row and its active row. Rows
/// outside any menu get an inert level.
#[derive(Clone)]
pub(crate) struct MenuLevel {
    state: Option<Arc<Mutex<LevelState>>>,
}

impl PartialEq for MenuLevel {
    fn eq(&self, other: &Self) -> bool {
        match (&self.state, &other.state) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl std::fmt::Debug for MenuLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MenuLevel").finish_non_exhaustive()
    }
}

impl EnvValue for MenuLevel {
    fn root() -> Self {
        MenuLevel { state: None }
    }
}

/// The level's registrations, kept in the menu's shown memory so the rows of the last pass are readable now.
#[derive(Clone, Default)]
pub(crate) struct LevelMemory {
    live: Arc<Mutex<LevelState>>,
}

impl MenuLevel {
    pub(crate) fn of(memory: &LevelMemory) -> Self {
        MenuLevel { state: Some(Arc::clone(&memory.live)) }
    }

    fn with<R: Default>(&self, f: impl FnOnce(&mut LevelState) -> R) -> R {
        match &self.state {
            Some(state) => f(&mut state.lock().unwrap_or_else(PoisonError::into_inner)),
            None => R::default(),
        }
    }

    /// Starts a body run over the level. The rows of this pass register in the measuring run and again in the real
    /// run; the first row a pass registers drops the rows of the passes before.
    /// Rows get no mnemonic when `lettered` is false: a Mac menu has no access letters.
    fn begin(&self, submenu: bool, values: bool, keyboard: bool, lettered: bool, pointer: Option<Pos2>, pad: f32) {
        self.with(|state| {
            state.pad = pad;
            state.keyboard = keyboard;
            state.lettered = lettered;
            state.submenu = submenu;
            state.values = values;
            if state.pointer != pointer {
                state.previous_pointer = state.pointer;
                state.pointer = pointer;
            }
        });
    }

    /// The padding of the popup around the level's rows.
    pub(super) fn pad(&self) -> f32 {
        self.with(|state| state.pad)
    }

    /// Ends a body run of pass `pass`: the rows of this pass give the letters the user sees. A pass that registered no
    /// rows shows none.
    pub(crate) fn end(&self, pass: u64) {
        self.with(|state| {
            if state.pass != pass {
                state.pass = pass;
                state.now.clear();
            }
            let letters = if state.lettered {
                let labels: Vec<&str> = state.now.iter().map(|row| row.text.as_str()).collect();
                mnemonic::assign(&labels)
            } else {
                vec![None; state.now.len()]
            };
            state.shown = state.now.iter().zip(letters).map(|(row, mnemonic)| Assigned { id: row.id, mnemonic, enabled: row.enabled }).collect();
        });
    }

    pub(super) fn register(&self, pass: u64, row: Row) {
        self.with(|state| {
            if state.pass != pass {
                state.pass = pass;
                state.now.clear();
            }
            match state.now.iter_mut().find(|known| known.id == row.id) {
                Some(known) => *known = row,
                None => state.now.push(row),
            };
        });
    }

    pub(super) fn mnemonic_of(&self, id: Id) -> Option<(char, usize)> {
        self.with(|state| {
            if !state.lettered {
                return None;
            }
            let at = state.now.iter().position(|row| row.id == id)?;
            let labels: Vec<&str> = state.now.iter().map(|row| row.text.as_str()).collect();
            mnemonic::assign(&labels)[at]
        })
    }

    /// The enabled row `typed` runs.
    pub(crate) fn target(&self, typed: char) -> Option<Id> {
        let typed = typed.to_ascii_uppercase();
        self.with(|state| state.shown.iter().find(|row| row.enabled && row.mnemonic.is_some_and(|(letter, _)| letter == typed)).map(|row| row.id))
    }

    pub(crate) fn values(&self) -> bool {
        self.with(|state| state.values)
    }

    /// Whether the keyboard drives a menu: the mnemonics show.
    pub(super) fn keyboard(&self) -> bool {
        self.with(|state| state.keyboard)
    }

    /// The container's typed letter names the row `id` to run in this pass.
    pub(crate) fn run(&self, id: Id) {
        self.with(|state| state.run = Some(id));
    }

    pub(super) fn take_run(&self, id: Id) -> bool {
        self.with(|state| {
            let mine = state.run == Some(id);
            if mine {
                state.run = None;
            }
            mine
        })
    }

    pub(super) fn is_submenu(&self) -> bool {
        self.with(|state| state.submenu)
    }

    pub(super) fn request_close(&self) {
        self.with(|state| state.close = true);
    }

    pub(super) fn request_step(&self, step: MenuStep) {
        self.with(|state| state.step = Some(step));
    }

    /// The Left or Right key a row of this top-level level left unhandled.
    pub(crate) fn take_step(&self) -> Option<MenuStep> {
        self.with(|state| state.step.take())
    }

    /// Whether a row asked its submenu level to close.
    pub(crate) fn take_close(&self) -> bool {
        self.with(|state| std::mem::take(&mut state.close))
    }

    /// The submenu row `id` is the level's open submenu, in place of any other. Returns whether that changed it.
    pub(super) fn open_submenu(&self, id: Id) -> bool {
        self.with(|state| {
            let changed = state.open_submenu.as_ref().map(|open| open.row) != Some(id);
            if changed {
                state.open_submenu = Some(OpenSubmenu { row: id, rect: None });
            }
            changed
        })
    }

    /// Closes the open submenu: a row that is not a submenu row rested under the pointer. Returns whether one was open.
    pub(super) fn clear_submenu(&self) -> bool {
        self.with(|state| state.open_submenu.take().is_some())
    }

    pub(super) fn submenu_open(&self, id: Id) -> bool {
        self.with(|state| state.open_submenu.as_ref().is_some_and(|open| open.row == id))
    }

    /// The submenu of row `id` was shown at `rect`, or closed itself.
    pub(super) fn note_submenu(&self, id: Id, open: bool, rect: Rect) {
        self.with(|state| match &mut state.open_submenu {
            Some(known) if known.row == id && open => known.rect = Some(rect),
            Some(known) if known.row == id => state.open_submenu = None,
            _ => {}
        });
    }

    /// The row `id` is the level's active row. Returns whether that changed it.
    pub(super) fn activate_row(&self, id: Id) -> bool {
        self.with(|state| {
            let changed = state.active != Some(id);
            state.active = Some(id);
            changed
        })
    }

    pub(crate) fn active(&self) -> Option<Id> {
        self.with(|state| state.active)
    }

    pub(crate) fn has_active(&self) -> bool {
        self.with(|state| state.active.is_some())
    }

    pub(super) fn is_active(&self, id: Id) -> bool {
        self.with(|state| state.active == Some(id))
    }

    /// Whether the user is in the open submenu of row `id`: the pointer is over it, or the keyboard has left `id` for it.
    pub(super) fn descended(&self, id: Id, focused: bool) -> bool {
        self.with(|state| {
            let Some(OpenSubmenu { row, rect }) = &state.open_submenu else { return false };
            *row == id && (rect.zip(state.pointer).is_some_and(|(rect, pointer)| rect.contains(pointer)) || (state.keyboard && !focused))
        })
    }

    /// Whether the pointer heads for the open submenu, which is not the one of row `id`.
    pub(super) fn aimed(&self, id: Id) -> bool {
        self.with(|state| {
            let Some(OpenSubmenu { row, rect: Some(rect) }) = &state.open_submenu else { return false };
            let (Some(now), Some(from)) = (state.pointer, state.previous_pointer.or(state.pointer)) else { return false };
            *row != id && heads_toward(from, now, *rect)
        })
    }
}

/// The level a menu container puts into its body's env: the rows of the body register with it.
pub(crate) fn level_env<C: Component>(cx: &Cx<'_, C>, values: &mut AppValues, submenu: bool, valued: bool, pad: f32) {
    let level = MenuLevel::of(&cx.recall::<LevelMemory>());
    let pointer = cx.core.ctx.input(|input| input.pointer.latest_pos());
    let keyboard = crate::runtime::focus::keyboard_in_menu(&cx.core.ctx);
    level.begin(submenu, valued, keyboard, !cx.env.shell().os.is_mac(), pointer, pad);
    values.set(level);
}
