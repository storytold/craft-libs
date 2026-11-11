//! One system installer at a time.
//!
//! apt, dnf, rpm and the Windows Installer each hold a system-wide lock: an install started
//! while another runs fails. Jobs wait their turn here instead. Where the installer takes
//! several packages, the jobs waiting for the same command share one call, so the password is
//! asked for once.

use std::ffi::OsString;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::protocol::CANCELLED;

type Outcome = Result<(), String>;

struct Waiting {
    id: u64,
    command: Vec<OsString>,
    target: OsString,
}

struct State {
    busy: bool,
    next_id: u64,
    waiting: Vec<Waiting>,
    /// The jobs of the call in progress.
    running: Vec<u64>,
    finished: Vec<(u64, Outcome)>,
}

/// Runs system installs one at a time. Share one queue between all install jobs of a process.
pub struct Queue {
    state: Mutex<State>,
    changed: Condvar,
}

/// Frees the installer when a call ends (even by unwinding) and hands its outcome to the other
/// jobs of the call.
struct Turn<'a> {
    queue: &'a Queue,
    others: Vec<u64>,
    outcome: Outcome,
}

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        let mut state = self.queue.lock();
        state.busy = false;
        state.running.clear();
        for id in self.others.drain(..) {
            state.finished.push((id, self.outcome.clone()));
        }
        self.queue.changed.notify_all();
    }
}

impl Default for Queue {
    fn default() -> Self {
        Self::new()
    }
}

impl Queue {
    /// An empty queue.
    pub const fn new() -> Self {
        let state = State { busy: false, next_id: 0, waiting: Vec::new(), running: Vec::new(), finished: Vec::new() };
        Self { state: Mutex::new(state), changed: Condvar::new() }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Until something changes, or long enough to ask again whether the job was cancelled.
    fn wait<'a>(&self, state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        self.changed.wait_timeout(state, Duration::from_millis(100)).unwrap_or_else(PoisonError::into_inner).0
    }

    /// Runs `work` once no other installer is running. `turn(true)` says it has to wait,
    /// `turn(false)` that the wait is over. `None` when `cancelled` (asked a few times a second,
    /// so it must not block) said so during the wait; `work` then never ran.
    pub fn alone<R>(&self, cancelled: &dyn Fn() -> bool, turn: &mut dyn FnMut(bool), work: impl FnOnce() -> R) -> Option<R> {
        let mut state = self.lock();
        let mut waited = false;
        loop {
            if waited && cancelled() {
                return None;
            }
            if !state.busy {
                break;
            }
            if waited {
                state = self.wait(state);
            } else {
                waited = true;
                drop(state);
                turn(true);
                state = self.lock();
            }
        }
        state.busy = true;
        drop(state);
        if waited {
            turn(false);
        }
        let _turn = Turn { queue: self, others: Vec::new(), outcome: Ok(()) };
        Some(work())
    }

    /// Runs `exec(command, targets)` once no other installer is running, with the targets of
    /// every job waiting for the same `command`, and gives each of those jobs its outcome.
    /// `turn(true)` says the job has to wait, `turn(false)` that its call has started.
    ///
    /// A job that is waiting leaves with [`CANCELLED`] once `cancelled` (asked a few times a
    /// second, so it must not block) says so; once its call has started it sees it through.
    pub fn batched(
        &self,
        command: Vec<OsString>,
        target: OsString,
        cancelled: &dyn Fn() -> bool,
        turn: &mut dyn FnMut(bool),
        exec: &dyn Fn(&[OsString], &[OsString]) -> Outcome,
    ) -> Outcome {
        let mut state = self.lock();
        let id = state.next_id;
        state.next_id = state.next_id.wrapping_add(1);
        state.waiting.push(Waiting { id, command: command.clone(), target });
        let (mut waited, mut started) = (false, false);
        loop {
            if let Some(at) = state.finished.iter().position(|(done, _)| *done == id) {
                return state.finished.swap_remove(at).1;
            }
            // Another job took this one into its call.
            let running = state.running.contains(&id);
            if !running && waited && cancelled() {
                state.waiting.retain(|w| w.id != id);
                return Err(CANCELLED.into());
            }
            if !running && !state.busy {
                break;
            }
            let tell = if running { waited && !started } else { !waited };
            if tell {
                (waited, started) = (true, running);
                drop(state);
                turn(!running);
                state = self.lock();
            } else {
                state = self.wait(state);
            }
        }
        let (batch, rest): (Vec<Waiting>, Vec<Waiting>) = std::mem::take(&mut state.waiting).into_iter().partition(|w| w.command == command);
        state.waiting = rest;
        state.busy = true;
        state.running = batch.iter().map(|w| w.id).collect();
        self.changed.notify_all();
        drop(state);
        if waited {
            turn(false);
        }
        let others = batch.iter().map(|w| w.id).filter(|other| *other != id).collect();
        let targets: Vec<OsString> = batch.into_iter().map(|w| w.target).collect();
        let mut call = Turn { queue: self, others, outcome: Err("the system installer was interrupted".into()) };
        let outcome = exec(&command, &targets);
        call.outcome.clone_from(&outcome);
        outcome
    }
}

#[cfg(test)]
mod tests;
