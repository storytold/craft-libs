//! The window's dialog host: one [`DialogHost`] per [`WindowKey`], shared by the shell that runs it and the bodies that
//! open dialogs on it.
//!
//! `Slot` needs `Clone` and belongs to one container id, and the host is neither `Clone` nor per container, so the
//! shell keeps the hosts in a context service keyed by window. [`WindowDialogs`] is the shared handle to one of them.

use crate::contract::BodyCx;
use crate::dialog::DialogHost;
use crate::runtime::egui::memory;
use crate::runtime::env::WindowKey;
use egui::{Context, Ui};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, TryLockError};

/// The hosts of the context's windows.
#[derive(Default)]
struct Hosts(HashMap<WindowKey, WindowDialogs>);

/// A window's dialog host, as a handle that clones cheaply. The `AppShell` of the window runs the host after its body
/// in each pass, so a dialog opened in any pass shows over the window from that pass on and its answer waits for
/// `take` until the app reads it.
///
/// While the host runs its dialogs, or inside a `with` closure, [`ShellDialogsCx::dialogs`] returns `None`: a dialog being run
/// opens children with the host methods of its own (`DialogHost::open_child`).
#[derive(Clone, Default)]
pub struct WindowDialogs(Arc<Mutex<DialogHost>>);

impl WindowDialogs {
    /// The host of `window`, created on first use.
    fn of(ctx: &Context, window: &WindowKey) -> WindowDialogs {
        memory::with::<Hosts, _>(ctx, |hosts| hosts.0.entry(window.clone()).or_default().clone())
    }

    /// The host of `window` when a body has asked for it.
    pub(super) fn existing(ctx: &Context, window: &WindowKey) -> Option<WindowDialogs> {
        memory::with::<Hosts, _>(ctx, |hosts| hosts.0.get(window).cloned())
    }

    /// Runs `f` on the host.
    pub fn with<R>(&self, f: impl FnOnce(&mut DialogHost) -> R) -> R {
        f(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Runs the open dialogs for this pass.
    pub(super) fn run(&self, ui: &mut Ui) {
        self.with(|host| host.run(ui));
    }
}

/// The window's dialog host, on a body's `BodyCx`.
pub trait ShellDialogsCx {
    /// The dialog host of the window this body is in; `None` where no shell scope is set, and while the host is locked:
    /// a dialog the host is running, or a `with` closure, reaches the host through its own `DialogHost` value
    /// (`open_child`), and this call would wait on a lock its own thread holds. Only an `AppShell` runs the host. Two
    /// windows ([`WindowKey`]s) have two hosts. A dialog opened here shows over the window after the body in the same
    /// pass.
    fn dialogs(&self) -> Option<WindowDialogs>;
}

impl ShellDialogsCx for BodyCx<'_> {
    fn dialogs(&self) -> Option<WindowDialogs> {
        let window = self.shell()?;
        let dialogs = WindowDialogs::of(self.memory.ctx(), window.window());
        let free = !matches!(dialogs.0.try_lock(), Err(TryLockError::WouldBlock));
        free.then_some(dialogs)
    }
}
