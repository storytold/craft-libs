//! Native dialogs through rfd's async dialogs, with a worker thread that waits for the answer.
//!
//! How rfd 0.17.2 shows a dialog (from its source):
//! - macOS: creating the future begins the panel, so it happens here on the main thread. The async
//!   path is always a sheet: on the parent's window, else the app's main or first window
//!   (`backend/macos/modal_future.rs:71-75`). Without a running app and a window it falls back to
//!   the blocking `runModal` (`modal_future.rs:79-118`), the crash this crate exists to avoid, so
//!   a sheet starts only with a parent in a running app ([`crate::macos::sheet_parent`]). Open and
//!   Folder don't go through rfd at all on macOS: see [`crate::macos`].
//! - Windows: the dialog runs on a thread rfd starts when the worker first polls the future.
//! - Linux and the BSDs: the desktop portal (zenity as a fallback) works when the worker polls the
//!   future; rfd's optional GTK3 backend runs on rfd's own GTK thread.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::mpsc;
use std::task::{Context, Poll, Wake, Waker};

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use crate::{Answer, FileDialog, Kind, Message, MessageButtons, MessageLevel, MessageResult, Reply, Request};

type Pending = Pin<Box<dyn Future<Output = Answer> + Send>>;

/// What [`crate::Dialogs`] keeps while a dialog is on screen: on macOS, the free-standing panel to
/// raise. Other platforms have none.
pub(crate) type Shown = Option<Panel>;
#[cfg(target_os = "macos")]
pub(crate) use crate::macos::Panel;
#[cfg(not(target_os = "macos"))]
pub(crate) type Panel = std::convert::Infallible;

pub(crate) fn show<W: HasWindowHandle + HasDisplayHandle>(request: Request, parent: Option<&W>, reply: Reply) -> Shown {
    // rfd quietly drops a parent whose handles it can't get; the checks below need to know.
    let parent = parent.filter(|w| w.window_handle().is_ok() && w.display_handle().is_ok());
    match request {
        Request::File(dialog) => show_file(dialog, parent, reply),
        Request::Message(message) => {
            show_message(message, parent, reply);
            None
        }
    }
}

fn show_file<W: HasWindowHandle + HasDisplayHandle>(d: FileDialog, parent: Option<&W>, reply: Reply) -> Shown {
    #[cfg(target_os = "macos")]
    let parent = match d.kind {
        Kind::Open { .. } | Kind::Folder => return crate::macos::show_panel(&d, reply),
        Kind::Save => match crate::macos::sheet_parent(parent) {
            Ok(parent) => Some(parent),
            Err(why) => {
                reply.send(Answer::Unavailable(why));
                return None;
            }
        },
    };
    let mut dialog = rfd::AsyncFileDialog::new();
    if let Some(parent) = parent {
        dialog = dialog.set_parent(parent);
    }
    if let Some(title) = &d.title {
        dialog = dialog.set_title(title);
    }
    for filter in &d.filters {
        dialog = dialog.add_filter(&filter.name, &filter.patterns());
    }
    if let Some(directory) = &d.directory {
        dialog = dialog.set_directory(directory);
    }
    if let Some(name) = &d.file_name {
        dialog = dialog.set_file_name(name);
    }
    // The worker first, so a dialog once shown always has somewhere to answer.
    let worker = worker(reply)?;
    let one = |file: Option<rfd::FileHandle>| file.map_or(Answer::Cancelled, |f| Answer::Paths(vec![f.path().to_path_buf()]));
    let pending: Pending = match d.kind {
        Kind::Open { multiple: false } => {
            let picked = dialog.pick_file();
            Box::pin(async move { one(picked.await) })
        }
        Kind::Open { multiple: true } => {
            let picked = dialog.pick_files();
            Box::pin(async move {
                match picked.await {
                    Some(files) if !files.is_empty() => Answer::Paths(files.iter().map(|f| f.path().to_path_buf()).collect()),
                    _ => Answer::Cancelled,
                }
            })
        }
        Kind::Folder => {
            let picked = dialog.pick_folder();
            Box::pin(async move { one(picked.await) })
        }
        Kind::Save => {
            let picked = dialog.save_file();
            Box::pin(async move { one(picked.await) })
        }
    };
    // The worker can't be gone: it waits for this. If it were, its reply answers Unavailable.
    let _ = worker.send(pending);
    None
}

fn show_message<W: HasWindowHandle + HasDisplayHandle>(m: Message, parent: Option<&W>, reply: Reply) {
    #[cfg(target_os = "macos")]
    let parent = match crate::macos::sheet_parent(parent) {
        Ok(parent) => Some(parent),
        Err(why) => return reply.send(Answer::Unavailable(why)),
    };
    let level = match m.level {
        MessageLevel::Info => rfd::MessageLevel::Info,
        MessageLevel::Warning => rfd::MessageLevel::Warning,
        MessageLevel::Error => rfd::MessageLevel::Error,
    };
    let buttons = match m.buttons {
        MessageButtons::Ok => rfd::MessageButtons::Ok,
        MessageButtons::OkCancel => rfd::MessageButtons::OkCancel,
        MessageButtons::YesNo => rfd::MessageButtons::YesNo,
        MessageButtons::YesNoCancel => rfd::MessageButtons::YesNoCancel,
        MessageButtons::OkCustom(ok) => rfd::MessageButtons::OkCustom(ok),
        MessageButtons::OkCancelCustom(ok, cancel) => rfd::MessageButtons::OkCancelCustom(ok, cancel),
        MessageButtons::YesNoCancelCustom(yes, no, cancel) => rfd::MessageButtons::YesNoCancelCustom(yes, no, cancel),
    };
    let mut dialog = rfd::AsyncMessageDialog::new().set_level(level).set_title(m.title).set_description(m.description).set_buttons(buttons);
    if let Some(parent) = parent {
        dialog = dialog.set_parent(parent);
    }
    let Some(worker) = worker(reply) else { return };
    let shown = dialog.show();
    let _ = worker.send(Box::pin(async move {
        Answer::Message(match shown.await {
            rfd::MessageDialogResult::Ok => MessageResult::Ok,
            rfd::MessageDialogResult::Cancel => MessageResult::Cancel,
            rfd::MessageDialogResult::Yes => MessageResult::Yes,
            rfd::MessageDialogResult::No => MessageResult::No,
            rfd::MessageDialogResult::Custom(label) => MessageResult::Custom(label),
        })
    }));
}

/// Starts a thread that waits for the dialog sent to it and answers through `reply`. `None` when
/// the thread couldn't start; `reply` is then dropped, which answers Unavailable.
fn worker(reply: Reply) -> Option<mpsc::Sender<Pending>> {
    let (tx, rx) = mpsc::channel::<Pending>();
    std::thread::Builder::new()
        .name("craft-dialogs".into())
        .spawn(move || {
            if let Ok(pending) = rx.recv() {
                reply.send(block_on(pending));
            }
        })
        .ok()?;
    Some(tx)
}

/// Runs `future` to completion on this thread, sleeping until it is woken.
fn block_on<F: Future>(future: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            return output;
        }
        std::thread::park();
    }
}
