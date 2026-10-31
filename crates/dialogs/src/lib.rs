#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Native file and message dialogs that never block the event loop.
//!
//! A blocking `rfd::FileDialog` or `rfd::MessageDialog` runs AppKit's modal loop
//! (`-[NSSavePanel runModal]`) inside winit's event handler on macOS. An event that arrives while
//! that loop spins (a file dragged over the window, the panel resized) re-enters the handler, winit
//! panics with "tried to handle event while another event is currently being handled", and since
//! the panic can't unwind out of the AppKit callback, the app aborts.
//!
//! [`Dialogs`] never blocks: [`Dialogs::ask`] and [`Dialogs::ask_message`] queue a request,
//! [`Dialogs::show_pending`] shows the first one at the end of the frame on the main thread, and
//! [`Dialogs::take_answers`] hands back `(then, Answer)` pairs on a later frame. `then` is the
//! app's own continuation (a command to re-run, a field to fill), so each app keeps its command
//! model. One dialog shows at a time; later requests wait in order.
//!
//! On macOS, Open and Folder show as a free-standing panel that doesn't block the window, as
//! native apps do; Save and message boxes show as a sheet on the parent window. Elsewhere every
//! dialog is a modal owned by the parent window.

mod filter;
mod native;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

pub use filter::Filter;

/// Wakes the app's event loop so it runs a frame (`ctx.request_repaint()` in an egui app). Called
/// from any thread when a dialog has answered or stopped.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// Answers every request instead of a native dialog (tests and automation).
pub type Override = Box<dyn FnMut(&Request) -> Answer>;

/// Which file dialog to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Choose existing files: one, or several when `multiple`.
    Open { multiple: bool },
    /// Choose one folder.
    Folder,
    /// Choose where to write a file.
    Save,
}

/// A file dialog request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDialog {
    pub kind: Kind,
    /// The dialog's title, or its message line on macOS, where panels have no title bar.
    pub title: Option<String>,
    /// The file types to offer. Extensions match in any letter case on every platform.
    pub filters: Vec<Filter>,
    /// The folder the dialog starts in.
    pub directory: Option<PathBuf>,
    /// The file name a Save dialog suggests.
    pub file_name: Option<String>,
}

impl FileDialog {
    /// A dialog of `kind` with nothing else set.
    pub fn new(kind: Kind) -> Self {
        Self { kind, title: None, filters: Vec::new(), directory: None, file_name: None }
    }

    /// Choose one existing file.
    pub fn open() -> Self {
        Self::new(Kind::Open { multiple: false })
    }

    /// Choose one or more existing files.
    pub fn open_many() -> Self {
        Self::new(Kind::Open { multiple: true })
    }

    /// Choose a folder.
    pub fn folder() -> Self {
        Self::new(Kind::Folder)
    }

    /// Choose where to save.
    pub fn save() -> Self {
        Self::new(Kind::Save)
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Adds a file type: `name` as the dialog lists it, `extensions` without the dot.
    pub fn filter(mut self, name: impl Into<String>, extensions: &[&str]) -> Self {
        self.filters.push(Filter::new(name, extensions));
        self
    }

    pub fn directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.directory = Some(directory.into());
        self
    }

    pub fn file_name(mut self, file_name: impl Into<String>) -> Self {
        self.file_name = Some(file_name.into());
        self
    }
}

/// The icon a message box shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MessageLevel {
    #[default]
    Info,
    Warning,
    Error,
}

/// The buttons a message box offers. The custom variants name the buttons, in the order of the
/// plain variant they replace.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MessageButtons {
    #[default]
    Ok,
    OkCancel,
    YesNo,
    YesNoCancel,
    OkCustom(String),
    OkCancelCustom(String, String),
    YesNoCancelCustom(String, String, String),
}

/// The button a message box was closed with. A custom button answers with its label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessageResult {
    Ok,
    Cancel,
    Yes,
    No,
    Custom(String),
}

/// A message box request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Message {
    pub level: MessageLevel,
    pub title: String,
    pub description: String,
    pub buttons: MessageButtons,
}

/// What [`Dialogs`] was asked to show, as an [`Override`] sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    File(FileDialog),
    Message(Message),
}

/// How a dialog ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// The user closed a file dialog without choosing.
    Cancelled,
    /// The chosen files, folder or save path.
    Paths(Vec<PathBuf>),
    /// The button a message box was closed with.
    Message(MessageResult),
    /// No dialog could be shown, or it stopped without an answer; the text says why. Report it as
    /// an error rather than taking it for Cancel.
    Unavailable(String),
}

/// The answer of a dialog whose worker ended without answering (it panicked, or couldn't start).
pub(crate) const STOPPED: &str = "the dialog stopped without an answer";

/// Hands one dialog's answer back to [`Dialogs`] from any thread, and wakes the app. Dropped
/// without an answer (a worker that panicked or never started), it still wakes the app, which then
/// answers [`Answer::Unavailable`], so a dialog never stays open for good.
pub(crate) struct Reply {
    tx: Option<Sender<Answer>>,
    wake: Wake,
}

impl Reply {
    pub(crate) fn send(mut self, answer: Answer) {
        if let Some(tx) = self.tx.take() {
            // The app gone meanwhile has no use for the answer.
            let _ = tx.send(answer);
        }
    }
}

