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
    dialogs.set_custom(move |request, reply| {
        log.borrow_mut().push((request, reply));
        crate::native::Shown::default()
    });
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

/// A scripted dialog: it stays open for `open` on the test's clock, then answers `answer`.
#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
async fn scripted(answer: Answer, open: std::time::Duration, clock: Rc<std::cell::Cell<std::time::Duration>>) -> Answer {
    clock.set(clock.get() + open);
    answer
}

/// An empty answer at once is a dialog that never showed: it is asked again (without a parent),
/// and empty at once again is unavailable. A slow empty answer is the user's cancel.
#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
#[trial]
fn a_dialog_that_closes_at_once_is_asked_again_without_a_parent() {
    use crate::native::timed;
    use crate::portal::{CLOSED_AT_ONCE, TOO_QUICK};
    use std::cell::Cell;
    use std::time::Duration;

    let (at_once, slow) = (Duration::from_millis(5), TOO_QUICK + Duration::from_millis(1));
    let picked = Answer::Paths(vec![std::path::PathBuf::from("/s/a.txt")]);
    // The first dialog's answer and how long it was open, the same for the second; gives the
    // answer and whether the second was asked.
    let run = |first: (Answer, Duration), second: (Answer, Duration)| {
        let clock = Rc::new(Cell::new(Duration::ZERO));
        let again = Rc::new(Cell::new(false));
        let (opened, reopened, read, asked) = (clock.clone(), clock.clone(), clock, again.clone());
        let answer = futures_lite::future::block_on(timed(
            scripted(first.0, first.1, opened),
            move || {
                asked.set(true);
                scripted(second.0, second.1, reopened)
            },
            move || read.get(),
        ));
        (answer, again.get())
    };
    assert_eq!(run((Answer::Cancelled, at_once), (Answer::Cancelled, at_once)), (Answer::Unavailable(CLOSED_AT_ONCE.into()), true));
    assert_eq!(run((Answer::Cancelled, at_once), (picked.clone(), at_once)), (picked.clone(), true), "the second dialog's pick counts");
    assert_eq!(run((Answer::Cancelled, slow), (picked.clone(), at_once)), (Answer::Cancelled, false), "a slow empty answer is a cancel");
    assert_eq!(run((picked.clone(), at_once), (Answer::Cancelled, at_once)), (picked, false), "a quick pick is a pick");
}
