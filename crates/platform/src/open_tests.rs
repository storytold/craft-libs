//! `craft_platform::open`.

// A source whose push calls the wake while holding its own borrow (the Apple-event inbox does)
// must not be drained from the wake: the wake only repaints, and the next `take` collects it.
#[test]
fn a_request_pushed_while_its_source_is_borrowed_is_taken_on_the_next_take() {
    use crate::open::{FileIntake, Intake};
    use craft_host::Wake;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let woken = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&woken);
    let (intake, sender) = FileIntake::new(Wake::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    }));
    let source: Rc<RefCell<Vec<PathBuf>>> = Rc::default();
    let drained = Rc::clone(&source);
    let _registration = sender.register_drain(move |sender| {
        let paths = std::mem::take(&mut *drained.borrow_mut());
        sender.open(paths);
    });

    {
        let mut held = source.borrow_mut();
        held.push(PathBuf::from("/tmp/a.docx"));
        sender.wake_ui();
    }
    assert_eq!(woken.load(Ordering::SeqCst), 1, "the wake ran once, under the source's borrow, and drained nothing");
    let taken = intake.take();
    assert!(matches!(taken.as_slice(), [Intake::Open(paths)] if paths.len() == 1), "{taken:?}");
    assert!(intake.take().is_empty(), "the source was emptied");
}

// The target is one argument after `--` (or the Windows handler), so no shell reads a `&` and no
// tool reads a leading `-` as an option.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn open_command_keeps_the_target_one_inert_argument() {
    let target = "https://x.test/?a=1&calc";
    let command = crate::open::open_command(target);
    let args: Vec<_> = command.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
    assert_ne!(command.get_program(), "cmd");
    assert_eq!(args.last().map(String::as_str), Some(target));
    if !cfg!(windows) {
        assert_eq!(args, ["--", target]);
    }
}