impl Drop for Reply {
    fn drop(&mut self) {
        // The sender goes first, so the woken app sees either the answer or the disconnect.
        self.tx = None;
        (self.wake)();
    }
}

/// Who shows a request.
enum Show {
    Native,
    Override(Override),
    /// Tests: a stand-in for the native dialog that answers (or doesn't) through the reply.
    #[cfg(test)]
    Custom(Box<dyn FnMut(Request, Reply)>),
}

/// The dialog on screen.
struct Open<T> {
    then: T,
    answer: Receiver<Answer>,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    shown: native::Shown,
}

/// The app's dialogs: a queue of requests, the one on screen, and the answers not yet taken.
///
/// `T` is the app's continuation for an answer. It stays on the main thread: no `Send` bound.
pub struct Dialogs<T> {
    wake: Wake,
    queue: VecDeque<(Request, T)>,
    open: Option<Open<T>>,
    answers: Vec<(T, Answer)>,
    show: Show,
}

impl<T> Dialogs<T> {
    pub fn new(wake: Wake) -> Self {
        Self { wake, queue: VecDeque::new(), open: None, answers: Vec::new(), show: Show::Native }
    }

    /// Queues a file dialog. It shows at the next [`Self::show_pending`] once the dialogs asked
    /// before it have answered; its answer comes back with `then`.
    pub fn ask(&mut self, dialog: FileDialog, then: T) {
        self.push(Request::File(dialog), then);
    }

    /// Queues a message box, like [`Self::ask`].
    pub fn ask_message(&mut self, message: Message, then: T) {
        self.push(Request::Message(message), then);
    }

    fn push(&mut self, request: Request, then: T) {
        if self.open.is_some() {
            self.raise();
        }
        self.queue.push_back((request, then));
        (self.wake)();
    }

    /// Shows the next queued request unless a dialog is open. Call it on the main thread at the end
    /// of every frame, with the window as `parent`. On macOS, Save and message boxes need the
    /// parent (they show as its sheet) and answer [`Answer::Unavailable`] without one.
    pub fn show_pending<W: HasWindowHandle + HasDisplayHandle>(&mut self, parent: Option<&W>) {
        self.poll();
        while self.open.is_none() {
            let Some((request, then)) = self.queue.pop_front() else { return };
            let (tx, answer) = mpsc::channel();
            let reply = Reply { tx: Some(tx), wake: self.wake.clone() };
            // A panic while showing (rfd's own checks) must not unwind into winit's handler.
            let shown = match &mut self.show {
                Show::Override(answer_with) => catch_unwind(AssertUnwindSafe(|| reply.send(answer_with(&request)))).map(|()| None),
                Show::Native => catch_unwind(AssertUnwindSafe(|| native::show(request, parent, reply))),
                #[cfg(test)]
                Show::Custom(show) => catch_unwind(AssertUnwindSafe(|| show(request, reply))).map(|()| None),
            };
            let shown = match shown {
                Ok(shown) => shown,
                Err(panic) => {
                    let why = panic_text(panic.as_ref()).unwrap_or(STOPPED);
                    self.answers.push((then, Answer::Unavailable(format!("the dialog couldn't be shown: {why}"))));
                    continue;
                }
            };
            self.open = Some(Open { then, answer, shown });
            self.poll();
        }
    }

    /// The answers that came in since the last call, in the order the dialogs were asked.
    pub fn take_answers(&mut self) -> Vec<(T, Answer)> {
        self.poll();
        std::mem::take(&mut self.answers)
    }

    /// A dialog is on screen, or its answer hasn't been collected yet (by [`Self::show_pending`]
    /// or [`Self::take_answers`]).
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Brings the open free-standing panel (Open or Folder on macOS) to the front, for a command
    /// that would show it again. Does nothing elsewhere, where dialogs are modal to the window.
    pub fn raise(&self) {
        #[cfg(target_os = "macos")]
        if let Some(panel) = self.open.as_ref().and_then(|open| open.shown.as_ref()) {
            macos::raise(panel);
        }
    }

    /// Requests waiting behind the open dialog.
    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    /// Answers every request with `answer_with` instead of a native dialog (tests and automation);
    /// `None` shows native dialogs again. The answers still come back from
    /// [`Self::take_answers`], after [`Self::show_pending`], like real ones.
    pub fn set_override(&mut self, answer_with: Option<Override>) {
        self.show = answer_with.map_or(Show::Native, Show::Override);
    }

    /// Tests: show requests through `show` instead of a native dialog.
    #[cfg(test)]
    fn set_custom(&mut self, show: impl FnMut(Request, Reply) + 'static) {
        self.show = Show::Custom(Box::new(show));
    }

    /// Moves the open dialog's answer, once it is in, to the answers.
    fn poll(&mut self) {
        let Some(open) = &self.open else { return };
        let answer = match open.answer.try_recv() {
            Ok(answer) => answer,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Answer::Unavailable(STOPPED.into()),
        };
        if let Some(open) = self.open.take() {
            self.answers.push((open.then, answer));
        }
    }
}

fn panic_text(panic: &(dyn std::any::Any + Send)) -> Option<&str> {
    panic.downcast_ref::<&str>().copied().or_else(|| panic.downcast_ref::<String>().map(String::as_str))
}
