use craft_ui::testing::trial;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use raw_window_handle::{DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle};

use crate::{Answer, Dialogs, FileDialog, Reply, Request, STOPPED, Wake};

/// No window: `show_pending(None::<&NoWindow>)`.
struct NoWindow;

impl HasWindowHandle for NoWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        Err(HandleError::Unavailable)
    }
}

impl HasDisplayHandle for NoWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Err(HandleError::Unavailable)
    }
}

const NONE: Option<&NoWindow> = None;

/// The requests shown so far, with the replies a test answers them through.
type Shown = Rc<RefCell<Vec<(Request, Reply)>>>;

/// Dialogs whose requests stay open until a test answers their reply; counts wake-ups.
/// `T` is an `Rc`, which isn't `Send`: continuations stay on the main thread.
fn held() -> (Dialogs<Rc<str>>, Shown, Arc<AtomicUsize>) {
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = wakes.clone();
    let mut dialogs = Dialogs::new(Wake::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    }));
    let shown = Rc::new(RefCell::new(Vec::new()));
    let log = shown.clone();
    dialogs.set_custom(move |request, reply| log.borrow_mut().push((request, reply)));
    (dialogs, shown, wakes)
}

fn answered(dialogs: &mut Dialogs<Rc<str>>) -> Vec<(String, Answer)> {
    dialogs.take_answers().into_iter().map(|(then, answer)| (then.to_string(), answer)).collect()
}

#[trial]
fn a_worker_that_ends_without_answering_closes_the_dialog() {
    let (mut dialogs, shown, wakes) = held();
    dialogs.ask(FileDialog::open(), "dropped".into());
    dialogs.ask(FileDialog::open(), "panicked".into());
    dialogs.ask(FileDialog::open(), "next".into());
    dialogs.show_pending(NONE);

    let (_, reply) = shown.borrow_mut().remove(0);
    let before = wakes.load(Ordering::SeqCst);
    drop(reply);
    assert!(wakes.load(Ordering::SeqCst) > before, "the drop wakes the app");
    dialogs.show_pending(NONE);

    // A worker thread that panics while it holds the reply.
    let (_, reply) = shown.borrow_mut().remove(0);
    let worker = std::thread::spawn(move || {
        let _reply = reply;
        std::panic::resume_unwind(Box::new("rfd panicked"));
    });
    assert!(worker.join().is_err());

    assert_eq!(
        answered(&mut dialogs),
        [("dropped".to_owned(), Answer::Unavailable(STOPPED.into())), ("panicked".to_owned(), Answer::Unavailable(STOPPED.into()))]
    );
    assert!(!dialogs.is_open());
    dialogs.show_pending(NONE);
    assert_eq!(shown.borrow().len(), 1, "the next request shows");
}
