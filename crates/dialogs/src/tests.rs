use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use raw_window_handle::{DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle};

use crate::{Answer, Dialogs, FileDialog, Kind, Message, MessageResult, Reply, Request, STOPPED, filter};

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
    let mut dialogs = Dialogs::new(Arc::new(move || {
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

#[test]
fn requests_wait_in_order_behind_the_open_dialog() {
    let (mut dialogs, shown, _) = held();
    dialogs.ask(FileDialog::open(), "a".into());
    dialogs.ask(FileDialog::save().file_name("b.txt"), "b".into());
    dialogs.ask_message(Message { title: "c".into(), ..Message::default() }, "c".into());
    dialogs.show_pending(NONE);
    dialogs.show_pending(NONE);
    assert_eq!(shown.borrow().len(), 1, "one dialog at a time");
    assert!(dialogs.is_open());
    assert_eq!(dialogs.queued(), 2);

    let (request, reply) = shown.borrow_mut().remove(0);
    assert_eq!(request, Request::File(FileDialog::open()));
    reply.send(Answer::Paths(vec![PathBuf::from("a.txt")]));
    dialogs.show_pending(NONE);
    let (request, reply) = shown.borrow_mut().remove(0);
    assert!(matches!(request, Request::File(FileDialog { kind: Kind::Save, .. })));
    reply.send(Answer::Cancelled);
    dialogs.show_pending(NONE);
    let (request, reply) = shown.borrow_mut().remove(0);
    assert!(matches!(request, Request::Message(_)));
    reply.send(Answer::Message(MessageResult::Ok));

    assert_eq!(
        answered(&mut dialogs),
        [
            ("a".to_owned(), Answer::Paths(vec![PathBuf::from("a.txt")])),
            ("b".to_owned(), Answer::Cancelled),
            ("c".to_owned(), Answer::Message(MessageResult::Ok)),
        ]
    );
    assert!(!dialogs.is_open());
    assert!(dialogs.take_answers().is_empty());
}

#[test]
fn a_cancelled_dialog_answers_cancelled_and_closes() {
    let (mut dialogs, shown, wakes) = held();
    dialogs.ask(FileDialog::folder(), "f".into());
    dialogs.show_pending(NONE);
    let before = wakes.load(Ordering::SeqCst);
    let (_, reply) = shown.borrow_mut().remove(0);
    reply.send(Answer::Cancelled);
    assert!(wakes.load(Ordering::SeqCst) > before, "the answer wakes the app");
    assert_eq!(answered(&mut dialogs), [("f".to_owned(), Answer::Cancelled)]);
    assert!(!dialogs.is_open());
}

#[test]
fn the_override_answers_every_request_on_a_later_take() {
    let mut dialogs: Dialogs<u32> = Dialogs::new(Arc::new(|| {}));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    dialogs.set_override(Some(Box::new(move |request: &Request| {
        log.borrow_mut().push(request.clone());
        match request {
            Request::File(FileDialog { kind: Kind::Open { .. }, .. }) => Answer::Paths(vec![PathBuf::from("in.pdf")]),
            Request::File(_) => Answer::Cancelled,
            Request::Message(_) => Answer::Message(MessageResult::Yes),
        }
    })));
    dialogs.ask(FileDialog::open_many(), 1);
    dialogs.ask(FileDialog::save(), 2);
    dialogs.ask_message(Message::default(), 3);
    assert!(dialogs.take_answers().is_empty(), "nothing answers before show_pending");
    dialogs.show_pending(NONE);
    assert_eq!(seen.borrow().len(), 3);
    assert_eq!(
        dialogs.take_answers(),
        [(1, Answer::Paths(vec![PathBuf::from("in.pdf")])), (2, Answer::Cancelled), (3, Answer::Message(MessageResult::Yes))]
    );
    assert!(!dialogs.is_open());
}

#[test]
fn a_panicking_override_answers_unavailable() {
    let mut dialogs: Dialogs<u32> = Dialogs::new(Arc::new(|| {}));
    dialogs.set_override(Some(Box::new(|_: &Request| panic!("test override"))));
    dialogs.ask(FileDialog::open(), 1);
    dialogs.show_pending(NONE);
    let answers = dialogs.take_answers();
    assert!(matches!(answers.as_slice(), [(1, Answer::Unavailable(why))] if why.contains("test override")), "{answers:?}");
    assert!(!dialogs.is_open());
}

#[test]
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

#[test]
fn a_panic_while_showing_answers_unavailable_and_moves_on() {
    let mut dialogs: Dialogs<u32> = Dialogs::new(Arc::new(|| {}));
    let shown = Rc::new(RefCell::new(0));
    let count = shown.clone();
    dialogs.set_custom(move |_, reply| {
        *count.borrow_mut() += 1;
        if *count.borrow() == 1 {
            std::panic::resume_unwind(Box::new("no window"));
        }
        reply.send(Answer::Cancelled);
    });
    dialogs.ask(FileDialog::open(), 1);
    dialogs.ask(FileDialog::open(), 2);
    dialogs.show_pending(NONE);
    assert_eq!(dialogs.take_answers(), [(1, Answer::Unavailable("the dialog couldn't be shown: no window".into())), (2, Answer::Cancelled)]);
}

#[test]
fn linux_globs_match_every_letter_case() {
    // Matches `name` against a glob of literals and `[..]` classes, as the portal does after `*.`.
    fn matches(glob: &str, name: &str) -> bool {
        let mut name = name.chars();
        let mut glob = glob.chars();
        while let Some(g) = glob.next() {
            let class: String = if g == '[' { glob.by_ref().take_while(|&c| c != ']').collect() } else { g.to_string() };
            if !name.next().is_some_and(|n| class.contains(n)) {
                return false;
            }
        }
        name.next().is_none()
    }
    let exts = |e: &[&str]| e.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    assert_eq!(filter::patterns(&exts(&["NEF", ".Jpg", "jpg"]), false), ["nef", "jpg"]);
    assert_eq!(filter::patterns(&exts(&["nef"]), true), ["nef", "NEF", "[nN][eE][fF]"]);
    assert_eq!(filter::patterns(&exts(&["mp4", "MP4"]), true), ["mp4", "MP4", "[mM][pP]4"]);
    let globs = filter::patterns(&exts(&["jpeg"]), true);
    for name in ["jpeg", "JPEG", "JpEg", "jPeG"] {
        assert!(globs.iter().any(|g| matches(g, name)), "{name}");
    }
    assert!(!globs.iter().any(|g| matches(g, "jpg") || matches(g, "jpegx")));
}
