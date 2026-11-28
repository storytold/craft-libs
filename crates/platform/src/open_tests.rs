//! `craft_platform::open`.

// Requests heard on another thread (a second launch, an Apple Event) reach the UI once each, in
// the order they arrived, an open for an action never merged with a plain open, and each wakes
// the UI: a lost wake leaves a file unopened until the user moves the mouse.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn an_intake_delivers_each_batch_once_in_order_and_wakes() {
    use crate::open::{FileIntake, Intake};
    use craft_host::Wake;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&wakes);
    let (intake, sender) = FileIntake::new(Wake::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    }));
    let root = std::env::temp_dir();
    let (b, a10, a2, c, d) = (root.join("b.pdf"), root.join("a10.pdf"), root.join("a2.pdf"), root.join("c.pdf"), root.join("d.pdf"));
    let bytes: Arc<[u8]> = Arc::from(&b"%PDF-1.7"[..]);
    let sent = bytes.clone();
    let (open, open_for) = ((b.clone(), a10.clone(), a2.clone()), (c.clone(), d.clone()));
    std::thread::spawn(move || {
        sender.open(vec![open.0, open.1, open.2]);
        sender.open_for(vec![open_for.0], "combine");
        sender.open_for(vec![open_for.1], "combine");
        sender.open_bytes("scan.pdf".into(), sent);
        // Empty requests queue nothing and wake nobody.
        sender.open(Vec::new());
        sender.open_for(Vec::new(), "combine");
        sender.open_bytes(String::new(), Arc::from(Vec::<u8>::new()));
    })
    .join()
    .unwrap();
    assert_eq!(
        intake.take(),
        vec![
            Intake::Open(vec![a2, a10, b]),
            Intake::OpenFor { paths: vec![c], action: "combine".into() },
            Intake::OpenFor { paths: vec![d], action: "combine".into() },
            Intake::Bytes { name: "scan.pdf".into(), bytes },
        ]
    );
    assert_eq!(wakes.load(Ordering::SeqCst), 4, "one wake per queued batch");
    assert!(intake.take().is_empty(), "each batch is delivered once");
    // A relative path is made absolute against the working directory, as for a plain open.
    intake.sender().open_for(vec![PathBuf::from("rel.pdf")], "print");
    assert!(matches!(intake.take().as_slice(), [Intake::OpenFor { paths, .. }] if paths.len() == 1 && paths[0].is_absolute()));
}

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

/// Only an application counts, never a document, a folder or a missing path.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn an_application_is_what_the_platform_launches() {
    use crate::open::is_application;
    let temp = crate::temp::TempDir::new("craft-open-app-").unwrap();
    let dir = temp.path();
    let doc = dir.join("notes.txt");
    std::fs::write(&doc, "hi").unwrap();
    assert!(!is_application(&doc), "a document");
    assert!(!is_application(dir), "a folder");
    assert!(!is_application(&dir.join("Missing.app")), "a missing path");
    if cfg!(target_os = "macos") {
        let app = dir.join("Editor.app");
        std::fs::create_dir_all(&app).unwrap();
        assert!(is_application(&app), "an .app folder");
    }
    if cfg!(windows) {
        let exe = dir.join("editor.exe");
        std::fs::write(&exe, "").unwrap();
        assert!(is_application(&exe), "an .exe file");
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let tool = dir.join("editor");
        std::fs::write(&tool, "#!/bin/sh\n").unwrap();
        assert!(!is_application(&tool), "not executable yet");
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(is_application(&tool), "an executable file");
    }
}

/// A missing application is `NotFound` (the caller falls back to its default editor); anything else that is not an
/// application is refused as `InvalidInput` before anything is started.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn open_with_reports_a_missing_application_and_refuses_what_is_not_one() {
    use crate::open::open_with;
    use std::io::ErrorKind;
    let temp = crate::temp::TempDir::new("craft-open-with-").unwrap();
    let dir = temp.path();
    let doc = dir.join("notes.txt");
    std::fs::write(&doc, "not an application").unwrap();
    let kind = |app: &std::path::Path| open_with(app, std::path::Path::new("image with spaces.png")).unwrap_err().kind();
    assert_eq!(kind(&dir.join("Missing.exe")), ErrorKind::NotFound);
    assert_eq!(kind(&doc), ErrorKind::InvalidInput);
    assert_eq!(kind(dir), ErrorKind::InvalidInput);
}
